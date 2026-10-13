//! The HTTP server (layer 6): serves the web pages, handles sign-in with signed session cookies,
//! checks a CSRF token on every form, turns form posts into commands (`POST /do/{command}`) and
//! exposes the same commands as JSON (`POST /api/v1/commands/{id}`, bearer token) and as MCP
//! (`POST /mcp`).
#![forbid(unsafe_code)]

mod auth;
mod exif;
mod mail;
mod render;
mod zip;

/// The browser sheet viewer, when `cargo xtask viewer` built it before this crate was compiled.
mod viewer_assets {
    include!(concat!(env!("OUT_DIR"), "/viewer_assets.rs"));
}

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Multipart, Path, Query, State};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, LOCATION, SET_COOKIE};
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use buildkiosk_core::{Id, Timestamp, text};
use buildkiosk_engine::model::finance::{BudgetChange, ChangeEvent, CommitmentChange, PrimeChange, PrimeContract};
use buildkiosk_engine::model::finance::{CommitmentInvoice, OwnerInvoice};
use buildkiosk_engine::model::incident::Incident;
use buildkiosk_engine::model::observation::Observation;
use buildkiosk_engine::model::punch::PunchItem;
use buildkiosk_engine::model::task::Task;
use buildkiosk_engine::model::{CompanyState, FileRef, Level, Tool};
use buildkiosk_engine::workitem::{self, WorkItem};
use buildkiosk_engine::{Actor, EngineError, find_command, view};
use buildkiosk_store::creds::check_password_rules;
use buildkiosk_store::{Credentials, DataDir, Tenant, hex, random_bytes};
use buildkiosk_web::invoicing::{self, InvoicePage};
use buildkiosk_web::linedocs::{self, LineDoc};
use buildkiosk_web::pages::{self, RfiFilter};
use buildkiosk_web::workitems::{self, WorkPage};
use buildkiosk_web::{Flash, Req, icons, style};
use serde_json::{Map, Value, json};
use tower_http::catch_panic::CatchPanicLayer;

use auth::{CSRF_COOKIE, RateLimit, SESSION_COOKIE};

/// Largest form or API body accepted.
const MAX_BODY: usize = 1024 * 1024;
/// Largest total of a multi-file upload.
const MAX_BULK: usize = 100 * 1024 * 1024;
/// Most files in one multi-file upload, and the most bytes in one zip download.
const MAX_BULK_FILES: usize = 50;
const MAX_ZIP: u64 = 200 * 1024 * 1024;
/// Largest file upload.
const MAX_UPLOAD: usize = 25 * 1024 * 1024;

/// A form submission that was refused, kept for a few minutes so the form can be filled in again.
struct Draft {
    person: u64,
    /// The `action` of the form that was submitted.
    action: String,
    values: Vec<(String, String)>,
    at: std::time::Instant,
}

/// How long a refused form is remembered.
const DRAFT_TTL: Duration = Duration::from_secs(600);

struct Inner {
    drafts: HashMap<String, Draft>,
    pictures: render::Pictures,
    tenant: Option<Tenant>,
    creds: Credentials,
    limiter: RateLimit,
}

/// The running server's state: one company in one data directory.
pub struct App {
    data: DataDir,
    inner: Mutex<Inner>,
    /// Serving behind HTTPS: cookies get `Secure` and responses carry HSTS.
    secure: bool,
    /// Backups taken on a schedule while the server runs.
    backups: Option<BackupPlan>,
}

/// Where, how often and how many scheduled backups are kept.
struct BackupPlan {
    dir: PathBuf,
    every_secs: u64,
    keep: usize,
    /// A second place every finished backup is copied to (another disk or a mounted share).
    copy_to: Option<PathBuf>,
}

type Shared = Arc<App>;

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| i64::try_from(d.as_millis()).unwrap_or(0)).unwrap_or(0)
}

impl App {
    /// Opens `company` (or the only/first one in the directory). A directory with no company
    /// starts in first-run setup.
    pub fn open(data: DataDir, company: Option<&str>) -> Result<App, String> {
        let slugs = data.companies().map_err(|e| e.to_string())?;
        let slug = match company {
            Some(c) => Some(c.to_string()),
            None => slugs.first().cloned(),
        };
        let (tenant, creds) = match slug {
            Some(s) => {
                let t = data.load(&s).map_err(|e| e.to_string())?;
                let c = t.store.credentials().map_err(|e| e.to_string())?;
                (Some(t), c)
            }
            None => (None, Credentials::default()),
        };
        Ok(App {
            data,
            inner: Mutex::new(Inner { drafts: HashMap::new(), pictures: render::Pictures::default(), tenant, creds, limiter: RateLimit::default() }),
            secure: false,
            backups: None,
        })
    }

    /// Takes a backup into `dir` every `every_hours` hours while the server runs, keeping the newest `keep`.
    /// Each lands in `dir/backup-<unix seconds>` and was replayed to prove it loads. The folder can't be
    /// inside the data directory.
    pub fn with_backups(mut self, dir: PathBuf, every_hours: u64, keep: usize) -> Result<App, String> {
        if dir.starts_with(self.data.root()) {
            return Err("the backup folder can't be inside the data directory".into());
        }
        if every_hours == 0 || keep == 0 {
            return Err("backups need an interval of at least one hour and to keep at least one".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        self.backups = Some(BackupPlan { dir, every_secs: every_hours.saturating_mul(3600), keep, copy_to: None });
        Ok(self)
    }

    /// Also copies every finished backup to `dir` (another disk, or a share mounted from another machine), keeping
    /// as many there as are kept in the backup folder. Needs `with_backups` first.
    pub fn with_backup_copy(mut self, dir: PathBuf) -> Result<App, String> {
        let Some(plan) = self.backups.as_mut() else { return Err("a second copy needs --backup-dir".into()) };
        if dir.starts_with(self.data.root()) || dir.starts_with(&plan.dir) || plan.dir.starts_with(&dir) {
            return Err("the second copy's folder can't be inside the data directory or the backup folder, or hold them".into());
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        plan.copy_to = Some(dir);
        Ok(self)
    }

    /// Backs the data up into `dest` (a new or empty folder) without stopping the server. Requests are
    /// held up only while the small files are copied; the journal and the uploaded files are copied
    /// after, and the backup is as of the moment it began.
    pub fn backup_now(&self, dest: &FsPath) -> Result<Vec<buildkiosk_store::CompanyReport>, String> {
        let begun = {
            let _writes_paused = self.lock();
            self.data.backup_begin(dest)
        }
        .map_err(|e| e.to_string())?;
        begun.finish().map_err(|e| e.to_string())
    }

    /// Marks the app as served over HTTPS (behind a TLS-terminating proxy): session and CSRF
    /// cookies are sent with `Secure` and responses carry `Strict-Transport-Security`.
    pub fn with_secure_cookies(mut self, secure: bool) -> App {
        self.secure = secure;
        self
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while the lock was held leaves the data intact (commands run on a copy), so
        // carry on with it.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn secret(&self) -> &[u8] {
        self.data.secret()
    }
}

impl Inner {
    /// Remembers a refused submission and returns `back` with a `draft` marker so the page it
    /// leads to can put the typed values back. Passwords and form machinery are never kept.
    fn draft_path(&mut self, person: u64, action: &str, form: &[(String, String)], back: &str) -> String {
        let now = std::time::Instant::now();
        self.drafts.retain(|_, d| now.duration_since(d.at) < DRAFT_TTL);
        let mut size = 0usize;
        let values: Vec<(String, String)> = form
            .iter()
            .filter(|(k, _)| !k.starts_with('_') && !k.to_ascii_lowercase().contains("password") && k != "current" && k != "confirm")
            .take(2_000)
            .filter(|(k, v)| {
                size += k.len() + v.len();
                size <= 200_000
            })
            .cloned()
            .collect();
        if values.is_empty() || self.drafts.len() >= 200 {
            return back.to_string();
        }
        let Ok(raw) = random_bytes::<8>() else { return back.to_string() };
        let token = hex(&raw);
        self.drafts.insert(token.clone(), Draft { person, action: action.to_string(), values, at: now });
        let (path, query) = back.split_once('?').unwrap_or((back, ""));
        let query: Vec<&str> = query.split('&').filter(|p| !p.is_empty() && !p.starts_with("draft=")).collect();
        if query.is_empty() { format!("{path}?draft={token}") } else { format!("{path}?{}&draft={token}", query.join("&")) }
    }

    fn run(&mut self, actor: Actor, id: &str, params: &Value) -> Result<Value, EngineError> {
        let Some(t) = self.tenant.as_mut() else { return Err(EngineError::not_found("the company is not set up")) };
        let out = t.engine.execute(&mut t.store, actor, Timestamp::from_millis(now_ms()), id, params)?;
        if t.store.since_snapshot() >= 500
            && let Err(e) = t.store.snapshot(&t.engine)
        {
            log::warn!("could not write a snapshot: {e}");
        }
        Ok(out)
    }
}

// -- routing

pub fn router(app: Shared) -> Router {
    let router = Router::new()
        .route("/", get(home))
        .route("/login", get(login_page).post(login_post))
        .route("/logout", post(logout))
        .route("/setup", get(setup_page).post(setup_post))
        .route("/directory", get(directory))
        .route("/prequalification", get(prequalification_page))
        .route("/prequalification/forms", get(prequal_forms_page))
        .route("/prequalification/{vendor}", get(prequal_detail_page))
        .route("/prequalification/{vendor}/upload", post(prequal_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/prequalification/{vendor}/files/{sha}", get(prequal_download))
        .route("/my-tasks", get(my_tasks_page))
        .route("/reports", get(company_reports_page))
        .route("/injury-log", get(company_injury_log_page))
        .route("/injury-log.csv", get(company_injury_log_csv))
        .route("/reports.csv", get(company_reports_csv))
        .route(
            "/reports.pdf",
            get(|s: State<Shared>, h: HeaderMap, Query(q): Q| async move {
                let report = q.get("report").and_then(|r| r.parse::<u64>().ok()).map(Id::new);
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::company_report(r, report))
            }),
        )
        .route("/directory/people/{id}", get(person_edit))
        .route("/directory/templates/{id}", get(template_edit))
        .route("/directory/companies/{id}", get(vendor_edit))
        .route("/people/{id}/password", post(set_password))
        .route("/account", get(account_page))
        .route("/account/password", post(account_password))
        .route("/account/token", post(account_token))
        .route("/account/token/revoke", post(account_token_revoke))
        .route("/account/email", post(account_email))
        .route("/admin/backups", get(backups_page))
        .route("/admin/backups/run", post(backups_run))
        .route("/admin/email", get(mail_settings).post(mail_settings_save))
        .route("/admin/email/test", post(mail_test))
        .route("/admin/email/clear", post(mail_clear))
        .route("/projects/{id}", get(project_home))
        .route("/settings", get(company_settings))
        .route("/projects/{id}/settings", get(project_settings))
        .route("/projects/{id}/team", get(team))
        .route("/projects/{id}/team/{person}", get(member_access))
        .route("/projects/{id}/rfis", get(rfi_list))
        .route("/projects/{id}/rfis/new", get(rfi_new))
        .route("/projects/{id}/submittals", get(submittal_list))
        .route("/notifications", get(notifications))
        .route("/projects/{id}/budget", get(budget_page))
        .route("/projects/{id}/budget.csv", get(budget_csv))
        .route("/projects/{id}/direct-costs", get(direct_costs_page))
        .route("/projects/{id}/commitments", get(commitment_list))
        .route("/projects/{id}/commitments/new", get(commitment_new))
        .route("/projects/{id}/commitments/save", post(commitment_save_new))
        .route("/projects/{id}/commitments/{cid}", get(commitment_detail))
        .route("/projects/{id}/commitments/{cid}/edit", get(commitment_edit))
        .route("/projects/{id}/commitments/{cid}/upload", post(commitment_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/commitments/{cid}/files/{sha}", get(commitment_download))
        .route("/projects/{id}/commitments/{cid}/save", post(commitment_save))
        .route("/projects/{id}/inspections", get(inspection_list))
        .route("/projects/{id}/inspections.csv", get(inspection_csv))
        .route("/projects/{id}/inspections/new", get(inspection_new))
        .route("/projects/{id}/inspections/templates", get(inspection_templates))
        .route("/projects/{id}/inspections/{iid}", get(inspection_detail))
        .route("/projects/{id}/inspections/{iid}/edit", get(inspection_edit))
        .route(
            "/projects/{id}/inspections/{iid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::inspection(r, Id::new(id), Id::new(x)))
            }),
        )
        .route("/projects/{id}/inspections/{iid}/answers", post(inspection_answers))
        .route("/projects/{id}/inspections/{iid}/line-file", post(inspection_line_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/inspections/{iid}/files/{sha}", get(inspection_file))
        .route("/projects/{id}/inspections/{iid}/upload", post(inspection_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/daily-logs", get(daily_log_list))
        .route("/projects/{id}/daily-logs/new", get(daily_log_new))
        .route(
            "/projects/{id}/daily-logs/week",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                page(&s.0, &h, &q, |r, _| buildkiosk_web::daily_logs::week(r, Id::new(id), week_day(&q, r)))
            }),
        )
        .route(
            "/projects/{id}/daily-logs/week/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::daily_log_week(r, Id::new(id), week_day(&q, r)))
            }),
        )
        .route("/projects/{id}/daily-logs/save", post(daily_log_save_new))
        .route("/projects/{id}/daily-logs/{lid}", get(daily_log_detail))
        .route("/projects/{id}/daily-logs/{lid}/edit", get(daily_log_edit))
        .route(
            "/projects/{id}/daily-logs/{lid}/files/{sha}",
            get(|s: State<Shared>, h: HeaderMap, Path((id, lid, sha)): Path<(u64, u64, String)>| async move {
                file_download(s.0, h, Kind::DAILY_LOG, id, lid, sha).await
            }),
        )
        .route(
            "/projects/{id}/daily-logs/{lid}/upload",
            post(|s: State<Shared>, h: HeaderMap, Path((id, lid)): Path<(u64, u64)>, m: Multipart| async move {
                file_upload(s.0, h, Kind::DAILY_LOG, id, lid, m).await
            })
            .layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)),
        )
        .route("/projects/{id}/daily-logs/{lid}/save", post(daily_log_save))
        .route("/projects/{id}/specifications", get(specifications))
        .route("/projects/{id}/transmittals", get(transmittals_page))
        .route("/projects/{id}/transmittals/new", get(transmittal_new))
        .route("/projects/{id}/transmittals/{tid}", get(transmittal_page))
        .route("/projects/{id}/transmittals/{tid}/edit", get(transmittal_edit))
        .route("/projects/{id}/transmittals/{tid}/upload", post(transmittal_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/transmittals/{tid}/files/{sha}", get(transmittal_download))
        .route("/projects/{id}/meetings", get(meetings_page))
        .route("/projects/{id}/meetings/new", get(meeting_new))
        .route("/projects/{id}/meetings/{mid}", get(meeting_page))
        .route("/projects/{id}/meetings/{mid}/edit", get(meeting_edit))
        .route("/projects/{id}/meetings/{mid}/upload", post(meeting_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/meetings/{mid}/files/{sha}", get(meeting_download))
        .route("/projects/{id}/photos", get(photos_page))
        .route("/projects/{id}/photos/slideshow", get(photos_slideshow))
        .route("/projects/{id}/photos/album.zip", get(photos_zip))
        .route("/projects/{id}/photos/upload", post(photo_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/photos/{pid}", get(photo_page))
        .route("/projects/{id}/photos/{pid}/image", get(photo_image))
        .route("/projects/{id}/photos/{pid}/thumb", get(photo_thumb))
        .route("/projects/{id}/photos/{pid}/download", get(photo_download))
        .route("/projects/{id}/correspondence", get(correspondence_page))
        .route("/projects/{id}/correspondence/new", get(correspondence_new))
        .route("/projects/{id}/correspondence/types", get(correspondence_types))
        .route("/projects/{id}/correspondence/types/{tid}", get(correspondence_type_edit))
        .route("/projects/{id}/correspondence/incoming", get(correspondence_incoming))
        .route("/projects/{id}/correspondence/{cid}", get(correspondence_detail))
        .route("/projects/{id}/correspondence/{cid}/edit", get(correspondence_edit))
        .route("/projects/{id}/correspondence/{cid}/upload", post(correspondence_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/correspondence/{cid}/files/{sha}", get(correspondence_download))
        .route("/projects/{id}/forms", get(forms_page))
        .route("/projects/{id}/forms/templates", get(form_templates_page))
        .route("/projects/{id}/forms/report", get(forms_report_page))
        .route("/projects/{id}/forms/report.csv", get(forms_report_csv))
        .route("/projects/{id}/forms/templates/{tid}", get(form_template_edit))
        .route("/projects/{id}/forms/{rid}", get(form_page))
        .route("/projects/{id}/forms/{rid}/upload", post(form_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/forms/{rid}/files/{sha}", get(form_download))
        .route("/projects/{id}/timesheets", get(timesheets_page))
        .route("/projects/{id}/timesheets.csv", get(timesheets_csv))
        .route("/projects/{id}/timesheets/cost", get(timesheets_cost))
        .route("/projects/{id}/timesheets/cost.csv", get(timesheets_cost_csv))
        .route("/projects/{id}/reports", get(reports_page))
        .route("/projects/{id}/reports.csv", get(reports_csv))
        .route("/projects/{id}/action-plans", get(action_plans_page))
        .route("/projects/{id}/action-plans/new", get(action_plan_new))
        .route("/projects/{id}/action-plans/{aid}", get(action_plan_page))
        .route("/projects/{id}/action-plans/{aid}/edit", get(action_plan_edit))
        .route("/projects/{id}/bidding", get(bidding_page))
        .route("/projects/{id}/bidding/new", get(bid_package_new))
        .route("/projects/{id}/bidding/{bid}", get(bid_package_page))
        .route("/projects/{id}/bidding/{bid}/edit", get(bid_package_edit))
        .route("/projects/{id}/bidding/{bid}/upload", post(bid_package_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/bidding/{bid}/files/{sha}", get(bid_package_download))
        .route("/projects/{id}/estimating", get(estimating_page))
        .route("/projects/{id}/estimating/new", get(estimate_new))
        .route("/projects/{id}/estimating/compare", get(estimate_compare))
        .route("/catalog", get(catalog_page))
        .route("/projects/{id}/estimating/{eid}", get(estimate_page))
        .route("/projects/{id}/estimating/{eid}/edit", get(estimate_edit))
        .route("/projects/{id}/estimating/{eid}/csv", get(estimate_csv))
        .route("/projects/{id}/schedule", get(schedule_page))
        .route("/projects/{id}/schedule.csv", get(schedule_csv))
        .route("/projects/{id}/schedule.xml", get(schedule_xml))
        .route(
            "/projects/{id}/schedule/calendar",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                page(&s.0, &h, &q, |r, _| buildkiosk_web::schedule::calendar(r, Id::new(id)))
            }),
        )
        .route(
            "/projects/{id}/schedule/lookahead",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let weeks = q.get("weeks").and_then(|w| w.trim().parse::<i64>().ok()).unwrap_or(3);
                let from = q.get("from").and_then(|f| buildkiosk_core::Date::parse(f.trim()));
                page(&s.0, &h, &q, |r, _| buildkiosk_web::schedule::lookahead(r, Id::new(id), weeks, from))
            }),
        )
        .route(
            "/projects/{id}/schedule/resources",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let weeks = q.get("weeks").and_then(|w| w.trim().parse::<i64>().ok()).unwrap_or(8);
                let from = q.get("from").and_then(|f| buildkiosk_core::Date::parse(f.trim()));
                page(&s.0, &h, &q, |r, _| buildkiosk_web::schedule::resources(r, Id::new(id), weeks, from))
            }),
        )
        .route(
            "/projects/{id}/schedule/import-xml",
            post(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, m: Multipart| async move {
                import_file(s.0, h, id, "schedule", "schedule.import_xml", "task", ("8 MB", "Microsoft Project XML file"), &[], m).await
            })
            .layer(DefaultBodyLimit::max(8 * 1024 * 1024 + 64 * 1024)),
        )
        .route(
            "/projects/{id}/budget/import-file",
            post(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, m: Multipart| async move {
                import_file(s.0, h, id, "budget", "budget.import", "budget line", ("1 MB", "CSV file"), &["add_codes"], m).await
            })
            .layer(DefaultBodyLimit::max(1024 * 1024 + 64 * 1024)),
        )
        .route("/projects/{id}/schedule/{tid}", get(schedule_task_page))
        .route("/projects/{id}/drawings", get(drawings_page))
        .route("/projects/{id}/drawings.csv", get(drawings_csv))
        .route("/projects/{id}/drawings/upload", post(drawing_upload_new).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/drawings/upload-set", post(drawing_upload_set).layer(DefaultBodyLimit::max(MAX_BULK + 64 * 1024)))
        .route("/projects/{id}/drawings/{did}/view", get(drawing_view))
        .route("/projects/{id}/drawings/{did}/view.png", get(drawing_view_png))
        .route("/projects/{id}/drawings/{did}/compare.png", get(drawing_compare_png))
        .route("/projects/{id}/drawings/{did}", get(drawing_page))
        .route("/projects/{id}/drawings/{did}/upload", post(drawing_upload_revision).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/drawings/{did}/files/{sha}", get(drawing_download))
        .route("/projects/{id}/documents", get(documents_page))
        .route("/projects/{id}/documents/upload", post(document_upload_new).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/documents/upload-many", post(document_upload_many).layer(DefaultBodyLimit::max(MAX_BULK + 64 * 1024)))
        .route("/projects/{id}/documents/zip", get(documents_zip))
        .route("/projects/{id}/documents/trash", get(documents_trash))
        .route("/projects/{id}/documents/{did}", get(document_page))
        .route("/projects/{id}/documents/{did}/upload", post(document_upload_version).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/documents/{did}/files/{sha}", get(document_download))
        .route("/projects/{id}/documents/{did}/view/{sha}", get(document_view))
        .route("/projects/{id}/submittals.csv", get(submittal_csv))
        .route("/projects/{id}/submittals/import", post(submittal_import_file).layer(DefaultBodyLimit::max(1024 * 1024 + 64 * 1024)))
        .route("/projects/{id}/submittals/new", get(submittal_new))
        .route(
            "/projects/{id}/submittals/log",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                page(&s.0, &h, &q, |r, _| buildkiosk_web::submittals::log(r, Id::new(id)))
            }),
        )
        .route(
            "/projects/{id}/submittals/log/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::submittal_log(r, Id::new(id)))
            }),
        )
        .route("/projects/{id}/submittals/{sid}/review", post(submittal_review).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/submittals/{sid}", get(submittal_detail))
        .route("/projects/{id}/submittals/{sid}/edit", get(submittal_edit))
        .route("/projects/{id}/submittals/{sid}/files/{sha}", get(submittal_file))
        .route("/projects/{id}/submittals/{sid}/upload", post(submittal_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/rfis.csv", get(rfi_csv))
        .route(
            "/projects/{id}/rfis/{rid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, rid)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::rfi(r, Id::new(id), Id::new(rid)))
            }),
        )
        .route(
            "/projects/{id}/submittals/{sid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::submittal(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/daily-logs/{lid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::daily_log(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/meetings/{mid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::meeting(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/meetings/{mid}/signin.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::meeting_signin(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/transmittals/{tid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::transmittal(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/correspondence/{cid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::correspondence(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/correspondence/{cid}/letter.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::correspondence_letter(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/estimating/{eid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::estimate(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/bidding/{bid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::bid_package(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/action-plans/{aid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::action_plan(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/forms/{rid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::form_record(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/schedule/lookahead.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let weeks = q.get("weeks").and_then(|w| w.trim().parse::<i64>().ok()).unwrap_or(3);
                let from = q.get("from").and_then(|f| buildkiosk_core::Date::parse(f.trim()));
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::lookahead(r, Id::new(id), weeks, from))
            }),
        )
        .route(
            "/projects/{id}/incidents/{iid}/pdf",
            get(|s: State<Shared>, h: HeaderMap, Path((id, x)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::incident(r, Id::new(id), Id::new(x)))
            }),
        )
        .route(
            "/projects/{id}/incidents/log.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let year = q.get("year").and_then(|y| y.trim().parse::<i32>().ok());
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::injury_log(r, Id::new(id), year))
            }),
        )
        .route(
            "/projects/{id}/incidents/log",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let year = q.get("year").and_then(|y| y.trim().parse::<i32>().ok());
                page(&s.0, &h, &q, |r, _| workitems::injury_log(r, Id::new(id), year))
            }),
        )
        .route("/projects/{id}/incidents/log.csv", get(injury_log_csv))
        .route(
            "/projects/{id}/budget.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::budget(r, Id::new(id), budget_view_choice(r, Id::new(id), &q)))
            }),
        )
        .route(
            "/projects/{id}/drawings.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::drawings(r, Id::new(id)))
            }),
        )
        .route(
            "/projects/{id}/schedule.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::schedule(r, Id::new(id)))
            }),
        )
        .route(
            "/projects/{id}/reports.pdf",
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>| async move {
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::project_report(r, Id::new(id)))
            }),
        )
        .route("/projects/{id}/rfis/{rid}", get(rfi_detail))
        .route("/projects/{id}/rfis/{rid}/files/{sha}", get(rfi_file))
        .route("/projects/{id}/rfis/{rid}/upload", post(rfi_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/rfis/{rid}/respond", post(rfi_respond).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/rfis/{rid}/edit", get(rfi_edit))
        .route("/do/{command}", post(do_command))
        .route("/mcp", post(mcp_http))
        .route("/api/v1/commands/{id}", post(api_command))
        .route("/static/app.css", get(css))
        .route("/static/favicon.svg", get(favicon))
        .route("/static/viewer/start.js", get(|h: HeaderMap| async move { embedded(&h, viewer_assets::START_JS, "text/javascript; charset=utf-8") }))
        .route(
            "/static/viewer/viewer.js",
            get(|h: HeaderMap| async move { embedded(&h, viewer_assets::VIEWER_JS, "text/javascript; charset=utf-8") }),
        )
        .route("/static/viewer/viewer_bg.wasm", get(|h: HeaderMap| async move { embedded(&h, viewer_assets::VIEWER_WASM, "application/wasm") }));
    let router = work_routes::<PunchItem>(router);
    let router = work_routes::<Observation>(router).route(
        "/projects/{id}/observations/trends",
        get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
            page(&s.0, &h, &q, |r, _| workitems::observation_trends(r, Id::new(id)))
        }),
    );
    let router = work_routes::<Task>(router).route(
        "/projects/{id}/tasks/calendar",
        get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
            let month = q.get("month").and_then(|m| buildkiosk_core::Date::parse(&format!("{}-01", m.trim())));
            page(&s.0, &h, &q, |r, _| workitems::task_calendar(r, Id::new(id), month))
        }),
    );
    let router = work_routes::<Incident>(router);
    let router = doc_routes::<ChangeEvent>(router);
    let router = doc_routes::<CommitmentChange>(router);
    let router = doc_routes::<BudgetChange>(router);
    let router = doc_routes::<PrimeContract>(router);
    let router = doc_routes::<PrimeChange>(router);
    let router = invoice_routes::<OwnerInvoice>(router);
    let router = invoice_routes::<CommitmentInvoice>(router);
    router
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(MAX_BODY))
        .layer(axum::middleware::map_response_with_state(app.clone(), security_headers))
        .layer(CatchPanicLayer::new())
        .with_state(app)
}

/// Serves until the process is stopped.
pub async fn serve(app: Shared, bind: SocketAddr) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(bind).await.map_err(|e| format!("cannot listen on {bind}: {e}"))?;
    log::info!("listening on http://{bind}");
    tokio::spawn(mail_loop(app.clone()));
    tokio::spawn(prequal_loop(app.clone()));
    tokio::spawn(form_loop(app.clone()));
    if app.backups.is_some() {
        tokio::spawn(backup_loop(app.clone()));
    }
    axum::serve(listener, router(app)).await.map_err(|e| e.to_string())
}

async fn security_headers(State(app): State<Shared>, mut res: Response) -> Response {
    let h = res.headers_mut();
    // A page that needs more (the sheet viewer) sets its own policy first.
    if !h.contains_key("content-security-policy") {
        h.insert(
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'none'; script-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
            ),
        );
    }
    let mut put = |k: &'static str, v: &'static str| {
        h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
    };
    put("x-content-type-options", "nosniff");
    put("referrer-policy", "same-origin");
    put("x-frame-options", "DENY");
    if app.secure {
        put("strict-transport-security", "max-age=31536000");
    }
    res
}

// -- helpers

fn redirect(to: &str) -> Response {
    let mut res = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(to) {
        res.headers_mut().insert(LOCATION, v);
    }
    res
}

fn with_cookie(mut res: Response, cookie: &str) -> Response {
    if let Ok(v) = HeaderValue::from_str(cookie) {
        res.headers_mut().append(SET_COOKIE, v);
    }
    res
}

fn html(status: StatusCode, markup: maud::Markup) -> Response {
    let mut res = (status, markup.into_string()).into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    res.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
    res
}

fn plain(status: StatusCode, msg: &str) -> Response {
    (status, msg.to_string()).into_response()
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let h = headers.get(COOKIE)?.to_str().ok()?;
    auth::cookie_value(h, name).map(str::to_string)
}

/// Percent-encodes a query value.
fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A redirect target that stays on this site.
fn local_path(s: &str) -> &str {
    if s.starts_with('/') && !s.starts_with("//") && !s.contains('\\') && !s.contains('\n') && !s.contains('\r') { s } else { "/" }
}

fn with_flash(path: &str, ok: bool, msg: &str) -> String {
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let msg: String = msg.chars().take(300).collect();
    let flash = format!("{}={}", if ok { "ok" } else { "error" }, encode(&msg));
    if query.is_empty() { format!("{path}?{flash}") } else { format!("{path}?{query}&{flash}") }
}

struct Session {
    person: Id<buildkiosk_engine::model::directory::Person>,
    cookie: String,
}

/// The signed-in person, if the cookie is valid and the person may still sign in.
fn session(app: &App, inner: &Inner, headers: &HeaderMap) -> Option<Session> {
    let raw = cookie(headers, SESSION_COOKIE)?;
    let data = auth::read_session(app.secret(), &raw, now_ms())?;
    let t = inner.tenant.as_ref()?;
    let id = Id::new(data.person);
    let p = t.engine.state().people.get(id)?;
    (p.is_enabled() && p.is_user && inner.creds.epoch(data.person) == data.epoch && inner.creds.has_password(data.person))
        .then_some(Session { person: id, cookie: raw })
}

/// The anonymous CSRF cookie (made if absent) and the token for it.
fn anon_csrf(app: &App, headers: &HeaderMap) -> (String, Option<String>) {
    match cookie(headers, CSRF_COOKIE).filter(|c| c.len() == 32 && c.bytes().all(|b| b.is_ascii_hexdigit())) {
        Some(c) => (auth::csrf_token(app.secret(), &c), None),
        None => {
            let c = random_bytes::<16>().map(|b| hex(&b)).unwrap_or_else(|_| "0".repeat(32));
            (auth::csrf_token(app.secret(), &c), Some(auth::set_cookie(CSRF_COOKIE, &c, 86_400, app.secure)))
        }
    }
}

fn flash_from(q: &HashMap<String, String>) -> Option<Flash> {
    if let Some(m) = q.get("error") {
        return Some(Flash { ok: false, msg: m.chars().take(300).collect() });
    }
    q.get("ok").map(|m| Flash { ok: true, msg: m.chars().take(300).collect() })
}

fn get_form<'a>(form: &'a [(String, String)], key: &str) -> &'a str {
    form.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
}

fn parse_form(body: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter(|p| !p.is_empty())
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
            (url_decode(k), url_decode(v))
        })
        .collect()
}

fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let hi = bytes.get(i + 1).and_then(|c| char::from(*c).to_digit(16));
                let lo = bytes.get(i + 2).and_then(|c| char::from(*c).to_digit(16));
                if let (Some(h), Some(l)) = (hi, lo)
                    && let Ok(v) = u8::try_from(h * 16 + l)
                {
                    out.push(v);
                    i += 2;
                } else {
                    out.push(b);
                }
            }
            _ => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// -- pages

/// Runs `render` for a signed-in person; redirects to sign-in or setup otherwise.
fn page(app: &App, headers: &HeaderMap, q: &HashMap<String, String>, render: impl FnOnce(&Req<'_>, &Tenant) -> maud::Markup) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(app, &inner, headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: flash_from(q) };
    let markup = render(&req, t);
    // A refused save sends the person back here with what they typed.
    let draft = q.get("draft").and_then(|tok| inner.drafts.get(tok)).filter(|d| d.person == s.person.raw() && d.at.elapsed() < DRAFT_TTL);
    match draft {
        Some(d) => {
            let mut res = html(StatusCode::OK, maud::PreEscaped(buildkiosk_web::restore::restore_form(&markup.into_string(), &d.action, &d.values)));
            res.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
            res
        }
        None => html(StatusCode::OK, markup),
    }
}

type Q = Query<HashMap<String, String>>;

async fn home(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::home(r))
}

async fn company_reports_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let report = q.get("report").and_then(|r| r.parse::<u64>().ok()).map(Id::new);
    use buildkiosk_engine::model::Choice as _;
    let chart = q.get("chart").and_then(|c| buildkiosk_engine::model::saved_report::ReportColumn::parse(c));
    page(&app, &headers, &q, |r, _| buildkiosk_web::company_reports::page(r, report, chart))
}

async fn company_injury_log_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let year = q.get("year").and_then(|y| y.trim().parse::<i32>().ok());
    page(&app, &headers, &q, |r, _| workitems::company_injury_log(r, year))
}

async fn company_injury_log_csv(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let year = q.get("year").and_then(|y| y.trim().parse::<i32>().ok());
    let mut res = workitems::company_injury_log_csv(&req, year).into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"injury-log-all-projects.csv\""));
    res
}

async fn company_reports_csv(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let report = q.get("report").and_then(|r| r.parse::<u64>().ok()).map(Id::new);
    let Some(csv) = buildkiosk_web::company_reports::csv(&req, report) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"report.csv\""));
    res
}

async fn my_tasks_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let (closed, all) = (q.contains_key("closed"), q.contains_key("all"));
    page(&app, &headers, &q, |r, _| workitems::my_tasks(r, closed, all))
}

async fn prequal_forms_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let edit = q.get("edit").and_then(|e| e.parse::<u64>().ok()).map(Id::new);
    page(&app, &headers, &q, |r, _| buildkiosk_web::prequal::forms_page(r, edit))
}

async fn prequal_detail_page(State(app): State<Shared>, headers: HeaderMap, Path(vendor): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::prequal::detail(r, Id::new(vendor)))
}

async fn prequal_download(State(app): State<Shared>, headers: HeaderMap, Path((vendor, sha)): Path<(u64, String)>) -> Response {
    let found = {
        let inner = app.lock();
        let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
        let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
        let state = t.engine.state();
        let may = buildkiosk_engine::model::perm::company_level(state, s.person, Tool::Prequalification) >= buildkiosk_engine::model::Level::Read;
        let file = state
            .prequalifications
            .iter()
            .find(|r| r.vendor.raw() == vendor && r.deleted.is_none())
            .filter(|_| may)
            .and_then(|r| r.attachments.iter().find(|a| a.sha256 == sha).cloned());
        file.and_then(|a| t.store.blobs.path(&a.sha256).map(|p| (p, a.name.clone())))
    };
    let Some((path, name)) = found else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        _ => return plain(StatusCode::NOT_FOUND, "That file is missing from storage."),
    };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn prequal_upload(State(app): State<Shared>, headers: HeaderMap, Path(vendor): Path<u64>, mut multipart: Multipart) -> Response {
    let back = format!("/prequalification/{vendor}");
    let (mut token, mut file) = (String::new(), None);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else {
            return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB."));
        };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if file.is_none() => file = Some((filename, mime, bytes)),
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return redirect(&with_flash(&back, false, "Choose a file to attach."));
    };
    if bytes.len() > MAX_UPLOAD {
        return redirect(&with_flash(&back, false, "Files can be at most 25 MB."));
    }
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let (sha256, size) = match t.store.blobs.put(&bytes) {
        Ok(x) => x,
        Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
    };
    match inner.run(Actor::Person(s.person), "prequal.attach", &json!({"vendor": vendor, "sha256": sha256, "name": name, "size": size, "mime": mime}))
    {
        Ok(_) => redirect(&with_flash(&back, true, "File attached.")),
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

/// Warns the Prequalification admins before approvals lapse: a pass every six hours (the engine sends each
/// warning once, so more passes cost nothing).
async fn prequal_loop(app: Shared) {
    tokio::time::sleep(Duration::from_secs(90)).await;
    loop {
        let a = app.clone();
        let pass = tokio::task::spawn_blocking(move || a.lock().run(Actor::System, "prequal.remind", &json!({}))).await;
        match pass {
            Ok(Ok(v)) => {
                if v.get("reminded").and_then(Value::as_u64).unwrap_or(0) > 0 {
                    log::info!("prequalification warnings sent: {v}");
                }
            }
            Ok(Err(e)) => log::warn!("the prequalification warning pass failed: {e}"),
            Err(e) => log::warn!("the prequalification warning pass stopped: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(6 * 3600)).await;
    }
}

/// Starts the forms that recurring schedules say are due: a pass every hour (a schedule makes one form per
/// day it comes due, so more passes cost nothing).
async fn form_loop(app: Shared) {
    tokio::time::sleep(Duration::from_secs(120)).await;
    loop {
        let a = app.clone();
        let pass = tokio::task::spawn_blocking(move || a.lock().run(Actor::System, "form.generate_due", &json!({}))).await;
        match pass {
            Ok(Ok(v)) => {
                if v.get("made").and_then(Value::as_u64).unwrap_or(0) > 0 {
                    log::info!("recurring forms started: {v}");
                }
            }
            Ok(Err(e)) => log::warn!("the recurring forms pass failed: {e}"),
            Err(e) => log::warn!("the recurring forms pass stopped: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(3600)).await;
    }
}

async fn forms_report_params(
    q: &std::collections::HashMap<String, String>,
) -> (
    Option<Id<buildkiosk_engine::model::form::FormTemplate>>,
    Option<buildkiosk_engine::model::form::FormStatus>,
    Option<buildkiosk_core::Date>,
    Option<buildkiosk_core::Date>,
) {
    use buildkiosk_engine::model::Choice;
    (
        q.get("template").and_then(|t| t.parse::<u64>().ok()).map(Id::new),
        q.get("status").and_then(|s| buildkiosk_engine::model::form::FormStatus::parse(s)),
        q.get("from").and_then(|d| buildkiosk_core::Date::parse(d)),
        q.get("to").and_then(|d| buildkiosk_core::Date::parse(d)),
    )
}

async fn forms_report_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let (t, st, f, to) = forms_report_params(&q).await;
    page(&app, &headers, &q, |r, _| buildkiosk_web::forms::report(r, Id::new(id), t, st, f, to))
}

async fn forms_report_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let (t, st, f, to) = forms_report_params(&q).await;
    let Some(t) = t else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let inner = app.lock();
    let Some(tenant) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &tenant.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::forms::report_csv(&req, Id::new(id), t, st, f, to) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"forms-report.csv\""));
    res
}

async fn prequalification_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::prequal::PrequalStatus::parse(s));
    let attention = q.get("attention").is_some_and(|a| a == "1");
    let edit = q.get("edit").and_then(|e| e.parse::<u64>().ok()).map(Id::new);
    page(&app, &headers, &q, |r, _| buildkiosk_web::prequal::page(r, status, attention, edit))
}

async fn directory(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::directory(r))
}

async fn template_edit(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::template_edit(r, Id::new(id)))
}

async fn vendor_edit(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::vendor_edit(r, Id::new(id)))
}

async fn person_edit(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::person_edit(r, Id::new(id)))
}

async fn project_home(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::project_home(r, Id::new(id)))
}

async fn team(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::team(r, Id::new(id)))
}

async fn company_settings(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::settings::company(r))
}

async fn project_settings(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::settings::project(r, Id::new(id)))
}

async fn member_access(State(app): State<Shared>, headers: HeaderMap, Path((id, person)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::member_access(r, Id::new(id), Id::new(person)))
}

async fn rfi_list(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let filter = RfiFilter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::rfi::RfiStatus::parse(s)),
        q: q.get("q").map(|s| s.chars().take(100).collect()).unwrap_or_default(),
        mine: q.get("mine").is_some_and(|m| m == "1"),
    };
    page(&app, &headers, &q, |r, _| pages::rfi_list(r, Id::new(id), &filter))
}

async fn rfi_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::rfi_form(r, Id::new(id), None))
}

async fn rfi_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| pages::rfi_detail(r, Id::new(id), Id::new(rid), &t.store.history("rfis", rid).unwrap_or_default()))
}

async fn rfi_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::rfi_form(r, Id::new(id), Some(Id::new(rid))))
}

async fn submittal_list(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let filter = buildkiosk_web::submittals::Filter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::submittal::SubmittalStatus::parse(s)),
        q: q.get("q").map(|s| s.chars().take(100).collect()).unwrap_or_default(),
        mine: q.get("mine").is_some_and(|m| m == "1"),
    };
    page(&app, &headers, &q, |r, _| buildkiosk_web::submittals::list(r, Id::new(id), &filter))
}

async fn daily_log_list(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let month: String = q.get("month").filter(|m| m.len() == 7 && m.bytes().all(|b| b.is_ascii_digit() || b == b'-')).cloned().unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::list(r, Id::new(id), &month))
}

async fn daily_log_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::form(r, Id::new(id), None, q.get("copy").is_some_and(|c| c == "1")))
}

async fn daily_log_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, lid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::detail(r, Id::new(id), Id::new(lid)))
}

async fn daily_log_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, lid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::form(r, Id::new(id), Some(Id::new(lid)), false))
}

async fn daily_log_save_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, body: Bytes) -> Response {
    daily_log_save_impl(app, headers, id, None, body)
}

async fn daily_log_save(State(app): State<Shared>, headers: HeaderMap, Path((id, lid)): Path<(u64, u64)>, body: Bytes) -> Response {
    daily_log_save_impl(app, headers, id, Some(lid), body)
}

/// The list-valued fields of the daily log form: `manpower.0.company`, `manpower.0.workers`…
/// become `manpower: [{company, workers}]`, in row order.
fn indexed_rows(form: &[(String, String)], list: &str) -> Value {
    let mut rows: std::collections::BTreeMap<usize, Map<String, Value>> = std::collections::BTreeMap::new();
    for (k, v) in form {
        let Some(rest) = k.strip_prefix(list).and_then(|r| r.strip_prefix('.')) else { continue };
        let Some((index, field)) = rest.split_once('.') else { continue };
        let Ok(index) = index.parse::<usize>() else { continue };
        if index < 1000 && rows.len() < 1000 {
            rows.entry(index).or_default().insert(field.to_string(), Value::String(v.clone()));
        }
    }
    Value::Array(rows.into_values().map(Value::Object).collect())
}

fn daily_log_save_impl(app: Shared, headers: HeaderMap, project: u64, log: Option<u64>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let base = format!("/projects/{project}/daily-logs");
    let back = match log {
        Some(l) => format!("{base}/{l}/edit"),
        None => format!("{base}/new"),
    };
    let mut params = Map::new();
    params.insert("project".into(), json!(project));
    if let Some(l) = log {
        params.insert("id".into(), json!(l));
    }
    for key in ["date", "sky", "high", "low", "weather_notes", "work_performed", "delays", "safety", "notes"] {
        params.insert(key.into(), Value::String(get_form(&form, key).to_string()));
    }
    for list in ["manpower", "equipment", "deliveries", "visitors"] {
        params.insert(list.into(), indexed_rows(&form, list));
    }
    let actor = Actor::Person(s.person);
    let command = if log.is_some() { "daily_log.update" } else { "daily_log.create" };
    let id = match inner.run(actor, command, &Value::Object(params)) {
        Ok(out) => log.or_else(|| out.get("id").and_then(Value::as_u64)).unwrap_or(0),
        Err(e) => {
            let back = inner.draft_path(s.person.raw(), &save_action(&back), &form, &back);
            return redirect(&with_flash(&back, false, &e.to_string()));
        }
    };
    let detail = format!("{base}/{id}");
    if !get_form(&form, "_submit").is_empty() {
        let params = json!({"project": project, "id": id});
        return match inner.run(actor, "daily_log.submit", &params) {
            Ok(_) => redirect(&with_flash(&detail, true, "Daily log submitted.")),
            Err(e) => redirect(&with_flash(&detail, false, &format!("Saved, but not submitted: {e}"))),
        };
    }
    redirect(&with_flash(&detail, true, "Daily log saved."))
}

/// The day a weekly summary was asked for (`?date=`), else today.
fn week_day(q: &HashMap<String, String>, r: &Req<'_>) -> buildkiosk_core::Date {
    q.get("date").and_then(|d| buildkiosk_core::Date::parse(d)).unwrap_or_else(|| r.today())
}

/// The list filter a work item page or PDF was asked for.
fn work_filter(q: &HashMap<String, String>) -> workitems::Filter {
    use buildkiosk_engine::model::Choice;
    let text = |k: &str| q.get(k).map(|s| s.chars().take(100).collect()).unwrap_or_default();
    workitems::Filter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::work::WorkStatus::parse(s)),
        q: text("q"),
        mine: q.get("mine").is_some_and(|m| m == "1"),
        assignee: q.get("assignee").and_then(|a| a.parse().ok()),
        location: text("location"),
        kind: text("kind"),
        group: workitems::Group::parse(q.get("group").map(String::as_str).unwrap_or("")),
    }
}

async fn work_list<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let filter = work_filter(&q);
    page(&app, &headers, &q, |r, _| workitems::list::<T>(r, Id::new(id), &filter))
}

async fn work_new<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| workitems::form::<T>(r, Id::new(id), None))
}

async fn work_detail<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| workitems::detail::<T>(r, Id::new(id), Id::new(iid), &t.store.history(T::TABLE, iid).unwrap_or_default()))
}

async fn work_edit<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| workitems::form::<T>(r, Id::new(id), Some(Id::new(iid))))
}

async fn work_csv<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = workitems::csv::<T>(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}.csv\"", T::PATH)) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn work_file<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::work::<T>(), id, iid, sha).await
}

async fn work_upload<T: WorkPage>(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, iid)): Path<(u64, u64)>,
    multipart: Multipart,
) -> Response {
    file_upload(app, headers, Kind::work::<T>(), id, iid, multipart).await
}

async fn doc_list<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| linedocs::list::<T>(r, Id::new(id)))
}

async fn doc_new<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| linedocs::form::<T>(r, Id::new(id), None))
}

async fn doc_detail<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| linedocs::detail::<T>(r, Id::new(id), Id::new(did), &t.store.history(T::TABLE, did).unwrap_or_default()))
}

async fn doc_edit<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| linedocs::form::<T>(r, Id::new(id), Some(Id::new(did))))
}

async fn doc_save_new<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, body: Bytes) -> Response {
    doc_save_impl::<T>(app, headers, id, None, body)
}

async fn doc_save<T: LineDoc>(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, body: Bytes) -> Response {
    doc_save_impl::<T>(app, headers, id, Some(did), body)
}

/// Saves a cost document form: its plain fields and the `lines.N.*` rows.
fn doc_save_impl<T: LineDoc>(app: Shared, headers: HeaderMap, project: u64, doc: Option<u64>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let base = format!("/projects/{project}/{}", T::PATH);
    let back = match doc {
        Some(d) => format!("{base}/{d}/edit"),
        None => format!("{base}/new"),
    };
    let mut params = Map::new();
    params.insert("project".into(), json!(project));
    if let Some(d) = doc {
        params.insert("id".into(), json!(d));
    }
    for key in T::FIELDS {
        params.insert((*key).into(), Value::String(get_form(&form, key).to_string()));
    }
    params.insert("lines".into(), indexed_rows(&form, "lines"));
    let command = format!("{}.{}", T::PREFIX, if doc.is_some() { "update" } else { "create" });
    match inner.run(Actor::Person(s.person), &command, &Value::Object(params)) {
        Ok(out) => {
            let id = doc.or_else(|| out.get("id").and_then(Value::as_u64)).unwrap_or(0);
            redirect(&with_flash(&format!("{base}/{id}"), true, "Saved."))
        }
        Err(e) => {
            let back = inner.draft_path(s.person.raw(), &save_action(&back), &form, &back);
            redirect(&with_flash(&back, false, &e.to_string()))
        }
    }
}

/// Registers the pages of a cost document under `/projects/{id}/<path>`.
fn doc_routes<T: LineDoc>(router: Router<Shared>) -> Router<Shared> {
    let base = format!("/projects/{{id}}/{}", T::PATH);
    router
        .route(&base, get(doc_list::<T>))
        .route(&format!("{base}/new"), get(doc_new::<T>))
        .route(&format!("{base}/save"), post(doc_save_new::<T>))
        .route(&format!("{base}/{{did}}"), get(doc_detail::<T>))
        .route(&format!("{base}/{{did}}/edit"), get(doc_edit::<T>))
        .route(&format!("{base}/{{did}}/save"), post(doc_save::<T>))
}

/// Registers the pages of a work item tool under `/projects/{id}/<path>`.
fn work_routes<T: WorkPage>(router: Router<Shared>) -> Router<Shared> {
    let base = format!("/projects/{{id}}/{}", T::PATH);
    router
        .route(&base, get(work_list::<T>))
        .route(&format!("{base}.csv"), get(work_csv::<T>))
        .route(
            &format!("{base}.pdf"),
            get(|s: State<Shared>, h: HeaderMap, Path(id): Path<u64>, Query(q): Q| async move {
                let filter = work_filter(&q);
                pdf_for(&s.0, &h, |r| buildkiosk_web::pdfs::work_list::<T>(r, Id::new(id), T::TITLE, &filter))
            }),
        )
        .route(&format!("{base}/import"), post(work_import_file::<T>).layer(DefaultBodyLimit::max(1024 * 1024 + 64 * 1024)))
        .route(&format!("{base}/new"), get(work_new::<T>))
        .route(&format!("{base}/{{iid}}"), get(work_detail::<T>))
        .route(&format!("{base}/{{iid}}/edit"), get(work_edit::<T>))
        .route(&format!("{base}/{{iid}}/files/{{sha}}"), get(work_file::<T>))
        .route(&format!("{base}/{{iid}}/upload"), post(work_upload::<T>).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
}

async fn inspection_list(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let filter = buildkiosk_web::inspections::Filter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::inspection::InspectionStatus::parse(s)),
        q: q.get("q").map(|s| s.chars().take(100).collect()).unwrap_or_default(),
        mine: q.get("mine").is_some_and(|m| m == "1"),
    };
    page(&app, &headers, &q, |r, _| buildkiosk_web::inspections::list(r, Id::new(id), &filter))
}

async fn inspection_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::inspections::form(r, Id::new(id), None))
}

async fn inspection_templates(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::inspections::templates(r, Id::new(id)))
}

async fn inspection_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| {
        buildkiosk_web::inspections::detail(r, Id::new(id), Id::new(iid), &t.store.history("inspections", iid).unwrap_or_default())
    })
}

async fn inspection_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::inspections::form(r, Id::new(id), Some(Id::new(iid))))
}

async fn inspection_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::inspections::csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"inspections.csv\""));
    res
}

async fn inspection_file(State(app): State<Shared>, headers: HeaderMap, Path((id, iid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::INSPECTION, id, iid, sha).await
}

async fn inspection_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::INSPECTION, id, iid, multipart).await
}

/// Attaches a file (a photo) to one checklist line: a multipart form with `_csrf`, `line` and `file`.
async fn inspection_line_upload(
    State(app): State<Shared>,
    headers: HeaderMap,
    Path((id, iid)): Path<(u64, u64)>,
    mut multipart: Multipart,
) -> Response {
    let back = format!("/projects/{id}/inspections/{iid}");
    let too_big = "The upload failed or the file is larger than 25 MB.";
    let (mut token, mut line, mut file) = (String::new(), String::new(), None);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, too_big)),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&back, false, too_big)) };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "line" => line = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if file.is_none() => file = Some((filename, mime, bytes)),
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return redirect(&with_flash(&back, false, "Choose a file to attach."));
    };
    let Ok(line) = line.trim().parse::<u32>() else { return redirect(&with_flash(&back, false, "Choose the line the file belongs to.")) };
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let (sha256, size) = match t.store.blobs.put(&bytes) {
        Ok(x) => x,
        Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
    };
    let params = json!({"project": id, "id": iid, "line": line, "sha256": sha256, "name": name, "size": size, "mime": mime});
    match inner.run(Actor::Person(s.person), "inspection.attach_line", &params) {
        Ok(_) => redirect(&with_flash(&back, true, "File attached to the line.")),
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

/// Saves the checklist form (`line.N.answer`, `line.N.note`), and completes the inspection when
/// asked to.
async fn inspection_answers(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let back = format!("/projects/{id}/inspections/{iid}");
    let mut lines: std::collections::BTreeMap<usize, Map<String, Value>> = std::collections::BTreeMap::new();
    for (k, v) in &form {
        let Some(rest) = k.strip_prefix("line.") else { continue };
        let Some((n, field)) = rest.split_once('.') else { continue };
        let Ok(n) = n.parse::<usize>() else { continue };
        if n < 300 {
            let row = lines.entry(n).or_default();
            row.insert("line".into(), json!(n + 1));
            row.insert(field.to_string(), Value::String(v.clone()));
        }
    }
    let answers: Vec<Value> = lines.into_values().map(Value::Object).collect();
    let actor = Actor::Person(s.person);
    if !answers.is_empty()
        && let Err(e) = inner.run(actor, "inspection.answer", &json!({"project": id, "id": iid, "answers": answers}))
    {
        return redirect(&with_flash(&back, false, &e.to_string()));
    }
    if !get_form(&form, "_complete").is_empty() {
        let params = json!({"project": id, "id": iid, "signature": get_form(&form, "signature")});
        return match inner.run(actor, "inspection.complete", &params) {
            Ok(_) => redirect(&with_flash(&back, true, "Inspection completed.")),
            Err(e) => redirect(&with_flash(&back, false, &format!("Answers saved, but not completed: {e}"))),
        };
    }
    redirect(&with_flash(&back, true, "Answers saved."))
}

/// The saved budget view to show: the one named in the link (0 is the standard layout), else the person's default.
fn budget_view_choice(
    req: &Req<'_>,
    project: Id<buildkiosk_engine::model::project::Project>,
    q: &HashMap<String, String>,
) -> Option<Id<buildkiosk_engine::model::finance::BudgetView>> {
    match q.get("view") {
        Some(v) => v.parse::<u64>().ok().filter(|n| *n > 0).map(Id::new),
        None => view::default_budget_view(req.engine.state(), req.person, project),
    }
}

async fn budget_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| {
        buildkiosk_web::finance::budget(
            r,
            Id::new(id),
            q.get("compare").and_then(|c| c.parse::<u64>().ok()).map(Id::new),
            budget_view_choice(r, Id::new(id), &q),
        )
    })
}

async fn injury_log_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let year = q.get("year").and_then(|y| y.trim().parse::<i32>().ok());
    let Some(csv) = workitems::injury_log_csv(&req, Id::new(id), year) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"injury-log.csv\""));
    res
}

async fn budget_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::finance::budget_csv(&req, Id::new(id), budget_view_choice(&req, Id::new(id), &q)) else {
        return plain(StatusCode::NOT_FOUND, "Not found");
    };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"budget.csv\""));
    res
}

async fn direct_costs_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::finance::direct_costs(r, Id::new(id)))
}

async fn commitment_list(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let filter = buildkiosk_web::finance::CommitmentFilter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::finance::CommitmentStatus::parse(s)),
        q: q.get("q").map(|s| s.chars().take(100).collect()).unwrap_or_default(),
    };
    page(&app, &headers, &q, |r, _| buildkiosk_web::finance::commitments(r, Id::new(id), &filter))
}

async fn commitment_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::finance::commitment_form(r, Id::new(id), None))
}

async fn commitment_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| {
        buildkiosk_web::finance::commitment(r, Id::new(id), Id::new(cid), &t.store.history("commitments", cid).unwrap_or_default())
    })
}

async fn commitment_download(State(app): State<Shared>, headers: HeaderMap, Path((id, cid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::COMMITMENT, id, cid, sha).await
}

async fn commitment_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::COMMITMENT, id, cid, multipart).await
}

async fn commitment_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::finance::commitment_form(r, Id::new(id), Some(Id::new(cid))))
}

async fn commitment_save_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, body: Bytes) -> Response {
    commitment_save_impl(app, headers, id, None, body)
}

async fn commitment_save(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, body: Bytes) -> Response {
    commitment_save_impl(app, headers, id, Some(cid), body)
}

/// Saves the commitment form: its fields and the `lines.N.*` rows.
fn commitment_save_impl(app: Shared, headers: HeaderMap, project: u64, commitment: Option<u64>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let base = format!("/projects/{project}/commitments");
    let back = match commitment {
        Some(c) => format!("{base}/{c}/edit"),
        None => format!("{base}/new"),
    };
    let mut params = Map::new();
    params.insert("project".into(), json!(project));
    if let Some(c) = commitment {
        params.insert("id".into(), json!(c));
    }
    for key in ["title", "kind", "vendor", "description", "retainage"] {
        params.insert(key.into(), Value::String(get_form(&form, key).to_string()));
    }
    params.insert("lines".into(), indexed_rows(&form, "lines"));
    let command = if commitment.is_some() { "commitment.update" } else { "commitment.create" };
    match inner.run(Actor::Person(s.person), command, &Value::Object(params)) {
        Ok(out) => {
            let id = commitment.or_else(|| out.get("id").and_then(Value::as_u64)).unwrap_or(0);
            redirect(&with_flash(&format!("{base}/{id}"), true, "Commitment saved."))
        }
        Err(e) => {
            let back = inner.draft_path(s.person.raw(), &save_action(&back), &form, &back);
            redirect(&with_flash(&back, false, &e.to_string()))
        }
    }
}

async fn invoice_list<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| invoicing::list::<T>(r, Id::new(id)))
}

async fn invoice_detail<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| invoicing::detail::<T>(r, Id::new(id), Id::new(iid), &t.store.history(T::TABLE, iid).unwrap_or_default()))
}

async fn invoice_print<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| invoicing::print::<T>(r, Id::new(id), Id::new(iid)))
}

async fn invoice_csv<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = invoicing::csv::<T>(&req, Id::new(id), Id::new(iid)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}-{iid}.csv\"", T::PATH)) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

/// Saves the amounts typed on a draft invoice (`amount.<row key>`), its period and its notes.
async fn invoice_save<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let back = format!("/projects/{id}/{}/{iid}", T::PATH);
    let lines: Vec<Value> =
        form.iter().filter_map(|(k, v)| k.strip_prefix("amount.").map(|row| json!({"row": row, "amount": v}))).take(1000).collect();
    let params = json!({"project": id, "id": iid, "period_end": get_form(&form, "period_end"), "notes": get_form(&form, "notes"), "lines": lines});
    match inner.run(Actor::Person(s.person), &format!("{}.update", T::PREFIX), &params) {
        Ok(_) => redirect(&with_flash(&back, true, "Amounts saved.")),
        Err(e) => {
            let action = format!("{back}/save");
            let back = inner.draft_path(s.person.raw(), &action, &form, &back);
            redirect(&with_flash(&back, false, &e.to_string()))
        }
    }
}

/// Registers the pages of an invoice kind under `/projects/{id}/<path>`.
fn invoice_routes<T: InvoicePage>(router: Router<Shared>) -> Router<Shared> {
    let base = format!("/projects/{{id}}/{}", T::PATH);
    router
        .route(&base, get(invoice_list::<T>))
        .route(&format!("{base}/{{iid}}"), get(invoice_detail::<T>))
        .route(&format!("{base}/{{iid}}/save"), post(invoice_save::<T>))
        .route(&format!("{base}/{{iid}}/print"), get(invoice_print::<T>))
        .route(
            &format!("{base}/{{iid}}/pdf"),
            get(|s: State<Shared>, h: HeaderMap, Path((id, iid)): Path<(u64, u64)>| async move {
                pdf_for(&s.0, &h, |r| invoicing::pdf::<T>(r, Id::new(id), Id::new(iid)))
            }),
        )
        .route(&format!("{base}/{{iid}}/csv"), get(invoice_csv::<T>))
}

async fn notifications(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::inbox::page(r))
}

async fn documents_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let folder = q.get("folder").and_then(|f| f.parse::<u64>().ok()).map(Id::new);
    let text = q.get("q").cloned().unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::documents::page(r, Id::new(id), folder, &text))
}

/// A version's file of a document the person may open, with its place on disk.
fn document_version_file(app: &Shared, headers: &HeaderMap, id: u64, did: u64, sha: &str) -> Result<(std::path::PathBuf, FileRef), Box<Response>> {
    let inner = app.lock();
    let Some(s) = session(app, &inner, headers) else { return Err(Box::new(redirect("/login"))) };
    let Some(t) = inner.tenant.as_ref() else { return Err(Box::new(redirect("/setup"))) };
    let not_found = || Box::new(plain(StatusCode::NOT_FOUND, "Not found"));
    let state = t.engine.state();
    let d = state
        .documents
        .get(Id::new(did))
        .filter(|d| d.project == Id::new(id) && d.deleted.is_none() && view::can_see_document(state, Actor::Person(s.person), d))
        .ok_or_else(not_found)?;
    let file = d.versions.iter().find(|v| v.file.sha256 == sha).map(|v| v.file.clone()).ok_or_else(not_found)?;
    let path = t.store.blobs.path(&file.sha256).ok_or_else(not_found)?;
    Ok((path, file))
}

/// The document page, with a preview of the current version when it is a PDF or a picture the viewer can draw.
async fn document_page(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    let current = {
        let inner = app.lock();
        inner.tenant.as_ref().and_then(|t| t.engine.state().documents.get(Id::new(did))).and_then(|d| d.current().map(|v| v.file.sha256.clone()))
    };
    let preview = match current {
        Some(sha) => match document_version_file(&app, &headers, id, did, &sha) {
            Ok((path, _)) => {
                let want = q.get("page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1);
                sheet_info(&app, path, &sha).await.map(|info| buildkiosk_web::documents::Preview {
                    sha,
                    pages: info.pages(),
                    page: want.min(info.pages()),
                })
            }
            Err(_) => None,
        },
        None => None,
    };
    page(&app, &headers, &q, |r, _| buildkiosk_web::documents::detail(r, Id::new(id), Id::new(did), preview.as_ref()))
}

/// One page of a document's PDF, or the picture itself, drawn for the page's preview.
async fn document_view(State(app): State<Shared>, headers: HeaderMap, Path((id, did, sha)): Path<(u64, u64, String)>, Query(q): Q) -> Response {
    let (path, file) = match document_version_file(&app, &headers, id, did, &sha) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let page = q.get("page").and_then(|p| p.parse::<usize>().ok()).unwrap_or(1).max(1);
    let width = q.get("w").and_then(|w| w.parse::<u32>().ok()).unwrap_or(900);
    match sheet_picture(&app, path, &file, page - 1, width).await {
        Some((bytes, kind)) => picture_response(bytes, kind),
        None => plain(StatusCode::NOT_FOUND, "This file can't be shown"),
    }
}

async fn document_download(State(app): State<Shared>, headers: HeaderMap, Path((id, did, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::DOCUMENT, id, did, sha).await
}

async fn document_upload_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, multipart: Multipart) -> Response {
    document_upload(app, headers, id, None, multipart).await
}

async fn document_upload_version(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    document_upload(app, headers, id, Some(did), multipart).await
}

async fn documents_trash(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::documents::trash(r, Id::new(id)))
}

/// Several files at once into one folder: a multipart form with `_csrf`, `folder` and many `file`s.
async fn document_upload_many(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, mut multipart: Multipart) -> Response {
    let base = format!("/projects/{id}/documents");
    let (mut token, mut folder) = (String::new(), String::new());
    let mut files: Vec<UploadFile> = Vec::new();
    let mut total = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&base, false, "The upload failed or was larger than 100 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&base, false, "The upload failed or was larger than 100 MB.")) };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "folder" => folder = String::from_utf8_lossy(&bytes).trim().to_string(),
            "file" if !bytes.is_empty() => {
                total = total.saturating_add(bytes.len());
                if files.len() >= MAX_BULK_FILES || total > MAX_BULK || bytes.len() > MAX_UPLOAD {
                    return redirect(&with_flash(&base, false, &format!("At most {MAX_BULK_FILES} files, 25 MB each and 100 MB in all.")));
                }
                files.push((filename, mime, bytes));
            }
            _ => {}
        }
    }
    let back = if folder.parse::<u64>().is_ok() { format!("{base}?folder={folder}") } else { base.clone() };
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    if files.is_empty() {
        return redirect(&with_flash(&back, false, "Choose at least one file."));
    }
    let (mut added, mut failed) = (0usize, Vec::new());
    for (name, mime, bytes) in &files {
        let stored = match store_blob(&mut inner, bytes) {
            Ok(x) => x,
            Err(msg) => {
                failed.push(format!("{name}: {msg}"));
                continue;
            }
        };
        let params = json!({"project": id, "folder": folder, "sha256": stored.0, "name": name, "size": stored.1, "mime": mime});
        match inner.run(Actor::Person(s.person), "document.add", &params) {
            Ok(_) => added += 1,
            Err(e) => failed.push(format!("{name}: {e}")),
        }
    }
    if failed.is_empty() {
        redirect(&with_flash(&back, true, &format!("Uploaded {added} file{}.", if added == 1 { "" } else { "s" })))
    } else {
        let first: Vec<String> = failed.iter().take(3).cloned().collect();
        redirect(&with_flash(&back, false, &format!("Uploaded {added} of {}. Not uploaded: {}", files.len(), first.join("; "))))
    }
}

/// A folder (with everything below it), or every document, as one zip file of current versions.
async fn documents_zip(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let folder: Option<u64> = q.get("folder").and_then(|f| f.parse().ok());
    // (path inside the zip, file on disk, size)
    let (name, files) = {
        let inner = app.lock();
        let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
        let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
        let state = t.engine.state();
        let actor = Actor::Person(s.person);
        if view::level(state, actor, Id::new(id), Tool::Documents) < Level::Read
            || state.projects.get(Id::new(id)).is_none_or(|p| p.deleted.is_some())
        {
            return plain(StatusCode::NOT_FOUND, "Not found");
        }
        let root: Option<Id<buildkiosk_engine::model::document::Folder>> = folder.map(Id::new);
        if let Some(r) = root
            && !state.folders.get(r).is_some_and(|f| f.project == Id::new(id) && f.deleted.is_none() && view::can_see_folder(state, actor, f))
        {
            return plain(StatusCode::NOT_FOUND, "Not found");
        }
        // The folder path of a document, relative to the folder being downloaded.
        let path_of = |mut at: Option<Id<buildkiosk_engine::model::document::Folder>>| -> Option<Vec<String>> {
            let mut segs = Vec::new();
            for _ in 0..=view::MAX_FOLDER_DEPTH {
                match at {
                    None => {
                        segs.reverse();
                        return if root.is_none() { Some(segs) } else { None };
                    }
                    Some(f) if Some(f) == root => {
                        segs.reverse();
                        return Some(segs);
                    }
                    Some(f) => {
                        let fx = state.folders.get(f)?;
                        segs.push(zip::clean_segment(&fx.name));
                        at = fx.parent;
                    }
                }
            }
            None
        };
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut files: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
        let mut docs: Vec<_> =
            state.documents.iter().filter(|d| d.project == Id::new(id) && d.deleted.is_none() && view::can_see_document(state, actor, d)).collect();
        docs.sort_by_key(|d| d.id.raw());
        for d in docs {
            let (Some(segs), Some(cur)) = (path_of(d.folder), d.current()) else { continue };
            let Some(disk) = t.store.blobs.path(&cur.file.sha256) else { continue };
            let mut name = zip::clean_segment(&cur.file.name);
            let dir = segs.join("/");
            let full = |n: &str| if dir.is_empty() { n.to_string() } else { format!("{dir}/{n}") };
            let mut n = 1;
            while !used.insert(full(&name).to_lowercase()) {
                n += 1;
                name = match cur.file.name.rsplit_once('.') {
                    Some((stem, ext)) => format!("{} ({n}).{}", zip::clean_segment(stem), zip::clean_segment(ext)),
                    None => format!("{} ({n})", zip::clean_segment(&cur.file.name)),
                };
            }
            files.push((full(&name), disk, cur.file.size));
        }
        let label = match root.and_then(|r| state.folders.get(r)) {
            Some(f) => zip::clean_segment(&f.name),
            None => state.projects.get(Id::new(id)).map(|p| zip::clean_segment(&p.name)).unwrap_or_else(|| "documents".into()),
        };
        (label, files)
    };
    if files.is_empty() {
        return redirect(&with_flash(&format!("/projects/{id}/documents"), false, "There are no documents to download."));
    }
    if files.iter().map(|f| f.2).sum::<u64>() > MAX_ZIP {
        return redirect(&with_flash(&format!("/projects/{id}/documents"), false, "That is more than 200 MB. Download a smaller folder."));
    }
    let built = tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for (path, disk, _) in files {
            entries.push(zip::Entry { name: path, data: std::fs::read(disk).ok()? });
        }
        zip::build(&entries)
    })
    .await
    .ok()
    .flatten();
    let Some(bytes) = built else { return plain(StatusCode::INTERNAL_SERVER_ERROR, "The zip file could not be made.") };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}.zip\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

type UploadFile = (String, String, Bytes);

/// The text fields and the (first) file of a multipart form.
async fn read_multipart(mut multipart: Multipart) -> Result<(HashMap<String, String>, Option<UploadFile>), ()> {
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut file: Option<UploadFile> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return Err(()),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else { return Err(()) };
        if name == "file" {
            if file.is_none() {
                file = Some((filename, mime, bytes));
            }
        } else if bytes.len() <= 20_000 {
            fields.entry(name).or_insert_with(|| String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    Ok((fields, file))
}

const UPLOAD_FAILED: &str = "The upload failed or the file is larger than 25 MB.";

/// A new document (with its folder, title and description) or a new version of one (with a note).
async fn document_upload(app: Shared, headers: HeaderMap, id: u64, did: Option<u64>, multipart: Multipart) -> Response {
    let mut back = match did {
        Some(d) => format!("/projects/{id}/documents/{d}"),
        None => format!("/projects/{id}/documents"),
    };
    let Ok((fields, file)) = read_multipart(multipart).await else { return redirect(&with_flash(&back, false, UPLOAD_FAILED)) };
    let folder = fields.get("folder").map(|f| f.trim().to_string()).filter(|f| !f.is_empty());
    if did.is_none()
        && let Some(f) = &folder
        && f.parse::<u64>().is_ok()
    {
        back = format!("{back}?folder={f}");
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, fields.get("_csrf").map(String::as_str).unwrap_or_default()) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return redirect(&with_flash(&back, false, "Choose a file to upload."));
    };
    let (sha256, size) = match store_blob(&mut inner, &bytes) {
        Ok(x) => x,
        Err(msg) => return redirect(&with_flash(&back, false, &msg)),
    };
    let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
    let (command, params) = match did {
        Some(d) => {
            ("document.add_version", json!({"project": id, "id": d, "note": get("note"), "sha256": sha256, "name": name, "size": size, "mime": mime}))
        }
        None => (
            "document.add",
            json!({"project": id, "folder": get("folder"), "title": get("title"), "description": get("description"), "sha256": sha256, "name": name, "size": size, "mime": mime}),
        ),
    };
    match inner.run(Actor::Person(s.person), command, &params) {
        Ok(out) => match (did, out.get("id").and_then(Value::as_u64)) {
            (None, Some(new)) => redirect(&with_flash(&format!("/projects/{id}/documents/{new}"), true, "File uploaded.")),
            _ => redirect(&with_flash(&back, true, "File uploaded.")),
        },
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

/// Puts an uploaded file in the blob store: its SHA-256 and size, or a message for the person.
fn store_blob(inner: &mut Inner, bytes: &Bytes) -> Result<(String, u64), String> {
    if bytes.len() > MAX_UPLOAD {
        return Err("Files can be at most 25 MB.".into());
    }
    let t = inner.tenant.as_mut().ok_or("The company is not set up.")?;
    t.store.blobs.put(bytes).map_err(|e| e.to_string())
}

async fn transmittals_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::transmittal::TransmittalStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::transmittals::list(r, Id::new(id), status, &text))
}

async fn transmittal_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::transmittals::form(r, Id::new(id), None))
}

async fn transmittal_page(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::transmittals::detail(r, Id::new(id), Id::new(tid)))
}

async fn transmittal_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::transmittals::form(r, Id::new(id), Some(Id::new(tid))))
}

async fn transmittal_download(State(app): State<Shared>, headers: HeaderMap, Path((id, tid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::TRANSMITTAL, id, tid, sha).await
}

async fn transmittal_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::TRANSMITTAL, id, tid, multipart).await
}

async fn meetings_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::meeting::MeetingStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::meetings::list(r, Id::new(id), status, &text))
}

async fn meeting_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::meetings::form(r, Id::new(id), None))
}

async fn meeting_page(State(app): State<Shared>, headers: HeaderMap, Path((id, mid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::meetings::detail(r, Id::new(id), Id::new(mid)))
}

async fn meeting_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, mid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::meetings::form(r, Id::new(id), Some(Id::new(mid))))
}

async fn meeting_download(State(app): State<Shared>, headers: HeaderMap, Path((id, mid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::MEETING, id, mid, sha).await
}

async fn meeting_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, mid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::MEETING, id, mid, multipart).await
}

async fn photos_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let album = q.get("album").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::photos::gallery(r, Id::new(id), &album, &text))
}

async fn photos_slideshow(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let album = q.get("album").map(|a| a.chars().take(100).collect::<String>()).unwrap_or_default();
    let at = q.get("i").and_then(|i| i.parse::<usize>().ok()).unwrap_or(0);
    let play = q.get("play").is_some_and(|p| p == "1");
    page(&app, &headers, &q, |r, _| buildkiosk_web::photos::slideshow(r, Id::new(id), &album, at, play))
}

/// The pictures of an album (or all of them) as one zip file: the originals, named by day and caption.
async fn photos_zip(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let album = q.get("album").map(|a| a.chars().take(100).collect::<String>()).unwrap_or_default();
    let back = format!("/projects/{id}/photos");
    let (name, files) = {
        let inner = app.lock();
        let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
        let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
        let state = t.engine.state();
        if view::level(state, Actor::Person(s.person), Id::new(id), Tool::Photos) < Level::Read {
            return plain(StatusCode::NOT_FOUND, "Not found");
        }
        let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut files: Vec<(String, std::path::PathBuf, u64)> = Vec::new();
        let mut shots: Vec<_> = buildkiosk_engine::cmd::photo::visible(state, Id::new(id))
            .into_iter()
            .filter(|x| album.is_empty() || x.album.eq_ignore_ascii_case(&album))
            .take(5_000)
            .collect();
        shots.sort_by_key(|x| (x.taken, x.id.raw()));
        for x in shots {
            let Some(disk) = t.store.blobs.path(&x.file.sha256) else { continue };
            let day = x.taken.map(|d| d.to_string()).unwrap_or_default();
            let mut n = 1;
            let mut name = zip::clean_segment(&format!("{day} {}", x.file.name));
            while !used.insert(name.to_lowercase()) {
                n += 1;
                name = zip::clean_segment(&format!("{day} ({n}) {}", x.file.name));
            }
            files.push((name, disk, x.file.size));
        }
        let label = if album.is_empty() {
            state.projects.get(Id::new(id)).map(|p| format!("{} photos", p.name)).unwrap_or_else(|| "photos".into())
        } else {
            album.clone()
        };
        (label, files)
    };
    if files.is_empty() {
        return redirect(&with_flash(&back, false, "There are no photos to download."));
    }
    if files.iter().map(|f| f.2).sum::<u64>() > MAX_ZIP {
        return redirect(&with_flash(&back, false, "That is more than 200 MB. Download one album at a time."));
    }
    let built = tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for (path, disk, _) in files {
            entries.push(zip::Entry { name: path, data: std::fs::read(disk).ok()? });
        }
        zip::build(&entries)
    })
    .await
    .ok()
    .flatten();
    let Some(bytes) = built else { return plain(StatusCode::INTERNAL_SERVER_ERROR, "The zip file could not be made.") };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/zip"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}.zip\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn photo_page(State(app): State<Shared>, headers: HeaderMap, Path((id, pid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::photos::detail(r, Id::new(id), Id::new(pid)))
}

/// The picture's type from its first bytes: only JPEG, PNG, GIF and WebP count.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
        Some("image/webp")
    } else {
        None
    }
}

/// The photo's file when the person may see it.
fn photo_file(app: &Shared, headers: &HeaderMap, id: u64, pid: u64) -> Result<(std::path::PathBuf, FileRef), Box<Response>> {
    let inner = app.lock();
    let Some(s) = session(app, &inner, headers) else { return Err(Box::new(redirect("/login"))) };
    let Some(t) = inner.tenant.as_ref() else { return Err(Box::new(redirect("/setup"))) };
    let state = t.engine.state();
    let found = state
        .photos
        .get(Id::new(pid))
        .filter(|x| {
            x.project == Id::new(id) && x.deleted.is_none() && view::level(state, Actor::Person(s.person), x.project, Tool::Photos) >= Level::Read
        })
        .and_then(|x| t.store.blobs.path(&x.file.sha256).map(|p| (p, x.file.clone())));
    found.ok_or_else(|| Box::new(plain(StatusCode::NOT_FOUND, "Not found")))
}

async fn photo_bytes(path: std::path::PathBuf) -> Result<Bytes, Box<Response>> {
    match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => Ok(Bytes::from(b)),
        _ => Err(Box::new(plain(StatusCode::NOT_FOUND, "That file is missing from storage."))),
    }
}

/// A photo shown in the page. Only a picture whose bytes still match one of the four allowed
/// types is sent with an image type; anything else is refused.
async fn photo_image(State(app): State<Shared>, headers: HeaderMap, Path((id, pid)): Path<(u64, u64)>) -> Response {
    let (path, file) = match photo_file(&app, &headers, id, pid) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let bytes = match photo_bytes(path).await {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(kind) = sniff_image(&bytes).filter(|k| *k == file.mime) else { return plain(StatusCode::NOT_FOUND, "Not a picture") };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    res.headers_mut().insert("cache-control", HeaderValue::from_static("private, max-age=3600"));
    res
}

/// The longest side of a gallery thumbnail, in pixels.
const THUMB_SIDE: u32 = 360;

/// A small copy of a picture: JPEG for a JPEG, PNG for the rest. `None` when the bytes can't be
/// decoded or are unreasonably large, in which case the full picture is sent instead.
fn thumbnail(bytes: &[u8], jpeg: bool) -> Option<Vec<u8>> {
    use image::ImageDecoder;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20_000);
    limits.max_image_height = Some(20_000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut decoder = reader.into_decoder().ok()?;
    decoder.set_limits(limits).ok()?;
    // Phones store pictures sideways and say so in the metadata; the thumbnail is re-encoded without that
    // metadata, so turn it the right way up here.
    let orientation = decoder.orientation().ok();
    let mut img = image::DynamicImage::from_decoder(decoder).ok()?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let small = img.thumbnail(THUMB_SIDE, THUMB_SIDE);
    let mut out = std::io::Cursor::new(Vec::new());
    if jpeg {
        image::DynamicImage::ImageRgb8(small.to_rgb8()).write_to(&mut out, image::ImageFormat::Jpeg).ok()?;
    } else {
        small.write_to(&mut out, image::ImageFormat::Png).ok()?;
    }
    Some(out.into_inner())
}

/// A photo's thumbnail for the gallery; falls back to the picture itself if it can't be shrunk.
async fn photo_thumb(State(app): State<Shared>, headers: HeaderMap, Path((id, pid)): Path<(u64, u64)>) -> Response {
    let (path, file) = match photo_file(&app, &headers, id, pid) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let bytes = match photo_bytes(path).await {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let Some(kind) = sniff_image(&bytes).filter(|k| *k == file.mime) else { return plain(StatusCode::NOT_FOUND, "Not a picture") };
    let jpeg = kind == "image/jpeg";
    let source = bytes.clone();
    let small = tokio::task::spawn_blocking(move || thumbnail(&source, jpeg)).await.ok().flatten();
    let (body, kind) = match small {
        Some(t) => (Bytes::from(t), if jpeg { "image/jpeg" } else { "image/png" }),
        None => (bytes, kind),
    };
    let mut res = body.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    res.headers_mut().insert("cache-control", HeaderValue::from_static("private, max-age=86400"));
    res
}

async fn photo_download(State(app): State<Shared>, headers: HeaderMap, Path((id, pid)): Path<(u64, u64)>) -> Response {
    let (path, file) = match photo_file(&app, &headers, id, pid) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    let bytes = match photo_bytes(path).await {
        Ok(b) => b,
        Err(r) => return *r,
    };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&file.name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

/// Every picture in the form (several `file` fields), each stored and added; the fields are the
/// shared album, place, date and caption.
async fn photo_upload(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/photos");
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut files: Vec<UploadFile> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, UPLOAD_FAILED)),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("photo").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&back, false, UPLOAD_FAILED)) };
        if name == "file" {
            if !bytes.is_empty() && files.len() < 100 {
                files.push((filename, String::new(), bytes));
            }
        } else if bytes.len() <= 20_000 {
            fields.entry(name).or_insert_with(|| String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, fields.get("_csrf").map(String::as_str).unwrap_or_default()) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    if files.is_empty() {
        return redirect(&with_flash(&back, false, "Choose at least one picture."));
    }
    let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
    let (mut added, mut skipped) = (0usize, 0usize);
    let mut last_error = String::new();
    for (name, _, bytes) in files {
        let Some(kind) = sniff_image(&bytes) else {
            skipped += 1;
            continue;
        };
        let (sha256, size) = match store_blob(&mut inner, &bytes) {
            Ok(x) => x,
            Err(msg) => return redirect(&with_flash(&back, false, &msg)),
        };
        let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes.as_ref()))
            .with_guessed_format()
            .ok()
            .and_then(|r| r.into_dimensions().ok())
            .unwrap_or((0, 0));
        let exif = if kind == "image/jpeg" { exif::read(&bytes) } else { exif::Exif::default() };
        // The day typed in wins; otherwise the day the camera recorded; otherwise today (the engine's default).
        let taken = Some(get("taken")).filter(|t| !t.trim().is_empty()).or_else(|| exif.taken.map(|d| d.to_string()));
        let mut params = json!({"project": id, "sha256": sha256, "name": name, "size": size, "mime": kind,
            "caption": get("caption"), "album": get("album"), "location": get("location"), "width": width, "height": height, "camera": exif.camera});
        if let (Some(t), Some(o)) = (taken, params.as_object_mut()) {
            o.insert("taken".into(), Value::String(t));
        }
        match inner.run(Actor::Person(s.person), "photo.add", &params) {
            Ok(_) => added += 1,
            Err(e) => last_error = e.to_string(),
        }
    }
    if added == 0 {
        let msg = if last_error.is_empty() { "Photos must be JPEG, PNG, GIF or WebP pictures.".to_string() } else { last_error };
        return redirect(&with_flash(&back, false, &msg));
    }
    let note = if skipped > 0 {
        format!("{added} added; {skipped} skipped because they are not JPEG, PNG, GIF or WebP pictures.")
    } else {
        format!("{added} added.")
    };
    redirect(&with_flash(&back, true, &note))
}

async fn correspondence_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::correspondence::CorrespondenceStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::list(r, Id::new(id), status, &text))
}

async fn correspondence_types(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::types_page(r, Id::new(id), None))
}

async fn correspondence_type_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::types_page(r, Id::new(id), Some(Id::new(tid))))
}

async fn correspondence_incoming(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::incoming_page(r, Id::new(id)))
}

async fn correspondence_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::form(r, Id::new(id), None))
}

async fn correspondence_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::detail(r, Id::new(id), Id::new(cid)))
}

async fn correspondence_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::correspondence::form(r, Id::new(id), Some(Id::new(cid))))
}

async fn correspondence_download(State(app): State<Shared>, headers: HeaderMap, Path((id, cid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::CORRESPONDENCE, id, cid, sha).await
}

async fn correspondence_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, cid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::CORRESPONDENCE, id, cid, multipart).await
}

async fn forms_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let template = q.get("template").and_then(|t| t.parse::<u64>().ok()).map(Id::new);
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::form::FormStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::forms::list(r, Id::new(id), template, status, &text))
}

async fn form_templates_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::forms::templates(r, Id::new(id), None))
}

async fn form_template_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::forms::templates(r, Id::new(id), Some(Id::new(tid))))
}

async fn form_page(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::forms::detail(r, Id::new(id), Id::new(rid)))
}

async fn form_download(State(app): State<Shared>, headers: HeaderMap, Path((id, rid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::FORM, id, rid, sha).await
}

async fn form_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::FORM, id, rid, multipart).await
}

async fn timesheets_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let week = q.get("week").and_then(|w| buildkiosk_core::Date::parse(w));
    page(&app, &headers, &q, |r, _| buildkiosk_web::timesheets::page(r, Id::new(id), week))
}

async fn timesheets_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let week = q.get("week").and_then(|w| buildkiosk_core::Date::parse(w));
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::timesheets::csv(&req, Id::new(id), week) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"timesheets.csv\""));
    res
}

async fn timesheets_cost(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let (from, to) = (q.get("from").and_then(|w| buildkiosk_core::Date::parse(w)), q.get("to").and_then(|w| buildkiosk_core::Date::parse(w)));
    let all = q.get("include").is_some_and(|v| v == "all");
    page(&app, &headers, &q, |r, _| buildkiosk_web::timesheets::cost_page(r, Id::new(id), from, to, all))
}

async fn timesheets_cost_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let (from, to) = (q.get("from").and_then(|w| buildkiosk_core::Date::parse(w)), q.get("to").and_then(|w| buildkiosk_core::Date::parse(w)));
    let all = q.get("include").is_some_and(|v| v == "all");
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::timesheets::cost_csv(&req, Id::new(id), from, to, all) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"labour-cost.csv\""));
    res
}

async fn reports_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::reports::page(r, Id::new(id)))
}

async fn reports_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::reports::csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"project-report.csv\""));
    res
}

async fn action_plans_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::action_plan::PlanStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::action_plans::list(r, Id::new(id), status, &text))
}

async fn action_plan_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::action_plans::form(r, Id::new(id), None))
}

async fn action_plan_page(State(app): State<Shared>, headers: HeaderMap, Path((id, aid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::action_plans::detail(r, Id::new(id), Id::new(aid)))
}

async fn action_plan_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, aid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::action_plans::form(r, Id::new(id), Some(Id::new(aid))))
}

async fn bidding_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let status = q.get("status").and_then(|s| buildkiosk_engine::model::bidding::BidStatus::parse(s));
    let text = q.get("q").map(|s| s.chars().take(100).collect::<String>()).unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::bidding::list(r, Id::new(id), status, &text))
}

async fn bid_package_download(State(app): State<Shared>, headers: HeaderMap, Path((id, bid, sha)): Path<(u64, u64, String)>) -> Response {
    let found = {
        let inner = app.lock();
        let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
        let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
        let state = t.engine.state();
        let may = view::level(state, Actor::Person(s.person), Id::new(id), Tool::Bidding) >= buildkiosk_engine::model::Level::Read;
        let file = state
            .bid_packages
            .get(Id::new(bid))
            .filter(|b| may && b.project.raw() == id && b.deleted.is_none())
            .and_then(|b| b.attachments.iter().chain(b.bids.iter().flat_map(|x| x.files.iter())).find(|f| f.sha256 == sha).cloned());
        file.and_then(|a| t.store.blobs.path(&a.sha256).map(|p| (p, a.name.clone())))
    };
    let Some((path, name)) = found else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        _ => return plain(StatusCode::NOT_FOUND, "That file is missing from storage."),
    };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn bid_package_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, bid)): Path<(u64, u64)>, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/bidding/{bid}");
    let (mut token, mut vendor, mut file) = (String::new(), String::new(), None);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else {
            return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB."));
        };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "vendor" => vendor = String::from_utf8_lossy(&bytes).trim().chars().take(20).collect(),
            "file" if file.is_none() => file = Some((filename, mime, bytes)),
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return redirect(&with_flash(&back, false, "Choose a file to attach."));
    };
    if bytes.len() > MAX_UPLOAD {
        return redirect(&with_flash(&back, false, "Files can be at most 25 MB."));
    }
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let (sha256, size) = match t.store.blobs.put(&bytes) {
        Ok(x) => x,
        Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
    };
    let mut params = json!({"project": id, "id": bid, "sha256": sha256, "name": name, "size": size, "mime": mime});
    if let (false, Some(map)) = (vendor.is_empty(), params.as_object_mut()) {
        map.insert("vendor".into(), json!(vendor));
    }
    match inner.run(Actor::Person(s.person), "bid_package.attach", &params) {
        Ok(_) => redirect(&with_flash(&back, true, "File attached.")),
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

async fn bid_package_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::bidding::form(r, Id::new(id), None))
}

async fn bid_package_page(State(app): State<Shared>, headers: HeaderMap, Path((id, bid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::bidding::detail(r, Id::new(id), Id::new(bid)))
}

async fn bid_package_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, bid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::bidding::form(r, Id::new(id), Some(Id::new(bid))))
}

async fn estimate_compare(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let get_id = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok()).map(Id::new);
    let (Some(a), Some(b)) = (get_id("a"), get_id("b")) else { return redirect(&format!("/projects/{id}/estimating")) };
    page(&app, &headers, &q, |r, _| buildkiosk_web::estimating::compare(r, Id::new(id), a, b))
}

async fn catalog_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let number = |k: &str| q.get(k).and_then(|v| v.parse::<u64>().ok());
    let (item, assembly) = (number("edit").map(Id::new), number("assembly").map(Id::new));
    let needle = q.get("q").cloned().unwrap_or_default();
    page(&app, &headers, &q, |r, _| buildkiosk_web::catalog::page(r, &needle, item, assembly))
}

async fn estimating_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::estimating::list(r, Id::new(id)))
}

async fn estimate_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::estimating::form(r, Id::new(id), None))
}

async fn estimate_page(State(app): State<Shared>, headers: HeaderMap, Path((id, eid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    let takeoff = q.get("takeoff").and_then(|t| t.parse::<u64>().ok()).map(Id::new);
    page(&app, &headers, &q, |r, _| buildkiosk_web::estimating::detail(r, Id::new(id), Id::new(eid), takeoff))
}

async fn estimate_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, eid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::estimating::form(r, Id::new(id), Some(Id::new(eid))))
}

async fn estimate_csv(State(app): State<Shared>, headers: HeaderMap, Path((id, eid)): Path<(u64, u64)>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::estimating::csv(&req, Id::new(id), Id::new(eid)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"estimate.csv\""));
    res
}

async fn schedule_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    let show = q.get("show").cloned().unwrap_or_default();
    // Dragging bars needs the browser module; its page may run it (and nothing else).
    let drag = viewer_assets::START_JS.is_some();
    let mut res = page(&app, &headers, &q, |r, _| buildkiosk_web::schedule::page(r, Id::new(id), &show, drag));
    if drag && res.status() == StatusCode::OK {
        res.headers_mut().insert("content-security-policy", HeaderValue::from_static(VIEWER_CSP));
    }
    res
}

async fn schedule_task_page(State(app): State<Shared>, headers: HeaderMap, Path((id, tid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::schedule::detail(r, Id::new(id), Id::new(tid)))
}

async fn schedule_xml(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let state = t.engine.state();
    let project = Id::new(id);
    if view::level(state, Actor::Person(s.person), project, Tool::Schedule) < buildkiosk_engine::model::Level::Read {
        return plain(StatusCode::NOT_FOUND, "Not found");
    }
    let today = Timestamp::from_millis(now_ms()).date_at(state.utc_offset());
    let xml = buildkiosk_engine::cmd::schedule_xml::export_xml(state, project, today);
    let mut res = xml.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/xml; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"schedule.xml\""));
    res
}

async fn schedule_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::schedule::csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"schedule.csv\""));
    res
}

async fn drawings_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::drawings::page(r, Id::new(id), &q))
}

async fn drawing_page(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::drawings::detail(r, Id::new(id), Id::new(did)))
}

async fn drawing_download(State(app): State<Shared>, headers: HeaderMap, Path((id, did, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::DRAWING, id, did, sha).await
}

async fn drawings_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::drawings::csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"drawings.csv\""));
    res
}

/// The sheet number and title in a file name: "A-101 Floor plan.pdf" is sheet A-101, "Floor plan".
fn sheet_from_file_name(file: &str) -> (String, String) {
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file).trim();
    let (number, rest) = match stem.find([' ', '_']) {
        Some(at) => (stem.get(..at).unwrap_or(stem), stem.get(at..).unwrap_or("")),
        None => (stem, ""),
    };
    let title = rest.trim_matches(|c: char| c.is_whitespace() || c == '_' || c == '-').replace('_', " ");
    (number.chars().take(50).collect(), title.chars().take(200).collect())
}

/// The discipline a sheet number's letters suggest ("S-201" is structural).
fn discipline_for_sheet(number: &str) -> &'static str {
    let upper = number.to_ascii_uppercase();
    if upper.starts_with("FP") {
        return "fire_protection";
    }
    match upper.chars().next() {
        Some('A') => "architectural",
        Some('S') => "structural",
        Some('M') => "mechanical",
        Some('P') => "plumbing",
        Some('E') => "electrical",
        Some('C') => "civil",
        Some('L') => "landscape",
        Some('I') => "interior",
        _ => "general",
    }
}

/// A set of sheets at once: each file becomes a sheet named by its file name (or a new revision of
/// a sheet already logged). A multipart form with `_csrf`, `revision`, `set`, `date` and many `file`s.
async fn drawing_upload_set(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/drawings");
    let failed_upload = "The upload failed or was larger than 100 MB.";
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut files: Vec<UploadFile> = Vec::new();
    let mut total = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, failed_upload)),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&back, false, failed_upload)) };
        if name == "file" {
            if bytes.is_empty() {
                continue;
            }
            total = total.saturating_add(bytes.len());
            if files.len() >= MAX_BULK_FILES || total > MAX_BULK || bytes.len() > MAX_UPLOAD {
                return redirect(&with_flash(&back, false, &format!("At most {MAX_BULK_FILES} sheets, 25 MB each and 100 MB in all.")));
            }
            files.push((filename, mime, bytes));
        } else if bytes.len() <= 2_000 {
            fields.insert(name, String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, fields.get("_csrf").map(String::as_str).unwrap_or_default()) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    if files.is_empty() {
        return redirect(&with_flash(&back, false, "Choose at least one sheet file."));
    }
    let get = |k: &str| fields.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    let revision = Some(get("revision")).filter(|r| !r.is_empty()).unwrap_or_else(|| "1".into());
    let actor = Actor::Person(s.person);
    let (mut added, mut revised, mut failed) = (0usize, 0usize, Vec::new());
    for (file, mime, bytes) in &files {
        let (number, title) = sheet_from_file_name(file);
        let (sha256, size) = match store_blob(&mut inner, bytes) {
            Ok(x) => x,
            Err(msg) => {
                failed.push(format!("{file}: {msg}"));
                continue;
            }
        };
        let existing = inner.tenant.as_ref().and_then(|t| {
            t.engine
                .state()
                .drawings
                .iter()
                .find(|d| d.project == Id::new(id) && d.deleted.is_none() && d.number.eq_ignore_ascii_case(&number))
                .map(|d| d.id.raw())
        });
        let mut params = Map::new();
        for (k, v) in [
            ("revision", json!(revision)),
            ("set", json!(get("set"))),
            ("date", json!(get("date"))),
            ("sha256", json!(sha256)),
            ("name", json!(file)),
            ("size", json!(size)),
            ("mime", json!(mime)),
        ] {
            params.insert(k.into(), v);
        }
        params.insert("project".into(), json!(id));
        let command = match existing {
            Some(did) => {
                params.insert("id".into(), json!(did));
                "drawing.add_revision"
            }
            None => {
                params.insert("number".into(), json!(number));
                params.insert("title".into(), json!(title));
                params.insert("discipline".into(), json!(discipline_for_sheet(&number)));
                "drawing.add"
            }
        };
        let params = Value::Object(params);
        match inner.run(actor, command, &params) {
            Ok(_) if existing.is_some() => revised += 1,
            Ok(_) => added += 1,
            Err(e) => failed.push(format!("{file}: {e}")),
        }
    }
    let summary = format!("{added} new sheet{} and {revised} new revision{}", if added == 1 { "" } else { "s" }, if revised == 1 { "" } else { "s" });
    if failed.is_empty() {
        redirect(&with_flash(&back, true, &format!("Uploaded {summary}.")))
    } else {
        let first: Vec<String> = failed.iter().take(3).cloned().collect();
        redirect(&with_flash(&back, false, &format!("Uploaded {summary}. Not uploaded: {}", first.join("; "))))
    }
}

/// The file of a sheet's revision (the current one for an empty label) when the person may see it.
fn sheet_revision_file(app: &Shared, headers: &HeaderMap, id: u64, did: u64, rev: &str) -> Result<(std::path::PathBuf, FileRef), Box<Response>> {
    let inner = app.lock();
    let Some(s) = session(app, &inner, headers) else { return Err(Box::new(redirect("/login"))) };
    let Some(t) = inner.tenant.as_ref() else { return Err(Box::new(redirect("/setup"))) };
    let state = t.engine.state();
    let not_found = || Box::new(plain(StatusCode::NOT_FOUND, "Not found"));
    if view::level(state, Actor::Person(s.person), Id::new(id), Tool::Drawings) < Level::Read {
        return Err(not_found());
    }
    let d = state.drawings.get(Id::new(did)).filter(|d| d.project == Id::new(id) && d.deleted.is_none()).ok_or_else(not_found)?;
    let r = if rev.is_empty() { d.revisions.last() } else { d.revisions.iter().find(|r| r.label.eq_ignore_ascii_case(rev)) };
    let file = r.and_then(|r| r.file.clone()).ok_or_else(not_found)?;
    let path = t.store.blobs.path(&file.sha256).ok_or_else(not_found)?;
    Ok((path, file))
}

/// What a sheet file is (cached by its hash); reads and parses it off the main thread.
async fn sheet_info(app: &Shared, path: std::path::PathBuf, sha: &str) -> Option<std::sync::Arc<render::Info>> {
    if let Some(known) = app.lock().pictures.info(sha) {
        return known;
    }
    let found = tokio::task::spawn_blocking(move || std::fs::read(path).ok().and_then(render::info)).await.ok().flatten();
    let mut inner = app.lock();
    inner.pictures.set_info(sha, found);
    inner.pictures.info(sha).flatten()
}

fn view_from_query(q: &HashMap<String, String>) -> buildkiosk_web::drawings::View {
    let num = |k: &str| q.get(k).and_then(|v| v.trim().parse::<u32>().ok());
    let text = |k: &str| q.get(k).map(|v| v.chars().take(50).collect::<String>()).unwrap_or_default();
    let point = |x: &str, y: &str| match (num(x).and_then(|v| u16::try_from(v).ok()), num(y).and_then(|v| u16::try_from(v).ok())) {
        (Some(x), Some(y)) if x <= buildkiosk_engine::model::drawing::MARKUP_SCALE && y <= buildkiosk_engine::model::drawing::MARKUP_SCALE => {
            Some((x, y))
        }
        _ => None,
    };
    let tool = text("tool");
    buildkiosk_web::drawings::View {
        rev: text("rev"),
        page: num("page").unwrap_or(1).clamp(1, 2_000),
        width: num("w").unwrap_or(900).clamp(render::MIN_WIDTH, render::MAX_WIDTH),
        tool: if matches!(tool.as_str(), "pin" | "rect" | "arrow" | "text") { tool } else { String::new() },
        first: point("x1", "y1"),
        click: None,
        compare: text("compare"),
    }
}

/// The viewer page. A click on the picture arrives as `at.x` and `at.y` (pixels from the corner of
/// the picture); they are turned into positions on the page here.
async fn drawing_view(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    let mut v = view_from_query(&q);
    let file = sheet_revision_file(&app, &headers, id, did, &v.rev);
    let shape = match file {
        Ok((path, f)) => sheet_info(&app, path, &f.sha256).await.and_then(|info| {
            let page = usize::try_from(v.page).unwrap_or(1).clamp(1, info.pages());
            let (w, h) = info.dims.get(page - 1).copied()?;
            let interactive = viewer_assets::START_JS.is_some() && !info.image && v.compare.is_empty();
            Some(buildkiosk_web::drawings::PageShape { pages: info.pages(), width: w, height: h, interactive })
        }),
        Err(r) if r.status() != StatusCode::NOT_FOUND => return *r,
        Err(_) => None,
    };
    if let Some(shape) = shape {
        // Pixels clicked, as a fraction of the picture shown.
        let shown_h = (v.width as f32 * shape.height / shape.width).round().max(1.0);
        let scale = f32::from(buildkiosk_engine::model::drawing::MARKUP_SCALE);
        let at = |k: &str| q.get(k).and_then(|s| s.trim().parse::<f32>().ok()).filter(|n| n.is_finite() && *n >= 0.0);
        if let (Some(px), Some(py)) = (at("at.x"), at("at.y")) {
            let nx = (px / v.width as f32 * scale).round().clamp(0.0, scale) as u16;
            let ny = (py / shown_h * scale).round().clamp(0.0, scale) as u16;
            let two = matches!(v.tool.as_str(), "rect" | "arrow");
            if two && v.first.is_none() {
                v.first = Some((nx, ny));
            } else {
                v.click = Some((nx, ny));
            }
        }
    }
    let interactive = shape.is_some_and(|s| s.interactive);
    let mut res = page(&app, &headers, &q, |r, _| buildkiosk_web::drawings::viewer(r, Id::new(id), Id::new(did), &v, shape));
    if interactive {
        res.headers_mut().insert("content-security-policy", HeaderValue::from_static(VIEWER_CSP));
    }
    res
}

/// A sheet page as a picture: a PDF page drawn at the width asked for, or the picture itself.
async fn sheet_picture(
    app: &Shared,
    path: std::path::PathBuf,
    file: &FileRef,
    page: usize,
    width: u32,
) -> Option<(std::sync::Arc<Vec<u8>>, &'static str)> {
    let info = sheet_info(app, path.clone(), &file.sha256).await?;
    if info.image {
        let bytes = tokio::task::spawn_blocking(move || std::fs::read(path)).await.ok()?.ok()?;
        let kind = sniff_image(&bytes)?;
        return Some((std::sync::Arc::new(bytes), kind));
    }
    if page >= info.pages() {
        return None;
    }
    let width = width.clamp(render::MIN_WIDTH, render::MAX_WIDTH);
    if let Some(done) = app.lock().pictures.get(&file.sha256, page, width) {
        return Some((done, "image/png"));
    }
    let png = tokio::task::spawn_blocking(move || std::fs::read(path).ok().and_then(|b| render::render_page(b, page, width))).await.ok().flatten()?;
    Some((app.lock().pictures.put(&file.sha256, page, width, png), "image/png"))
}

fn picture_response(bytes: std::sync::Arc<Vec<u8>>, kind: &'static str) -> Response {
    let mut res = Bytes::copy_from_slice(&bytes).into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    res.headers_mut().insert("cache-control", HeaderValue::from_static("private, max-age=3600"));
    res
}

async fn drawing_view_png(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    let v = view_from_query(&q);
    let (path, file) = match sheet_revision_file(&app, &headers, id, did, &v.rev) {
        Ok(x) => x,
        Err(r) => return *r,
    };
    match sheet_picture(&app, path, &file, usize::try_from(v.page).unwrap_or(1).saturating_sub(1), v.width).await {
        Some((bytes, kind)) => picture_response(bytes, kind),
        None => plain(StatusCode::NOT_FOUND, "This sheet can't be shown"),
    }
}

/// Two revisions of a page laid over each other.
async fn drawing_compare_png(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, Query(q): Q) -> Response {
    let v = view_from_query(&q);
    let label = |k: &str| q.get(k).map(|s| s.chars().take(50).collect::<String>()).unwrap_or_default();
    let (a_label, b_label) = (label("a"), label("b"));
    let (a, b) = match (sheet_revision_file(&app, &headers, id, did, &a_label), sheet_revision_file(&app, &headers, id, did, &b_label)) {
        (Ok(a), Ok(b)) => (a, b),
        (Err(r), _) | (_, Err(r)) => return *r,
    };
    let page = usize::try_from(v.page).unwrap_or(1).saturating_sub(1);
    let (pa, pb) = (sheet_picture(&app, a.0, &a.1, page, v.width).await, sheet_picture(&app, b.0, &b.1, page, v.width).await);
    let (Some((pa, _)), Some((pb, _))) = (pa, pb) else { return plain(StatusCode::NOT_FOUND, "These sheets can't be compared") };
    match tokio::task::spawn_blocking(move || render::compare(&pa, &pb)).await.ok().flatten() {
        Some(png) => picture_response(std::sync::Arc::new(png), "image/png"),
        None => plain(StatusCode::NOT_FOUND, "These sheets can't be compared"),
    }
}

async fn drawing_upload_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, multipart: Multipart) -> Response {
    drawing_upload(app, headers, id, None, multipart).await
}

async fn drawing_upload_revision(State(app): State<Shared>, headers: HeaderMap, Path((id, did)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    drawing_upload(app, headers, id, Some(did), multipart).await
}

/// A new sheet or a new revision of one; the file is optional (a revision can be logged before the
/// sheet arrives).
async fn drawing_upload(app: Shared, headers: HeaderMap, id: u64, did: Option<u64>, multipart: Multipart) -> Response {
    let back = match did {
        Some(d) => format!("/projects/{id}/drawings/{d}"),
        None => format!("/projects/{id}/drawings"),
    };
    let Ok((fields, file)) = read_multipart(multipart).await else { return redirect(&with_flash(&back, false, UPLOAD_FAILED)) };
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, fields.get("_csrf").map(String::as_str).unwrap_or_default()) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
    let mut params = serde_json::Map::new();
    params.insert("project".into(), json!(id));
    for k in ["revision", "date", "set", "note"] {
        params.insert(k.into(), json!(get(k)));
    }
    if let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) {
        match store_blob(&mut inner, &bytes) {
            Ok((sha256, size)) => {
                params.insert("sha256".into(), json!(sha256));
                params.insert("name".into(), json!(name));
                params.insert("size".into(), json!(size));
                params.insert("mime".into(), json!(mime));
            }
            Err(msg) => return redirect(&with_flash(&back, false, &msg)),
        }
    }
    let command = match did {
        Some(d) => {
            params.insert("id".into(), json!(d));
            "drawing.add_revision"
        }
        None => {
            for k in ["number", "title", "discipline", "area"] {
                params.insert(k.into(), json!(get(k)));
            }
            "drawing.add"
        }
    };
    let params = Value::Object(params);
    match inner.run(Actor::Person(s.person), command, &params) {
        Ok(out) => match (did, out.get("id").and_then(Value::as_u64)) {
            (None, Some(new)) => redirect(&with_flash(&format!("/projects/{id}/drawings/{new}"), true, "Sheet added.")),
            _ => redirect(&with_flash(&back, true, "Revision added.")),
        },
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

async fn specifications(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::specs::page(r, Id::new(id)))
}

/// Imports a register from an uploaded CSV file (up to 1 MB).
async fn submittal_import_file(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, multipart: Multipart) -> Response {
    import_file(app, headers, id, "submittals", "submittal.import", "submittals", ("1 MB", "CSV file"), &[], multipart).await
}

async fn work_import_file<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, multipart: Multipart) -> Response {
    import_file(app, headers, id, T::PATH, &format!("{}.import", T::PREFIX), T::NOUN, ("1 MB", "CSV file"), &[], multipart).await
}

/// Runs an import command on an uploaded CSV file (a multipart form with `_csrf` and `file`).
#[allow(clippy::too_many_arguments)]
async fn import_file(
    app: Shared,
    headers: HeaderMap,
    id: u64,
    path: &str,
    command: &str,
    noun: &str,
    (limit, what): (&str, &str),
    extra: &[&str],
    mut multipart: Multipart,
) -> Response {
    let back = format!("/projects/{id}/{path}");
    let too_big = format!("The upload failed or the file is larger than {limit}.");
    let (mut token, mut file) = (String::new(), None);
    let mut params = serde_json::Map::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, &too_big)),
        };
        let name = field.name().unwrap_or("").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&back, false, &too_big)) };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if file.is_none() => file = Some(bytes),
            // Plain form fields the command takes besides the file (such as a tick box).
            other if extra.contains(&other) => {
                params.insert(other.to_string(), Value::String(String::from_utf8_lossy(&bytes).chars().take(100).collect()));
            }
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some(bytes) = file.filter(|b| !b.is_empty()) else { return redirect(&with_flash(&back, false, &format!("Choose a {what} to import."))) };
    let Ok(text) = String::from_utf8(bytes.to_vec()) else {
        return redirect(&with_flash(&back, false, &format!("The file isn't text. Save it as a {what} (UTF-8) and try again.")));
    };
    let text = text.trim_start_matches('\u{feff}');
    params.insert("project".into(), json!(id));
    params.insert("text".into(), json!(text));
    match inner.run(Actor::Person(s.person), command, &Value::Object(params)) {
        Ok(v) => {
            let n = v.get("created").and_then(Value::as_u64).unwrap_or(0);
            let codes = v
                .get("codes_added")
                .and_then(Value::as_u64)
                .filter(|c| *c > 0)
                .map(|c| format!(" Added {c} cost code{}.", if c == 1 { "" } else { "s" }))
                .unwrap_or_default();
            redirect(&with_flash(&back, true, &format!("Imported {n} {noun}{}.{codes}", if noun.ends_with('s') { "" } else { "s" })))
        }
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

async fn submittal_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::submittals::csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"submittals.csv\""));
    res
}

async fn submittal_new(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::submittals::form(r, Id::new(id), None))
}

async fn submittal_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, sid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| {
        buildkiosk_web::submittals::detail(r, Id::new(id), Id::new(sid), &t.store.history("submittals", sid).unwrap_or_default())
    })
}

async fn submittal_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, sid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::submittals::form(r, Id::new(id), Some(Id::new(sid))))
}

async fn not_found(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let mut res = page(&app, &headers, &HashMap::new(), |r, _| pages::not_found(r));
    if res.status() == StatusCode::OK {
        *res.status_mut() = StatusCode::NOT_FOUND;
    }
    res
}

/// What the sheet viewer page is allowed: its own script and WebAssembly module (nothing inline, nothing
/// from elsewhere), and fetching the sheet file from this server.
const VIEWER_CSP: &str = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; style-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

/// A file built into the server, stored gzipped (or "not found" when this build has none). Browsers
/// that accept gzip get it as is; anyone else gets it unpacked.
fn embedded(headers: &HeaderMap, gz: Option<&'static [u8]>, kind: &'static str) -> Response {
    use std::io::Read as _;
    let Some(gz) = gz else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let accepts = headers
        .get_all("accept-encoding")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .any(|v| v.split(',').any(|e| e.trim().split(';').next().is_some_and(|n| n.eq_ignore_ascii_case("gzip"))));
    let mut res = if accepts {
        let mut res = Bytes::from_static(gz).into_response();
        res.headers_mut().insert("content-encoding", HeaderValue::from_static("gzip"));
        res
    } else {
        let mut plain_bytes = Vec::new();
        if flate2::read::GzDecoder::new(gz).read_to_end(&mut plain_bytes).is_err() {
            return plain(StatusCode::INTERNAL_SERVER_ERROR, "Not available");
        }
        Bytes::from(plain_bytes).into_response()
    };
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static(kind));
    res.headers_mut().insert("vary", HeaderValue::from_static("accept-encoding"));
    // Not fingerprinted, so it is checked again within the hour.
    res.headers_mut().insert("cache-control", HeaderValue::from_static("public, max-age=3600"));
    res
}

async fn css() -> Response {
    let mut res = style::stylesheet().to_string().into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/css; charset=utf-8"));
    res.headers_mut().insert("cache-control", HeaderValue::from_static("public, max-age=31536000, immutable"));
    res
}

async fn favicon() -> Response {
    let mut res = icons::Icon::favicon().into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("image/svg+xml"));
    res
}

// -- sign-in, sign-out, setup

fn session_cookie(app: &App, inner: &Inner, person: u64) -> String {
    let value = auth::issue_session(app.secret(), person, inner.creds.epoch(person), now_ms());
    auth::set_cookie(SESSION_COOKIE, &value, auth::SESSION_MS / 1000, app.secure)
}

async fn login_page(State(app): State<Shared>, headers: HeaderMap) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    if session(&app, &inner, &headers).is_some() {
        return redirect("/");
    }
    let (token, set) = anon_csrf(&app, &headers);
    let name = t.engine.state().company().map(|c| c.name.clone()).unwrap_or_default();
    let res = html(StatusCode::OK, pages::login(&token, &name, "", None));
    match set {
        Some(c) => with_cookie(res, &c),
        None => res,
    }
}

async fn login_post(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let csrf_cookie = cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if !auth::csrf_ok(app.secret(), &csrf_cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let email = get_form(&form, "email").trim().to_lowercase();
    let password = get_form(&form, "password");
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let company = t.engine.state().company().map(|c| c.name.clone()).unwrap_or_default();
    let person = t
        .engine
        .state()
        .people
        .iter()
        .find(|p| p.is_enabled() && p.is_user && p.email.as_deref().is_some_and(|e| e.to_lowercase() == email))
        .map(|p| p.id.raw());
    let fail = |status: StatusCode, msg: &str, email: &str| {
        html(status, pages::login(&auth::csrf_token(app.secret(), &csrf_cookie), &company, email, Some(msg)))
    };
    if !inner.limiter.allow(&email) {
        return fail(StatusCode::TOO_MANY_REQUESTS, "Too many attempts. Wait a few minutes and try again.", &email);
    }
    // Always verify something, so a wrong email takes as long as a wrong password.
    let ok = match person {
        Some(p) => inner.creds.check_password(p, password),
        None => inner.creds.check_password(0, password),
    };
    match person {
        Some(p) if ok => {
            inner.limiter.clear(&email);
            let c = session_cookie(&app, &inner, p);
            with_cookie(redirect("/"), &c)
        }
        _ => fail(StatusCode::UNAUTHORIZED, "That email and password don't match.", &email),
    }
}

async fn logout(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let csrf_cookie = cookie(&headers, SESSION_COOKIE).unwrap_or_default();
    if !auth::csrf_ok(app.secret(), &csrf_cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    with_cookie(redirect("/login"), &auth::set_cookie(SESSION_COOKIE, "", 0, app.secure))
}

async fn setup_page(State(app): State<Shared>, headers: HeaderMap) -> Response {
    if app.lock().tenant.is_some() {
        return redirect("/login");
    }
    let (token, set) = anon_csrf(&app, &headers);
    let res = html(StatusCode::OK, pages::setup(&token, &[], None));
    match set {
        Some(c) => with_cookie(res, &c),
        None => res,
    }
}

async fn setup_post(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    if inner.tenant.is_some() {
        return redirect("/login");
    }
    let csrf_cookie = cookie(&headers, CSRF_COOKIE).unwrap_or_default();
    if !auth::csrf_ok(app.secret(), &csrf_cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let token = auth::csrf_token(app.secret(), &csrf_cookie);
    let fail = |msg: String| html(StatusCode::BAD_REQUEST, pages::setup(&token, &form, Some(&msg)));
    let password = get_form(&form, "password");
    if let Err(e) = check_password_rules(password) {
        return fail(e);
    }
    let name = get_form(&form, "name").trim().to_string();
    let slug = text::slug(&name, 63);
    if !buildkiosk_store::valid_slug(&slug) {
        return fail("Enter a company name made of letters or digits.".into());
    }
    let params = json!({
        "name": name,
        "slug": slug,
        "admin_first_name": get_form(&form, "first_name"),
        "admin_last_name": get_form(&form, "last_name"),
        "admin_email": get_form(&form, "email"),
    });
    let mut tenant = match app.data.create(&slug) {
        Ok(t) => t,
        Err(e) => return fail(e.to_string()),
    };
    let out = match tenant.engine.execute(&mut tenant.store, Actor::System, Timestamp::from_millis(now_ms()), "company.setup", &params) {
        Ok(v) => v,
        Err(e) => return fail(e.to_string()),
    };
    let Some(admin) = out.get("admin").and_then(Value::as_u64) else { return fail("The company was created but its administrator was not.".into()) };
    let mut creds = Credentials::default();
    if let Err(e) = creds.set_password(admin, password, Timestamp::from_millis(now_ms())).and_then(|()| tenant.store.save_credentials(&creds)) {
        return fail(e.to_string());
    }
    inner.tenant = Some(tenant);
    inner.creds = creds;
    let c = session_cookie(&app, &inner, admin);
    with_cookie(redirect("/"), &c)
}

/// A company admin sets someone's password (which signs them out everywhere).
async fn set_password(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let back = "/directory";
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let state = t.engine.state();
    if !state.people.get(s.person).is_some_and(|p| p.company_admin) {
        return redirect(&with_flash(back, false, "Only company admins can set passwords."));
    }
    if !state.people.get(Id::new(id)).is_some_and(|p| p.is_enabled() && p.is_user) {
        return redirect(&with_flash(back, false, "That person can't sign in."));
    }
    let password = get_form(&form, "password");
    if let Err(e) = check_password_rules(password) {
        return redirect(&with_flash(back, false, &e));
    }
    let mut creds = inner.creds.clone();
    let saved = creds
        .set_password(id, password, Timestamp::from_millis(now_ms()))
        .and_then(|()| inner.tenant.as_ref().map_or(Ok(()), |t| t.store.save_credentials(&creds)));
    match saved {
        Ok(()) => {
            inner.creds = creds;
            // Keep the admin signed in if they changed their own password.
            let res = redirect(&with_flash(back, true, "Password set."));
            if id == s.person.raw() { with_cookie(res, &session_cookie(&app, &inner, id)) } else { res }
        }
        Err(e) => redirect(&with_flash(back, false, &e.to_string())),
    }
}

// -- account

fn token_rows(inner: &Inner, person: u64) -> Vec<buildkiosk_web::account::TokenRow> {
    inner
        .creds
        .tokens
        .iter()
        .filter(|t| t.person == person)
        .map(|t| buildkiosk_web::account::TokenRow { id: t.id.clone(), name: t.name.clone(), created: t.created })
        .collect()
}

async fn account_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: flash_from(&q) };
    html(StatusCode::OK, buildkiosk_web::account::page(&req, &token_rows(&inner, s.person.raw()), None, &mail_pref(&inner, s.person.raw())))
}

/// Checks the session and the CSRF token of an account form.
fn account_form(app: &Shared, inner: &Inner, headers: &HeaderMap, form: &[(String, String)]) -> Result<Session, Box<Response>> {
    let Some(s) = session(app, inner, headers) else { return Err(Box::new(redirect("/login"))) };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(form, "_csrf")) {
        return Err(Box::new(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.")));
    }
    Ok(s)
}

async fn account_password(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let s = match account_form(&app, &inner, &headers, &form) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let back = "/account";
    let id = s.person.raw();
    if !inner.creds.check_password(id, get_form(&form, "current")) {
        return redirect(&with_flash(back, false, "The current password is not right."));
    }
    let password = get_form(&form, "password");
    if password != get_form(&form, "confirm") {
        return redirect(&with_flash(back, false, "The new password and its repeat are different."));
    }
    if let Err(e) = check_password_rules(password) {
        return redirect(&with_flash(back, false, &e));
    }
    let mut creds = inner.creds.clone();
    let saved = creds
        .set_password(id, password, Timestamp::from_millis(now_ms()))
        .and_then(|()| inner.tenant.as_ref().map_or(Ok(()), |t| t.store.save_credentials(&creds)));
    match saved {
        Ok(()) => {
            inner.creds = creds;
            with_cookie(redirect(&with_flash(back, true, "Password changed.")), &session_cookie(&app, &inner, id))
        }
        Err(e) => redirect(&with_flash(back, false, &e.to_string())),
    }
}

// -- email

fn mail_pref(inner: &Inner, person: u64) -> buildkiosk_web::account::MailPref {
    buildkiosk_web::account::MailPref { configured: inner.creds.smtp.is_some(), on: !inner.creds.email_off.contains(&person) }
}

/// Saves the credential store; on failure leaves the in-memory copy alone.
fn save_creds(inner: &mut Inner, creds: Credentials) -> Result<(), String> {
    if let Some(t) = inner.tenant.as_ref() {
        t.store.save_credentials(&creds).map_err(|e| e.to_string())?;
    }
    inner.creds = creds;
    Ok(())
}

async fn account_email(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let s = match account_form(&app, &inner, &headers, &form) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let mut creds = inner.creds.clone();
    if get_form(&form, "on").is_empty() {
        creds.email_off.insert(s.person.raw());
    } else {
        creds.email_off.remove(&s.person.raw());
    }
    match save_creds(&mut inner, creds) {
        Ok(()) => redirect(&with_flash("/account", true, "Saved.")),
        Err(e) => redirect(&with_flash("/account", false, &e)),
    }
}

/// The signed-in company admin, or the response to send instead.
fn admin_form(app: &Shared, inner: &Inner, headers: &HeaderMap, form: Option<&[(String, String)]>) -> Result<Session, Box<Response>> {
    let Some(s) = session(app, inner, headers) else { return Err(Box::new(redirect("/login"))) };
    if let Some(form) = form
        && !auth::csrf_ok(app.secret(), &s.cookie, get_form(form, "_csrf"))
    {
        return Err(Box::new(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.")));
    }
    let is_admin = inner.tenant.as_ref().is_some_and(|t| t.engine.state().people.get(s.person).is_some_and(|p| p.company_admin));
    if !is_admin {
        return Err(Box::new(redirect(&with_flash("/account", false, "Only company admins can do that."))));
    }
    Ok(s)
}

async fn mail_settings(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let inner = app.lock();
    let s = match admin_form(&app, &inner, &headers, None) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: flash_from(&q) };
    let m = match inner.creds.smtp.as_ref() {
        Some(c) => buildkiosk_web::account::MailForm {
            host: c.host.clone(),
            port: c.port.to_string(),
            security: c.security.clone(),
            username: c.username.clone(),
            has_password: !c.password.is_empty(),
            from: c.from.clone(),
            base_url: c.base_url.clone(),
            configured: true,
        },
        None => buildkiosk_web::account::MailForm { port: "587".into(), ..Default::default() },
    };
    html(StatusCode::OK, buildkiosk_web::account::mail_page(&req, &m))
}

async fn mail_settings_save(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    if let Err(r) = admin_form(&app, &inner, &headers, Some(&form)) {
        return *r;
    }
    let back = "/admin/email";
    let Ok(port) = get_form(&form, "port").trim().parse::<u16>() else {
        return redirect(&with_flash(back, false, "The port must be a number such as 587."));
    };
    let old = inner.creds.smtp.clone();
    let password = match get_form(&form, "password") {
        "" => old.as_ref().map(|o| o.password.clone()).unwrap_or_default(),
        p => p.to_string(),
    };
    let new = mail::check(buildkiosk_store::creds::SmtpSettings {
        host: get_form(&form, "host").to_string(),
        port,
        security: get_form(&form, "security").to_string(),
        username: get_form(&form, "username").trim().to_string(),
        password,
        from: get_form(&form, "from").to_string(),
        base_url: get_form(&form, "base_url").to_string(),
    });
    let new = match new {
        Ok(n) => n,
        Err(e) => return redirect(&with_flash(back, false, &e)),
    };
    let mut creds = inner.creds.clone();
    if old.is_none() {
        // Don't send the backlog the first time email is turned on.
        creds.mail_mark = inner.tenant.as_ref().and_then(|t| t.engine.state().notifications.iter().map(|n| n.id.raw()).max()).unwrap_or(0);
    }
    creds.smtp = Some(new);
    match save_creds(&mut inner, creds) {
        Ok(()) => redirect(&with_flash(back, true, "Saved. Send a test email to check it.")),
        Err(e) => redirect(&with_flash(back, false, &e)),
    }
}

async fn mail_clear(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    if let Err(r) = admin_form(&app, &inner, &headers, Some(&form)) {
        return *r;
    }
    let mut creds = inner.creds.clone();
    creds.smtp = None;
    match save_creds(&mut inner, creds) {
        Ok(()) => redirect(&with_flash("/admin/email", true, "Email is turned off.")),
        Err(e) => redirect(&with_flash("/admin/email", false, &e)),
    }
}

async fn mail_test(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let (settings, to) = {
        let inner = app.lock();
        let s = match admin_form(&app, &inner, &headers, Some(&form)) {
            Ok(s) => s,
            Err(r) => return *r,
        };
        let to = inner.tenant.as_ref().and_then(|t| t.engine.state().people.get(s.person).and_then(|p| p.email.clone()));
        (inner.creds.smtp.clone(), to)
    };
    let back = "/admin/email";
    let (Some(settings), Some(to)) = (settings, to) else {
        return redirect(&with_flash(back, false, "Save the settings first, and give your own account an email address."));
    };
    let m = mail::Mail {
        id: 0,
        to: to.clone(),
        subject: "BuildKiosk test email".into(),
        body: "This is a test message from BuildKiosk. Email is working.".into(),
    };
    let sent = tokio::task::spawn_blocking(move || mail::send(&settings, &m)).await.unwrap_or_else(|e| Err(e.to_string()));
    match sent {
        Ok(()) => redirect(&with_flash(back, true, &format!("A test email was sent to {to}."))),
        Err(e) => redirect(&with_flash(back, false, &format!("The email could not be sent: {e}"))),
    }
}

/// Notifications that still need emailing, and the id to mark as done once they are sent.
fn mail_pending(app: &App) -> Option<(buildkiosk_store::creds::SmtpSettings, Vec<mail::Mail>, u64)> {
    let inner = app.lock();
    let smtp = inner.creds.smtp.clone()?;
    let state = inner.tenant.as_ref()?.engine.state();
    let mut news: Vec<_> = state.notifications.iter().filter(|n| n.id.raw() > inner.creds.mail_mark).collect();
    news.sort_by_key(|n| n.id.raw());
    news.truncate(100);
    let mark = news.last()?.id.raw();
    let mails = news
        .iter()
        .filter(|n| !n.read && !inner.creds.email_off.contains(&n.person.raw()))
        .filter_map(|n| {
            let p = state.people.get(n.person)?;
            let to = p.email.clone().filter(|_| p.is_user && p.active)?;
            let subject: String = n.text.chars().filter(|c| !c.is_control()).take(120).collect();
            let link = if n.link.is_empty() || smtp.base_url.is_empty() { String::new() } else { format!("\n\n{}{}", smtp.base_url, n.link) };
            Some(mail::Mail {
                id: n.id.raw(),
                to,
                subject,
                body: format!(
                    "{}{link}\n\nYou get this because email notifications are on for your BuildKiosk account. Turn them off under My account.\n",
                    n.text
                ),
            })
        })
        .collect();
    Some((smtp, mails, mark))
}

/// Emails the notifications made since the last pass. Returns how many were sent.
fn mail_pass(app: &App) -> usize {
    let Some((smtp, mails, mark)) = mail_pending(app) else { return 0 };
    let mut sent = 0;
    for m in &mails {
        match mail::send(&smtp, m) {
            Ok(()) => sent += 1,
            Err(e) => {
                // Stop at the first failure and try again on the next pass: nothing is lost.
                log::warn!("could not send a notification email: {e}");
                let done = m.id.saturating_sub(1);
                let mut inner = app.lock();
                if done > inner.creds.mail_mark {
                    let mut creds = inner.creds.clone();
                    creds.mail_mark = done;
                    if let Err(e) = save_creds(&mut inner, creds) {
                        log::warn!("could not record sent emails: {e}");
                    }
                }
                return sent;
            }
        }
    }
    let mut inner = app.lock();
    let mut creds = inner.creds.clone();
    creds.mail_mark = mark;
    if let Err(e) = save_creds(&mut inner, creds) {
        log::warn!("could not record sent emails: {e}");
    }
    sent
}

/// The scheduled backups: checks every five minutes whether one is due.
async fn backup_loop(app: Shared) {
    tokio::time::sleep(Duration::from_secs(60)).await;
    loop {
        let a = app.clone();
        match tokio::task::spawn_blocking(move || backup_pass(&a, u64::try_from(now_ms() / 1000).unwrap_or(0))).await {
            Ok(Ok(Some(dir))) => log::info!("backup written to {}", dir.display()),
            Ok(Ok(None)) => {}
            Ok(Err(e)) => log::error!("the scheduled backup failed: {e}"),
            Err(e) => log::error!("the scheduled backup stopped: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(300)).await;
    }
}

/// The unix seconds in a scheduled backup's folder name (`backup-1760000000`).
fn backup_time(name: &str) -> Option<u64> {
    name.strip_prefix("backup-").filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).and_then(|n| n.parse().ok())
}

/// The finished backups in a folder (`backup-<seconds>`), oldest first; unfinished ones (`.partial`) are removed.
fn finished_backups(dir: &FsPath) -> Result<Vec<(u64, PathBuf)>, String> {
    let mut done: Vec<(u64, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(stem) = name.strip_suffix(".partial").filter(|s| backup_time(s).is_some()) {
            // A backup that was interrupted: not a backup.
            log::warn!("removing the unfinished backup {stem}.partial");
            let _ = std::fs::remove_dir_all(entry.path());
        } else if let Some(t) = backup_time(&name) {
            done.push((t, entry.path()));
        }
    }
    done.sort();
    Ok(done)
}

/// Drops the oldest backups beyond how many to keep. Only folders this scheduler named are ever touched.
fn prune_backups(done: &mut Vec<(u64, PathBuf)>, keep: usize) {
    while done.len() > keep {
        let (_, oldest) = done.remove(0);
        if let Err(e) = std::fs::remove_dir_all(&oldest) {
            log::warn!("cannot remove the old backup {}: {e}", oldest.display());
        }
    }
}

/// Takes a backup now into the plan's folder (and its second copy), then prunes both. Returns the new folder.
fn take_backup(app: &App, plan: &BackupPlan, now_secs: u64) -> Result<Option<PathBuf>, String> {
    let name = format!("backup-{now_secs}");
    let partial = plan.dir.join(format!("{name}.partial"));
    let kept = plan.dir.join(&name);
    if kept.exists() {
        return Ok(None);
    }
    if let Err(e) = app.backup_now(&partial) {
        let _ = std::fs::remove_dir_all(&partial);
        return Err(e);
    }
    std::fs::rename(&partial, &kept).map_err(|e| format!("cannot finish the backup: {e}"))?;
    let mut done = finished_backups(&plan.dir)?;
    prune_backups(&mut done, plan.keep);
    if let Some(second) = &plan.copy_to {
        // The second copy is best effort: the backup itself is safe, so a full or missing disk is reported, not fatal.
        let copied = (|| -> Result<(), String> {
            let target_partial = second.join(format!("{name}.partial"));
            let _ = std::fs::remove_dir_all(&target_partial);
            buildkiosk_store::ops::copy_backup(&kept, &target_partial).map_err(|e| e.to_string())?;
            std::fs::rename(&target_partial, second.join(&name)).map_err(|e| e.to_string())?;
            let mut there = finished_backups(second)?;
            prune_backups(&mut there, plan.keep);
            Ok(())
        })();
        if let Err(e) = copied {
            log::error!("the backup {name} was made but could not be copied to {}: {e}", second.display());
        }
    }
    Ok(Some(kept))
}

/// Takes a backup if the newest one is older than the interval, then drops the oldest beyond how many
/// to keep. Only folders this scheduler named (`backup-<seconds>` and its `.partial`) are ever touched.
fn backup_pass(app: &App, now_secs: u64) -> Result<Option<PathBuf>, String> {
    let Some(plan) = &app.backups else { return Ok(None) };
    let mut done = finished_backups(&plan.dir)?;
    let due = done.last().is_none_or(|(t, _)| now_secs.saturating_sub(*t) >= plan.every_secs);
    if due {
        return take_backup(app, plan, now_secs);
    }
    prune_backups(&mut done, plan.keep);
    Ok(None)
}

/// The size of a folder and everything in it (it stops counting past 100,000 files).
fn folder_bytes(dir: &FsPath) -> u64 {
    let (mut total, mut seen) = (0u64, 0usize);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            seen += 1;
            if seen > 100_000 {
                return total;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(e.path()),
                Ok(t) if t.is_file() => total = total.saturating_add(e.metadata().map(|m| m.len()).unwrap_or(0)),
                _ => {}
            }
        }
    }
    total
}

async fn backups_page(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    let inner = app.lock();
    let s = match admin_form(&app, &inner, &headers, None) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: flash_from(&q) };
    html(StatusCode::OK, buildkiosk_web::account::backups_page(&req, &backups_info(&app)))
}

fn backups_info(app: &App) -> buildkiosk_web::account::BackupInfo {
    use buildkiosk_web::account::{BackupInfo, BackupRow};
    let Some(plan) = &app.backups else { return BackupInfo::default() };
    let second: Vec<u64> =
        plan.copy_to.as_deref().and_then(|d| finished_backups(d).ok()).map(|v| v.into_iter().map(|(t, _)| t).collect()).unwrap_or_default();
    let mut rows: Vec<BackupRow> = finished_backups(&plan.dir)
        .unwrap_or_default()
        .into_iter()
        .map(|(t, p)| BackupRow {
            name: format!("backup-{t}"),
            taken: i64::try_from(t).unwrap_or(0),
            bytes: folder_bytes(&p),
            copied: second.contains(&t),
        })
        .collect();
    rows.reverse();
    BackupInfo {
        configured: true,
        dir: plan.dir.display().to_string(),
        copy_to: plan.copy_to.as_ref().map(|d| d.display().to_string()),
        every_hours: plan.every_secs / 3600,
        keep: plan.keep,
        rows,
    }
}

/// Takes a backup now, on a company admin's request.
async fn backups_run(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    {
        let inner = app.lock();
        if let Err(r) = admin_form(&app, &inner, &headers, Some(&form)) {
            return *r;
        }
    }
    let a = app.clone();
    let made = tokio::task::spawn_blocking(move || {
        let Some(plan) = &a.backups else { return Err("backups are not turned on; start the server with --backup-dir".to_string()) };
        take_backup(&a, plan, u64::try_from(now_ms() / 1000).unwrap_or(0))
    })
    .await;
    match made {
        Ok(Ok(Some(dir))) => {
            redirect(&with_flash("/admin/backups", true, &format!("Backup {} written.", dir.file_name().and_then(|n| n.to_str()).unwrap_or(""))))
        }
        Ok(Ok(None)) => redirect(&with_flash("/admin/backups", false, "A backup was already made this second; try again in a moment.")),
        Ok(Err(e)) => redirect(&with_flash("/admin/backups", false, &e)),
        Err(_) => redirect(&with_flash("/admin/backups", false, "The backup stopped unexpectedly.")),
    }
}

async fn mail_loop(app: Shared) {
    loop {
        tokio::time::sleep(Duration::from_secs(20)).await;
        let a = app.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || mail_pass(&a)).await {
            log::warn!("the email sender stopped a pass: {e}");
        }
    }
}

async fn account_token(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let s = match account_form(&app, &inner, &headers, &form) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let name = get_form(&form, "name").trim().to_string();
    if name.is_empty() {
        return redirect(&with_flash("/account", false, "Give the token a name."));
    }
    let mut creds = inner.creds.clone();
    let made = creds
        .create_token(s.person.raw(), &name, Timestamp::from_millis(now_ms()))
        .and_then(|t| inner.tenant.as_ref().map_or(Ok(()), |tn| tn.store.save_credentials(&creds)).map(|()| t));
    match made {
        Ok(token) => {
            inner.creds = creds;
            let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
            let csrf = auth::csrf_token(app.secret(), &s.cookie);
            let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
            let mut res = html(
                StatusCode::OK,
                buildkiosk_web::account::page(&req, &token_rows(&inner, s.person.raw()), Some(&token), &mail_pref(&inner, s.person.raw())),
            );
            // The page holds a secret: keep it out of caches and history.
            res.headers_mut().insert("cache-control", HeaderValue::from_static("no-store"));
            res
        }
        Err(e) => redirect(&with_flash("/account", false, &e.to_string())),
    }
}

async fn account_token_revoke(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let s = match account_form(&app, &inner, &headers, &form) {
        Ok(s) => s,
        Err(r) => return *r,
    };
    let id = get_form(&form, "id");
    // Only the person's own tokens can be revoked here.
    if !inner.creds.tokens.iter().any(|t| t.id == id && t.person == s.person.raw()) {
        return redirect(&with_flash("/account", false, "That token is not yours."));
    }
    let mut creds = inner.creds.clone();
    creds.revoke_token(id);
    match inner.tenant.as_ref().map_or(Ok(()), |t| t.store.save_credentials(&creds)) {
        Ok(()) => {
            inner.creds = creds;
            redirect(&with_flash("/account", true, "Token revoked."))
        }
        Err(e) => redirect(&with_flash("/account", false, &e.to_string())),
    }
}

// -- commands

/// Turns the posted fields into command parameters: a field named `x[]` collects all its values
/// into a list (blank ones dropped); any other field takes its last value. Fields starting with
/// `_` belong to the form machinery.
/// The `action` of the save form on an edit or new page (`…/edit` and `…/new` post to `…/save`).
fn save_action(back: &str) -> String {
    match back.strip_suffix("/edit").or_else(|| back.strip_suffix("/new")) {
        Some(base) => format!("{base}/save"),
        None => back.to_string(),
    }
}

fn form_params(form: &[(String, String)]) -> Value {
    let mut map = Map::new();
    // `items.0.description` style fields become `items: [{description}]`, in row order.
    let mut lists: std::collections::BTreeMap<String, std::collections::BTreeMap<usize, Map<String, Value>>> = std::collections::BTreeMap::new();
    let mut objects: std::collections::BTreeMap<&str, Map<String, Value>> = std::collections::BTreeMap::new();
    for (k, v) in form {
        if k.starts_with('_') {
            continue;
        }
        if let Some((object, key)) = ["answers", "levels", "overrides"].iter().find_map(|o| k.strip_prefix(&format!("{o}.")).map(|rest| (*o, rest))) {
            // `answers.f1` and `levels.rfis` style fields become `answers: {f1: ...}` and `levels: {rfis: ...}`.
            let entry = objects.entry(object).or_default();
            if entry.len() < 200 {
                entry.insert(key.to_string(), Value::String(v.clone()));
            }
            continue;
        }
        if let Some((list, rest)) = k.split_once('.')
            && let Some((index, field)) = rest.split_once('.')
            && let Ok(index) = index.parse::<usize>()
        {
            if index < 1000 && lists.len() < 20 {
                lists.entry(list.to_string()).or_default().entry(index).or_default().insert(field.to_string(), Value::String(v.clone()));
            }
            continue;
        }
        match k.strip_suffix("[]") {
            Some(name) => {
                let entry = map.entry(name.to_string()).or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(items) = entry
                    && !v.trim().is_empty()
                    && items.len() < 1000
                {
                    items.push(Value::String(v.clone()));
                }
            }
            None => {
                map.insert(k.clone(), Value::String(v.clone()));
            }
        }
    }
    for (name, fields) in objects {
        map.insert(name.into(), Value::Object(fields));
    }
    for (list, rows) in lists {
        map.insert(list, Value::Array(rows.into_values().map(Value::Object).collect()));
    }
    Value::Object(map)
}

async fn do_command(State(app): State<Shared>, headers: HeaderMap, Path(command): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, get_form(&form, "_csrf")) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let next = local_path(get_form(&form, "_next")).to_string();
    let back = local_path(match get_form(&form, "_back") {
        "" => get_form(&form, "_next"),
        b => b,
    })
    .to_string();
    let Some(spec) = find_command(&command) else { return redirect(&with_flash(&back, false, &format!("Unknown command `{command}`."))) };
    match inner.run(Actor::Person(s.person), &command, &form_params(&form)) {
        Ok(result) => {
            let id = result.get("id").and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_default();
            redirect(&with_flash(&next.replace("{id}", &id), true, &format!("{}: done.", spec.label)))
        }
        Err(e) => {
            let back = inner.draft_path(s.person.raw(), &format!("/do/{command}"), &form, &back);
            redirect(&with_flash(&back, false, &e.to_string()))
        }
    }
}

fn api_error(e: &EngineError) -> Response {
    let status = StatusCode::from_u16(e.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    json_response(status, &json!({"ok": false, "error": {"kind": e.kind(), "message": e.to_string()}}))
}

fn json_response(status: StatusCode, v: &Value) -> Response {
    let mut res = (status, v.to_string()).into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    res
}

/// The MCP server over HTTP: one JSON-RPC message (or batch) per POST, with an API token. A
/// notification answers 202 with no body.
async fn mcp_http(State(app): State<Shared>, headers: HeaderMap, body: Bytes) -> Response {
    let token = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let mut inner = app.lock();
    let person = inner
        .creds
        .token_person(token)
        .filter(|p| inner.tenant.as_ref().and_then(|t| t.engine.state().people.get(Id::new(*p))).is_some_and(|p| p.is_enabled() && p.is_user));
    let Some(person) = person else {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32001, "message": "send a valid API token as `Authorization: Bearer <token>`"}}),
        );
    };
    let Some(tenant) = inner.tenant.as_mut() else { return plain(StatusCode::SERVICE_UNAVAILABLE, "The company is not set up.") };
    let text = String::from_utf8_lossy(&body).into_owned();
    match buildkiosk_mcp::handle_line(tenant, Actor::Person(Id::new(person)), Timestamp::from_millis(now_ms()), &text) {
        Some(reply) => {
            let mut res = reply.into_response();
            res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            res
        }
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Builds a PDF for the signed-in person with `build` and sends it as a download.
fn pdf_for(app: &Shared, headers: &HeaderMap, build: impl FnOnce(&Req<'_>) -> Option<(String, Vec<u8>)>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(app, &inner, headers) else { return redirect("/login") };
    let req = Req { engine: &t.engine, person: s.person, csrf: "", now: Timestamp::from_millis(now_ms()), flash: None };
    let Some((name, bytes)) = build(&req) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/pdf"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn api_command(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<String>, body: Bytes) -> Response {
    let token = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).unwrap_or("");
    let mut inner = app.lock();
    let person = inner
        .creds
        .token_person(token)
        .filter(|p| inner.tenant.as_ref().and_then(|t| t.engine.state().people.get(Id::new(*p))).is_some_and(|p| p.is_enabled() && p.is_user));
    let Some(person) = person else {
        return json_response(
            StatusCode::UNAUTHORIZED,
            &json!({"ok": false, "error": {"kind": "unauthorized", "message": "send a valid API token as `Authorization: Bearer <token>`"}}),
        );
    };
    let params: Value = if body.iter().all(u8::is_ascii_whitespace) {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return json_response(StatusCode::BAD_REQUEST, &json!({"ok": false, "error": {"kind": "bad_json", "message": e.to_string()}})),
        }
    };
    match inner.run(Actor::Person(Id::new(person)), &id, &params) {
        Ok(result) => json_response(StatusCode::OK, &json!({"ok": true, "result": result})),
        Err(e) => api_error(&e),
    }
}

async fn rfi_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = pages::rfi_csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let mut res = csv.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/csv; charset=utf-8"));
    res.headers_mut().insert("content-disposition", HeaderValue::from_static("attachment; filename=\"rfis.csv\""));
    res
}

/// A file name that is safe inside a `Content-Disposition` header.
fn safe_filename(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' { c } else { '_' }).take(100).collect();
    if s.is_empty() { "file".into() } else { s }
}

/// A tool whose items carry files: its attach command, its page path and how to find an item's
/// files.
#[derive(Clone, Copy)]
struct Kind {
    attach: &'static str,
    path: &'static str,
    files: fn(&CompanyState, Actor, u64, u64) -> Option<Vec<FileRef>>,
}

impl Kind {
    const RFI: Kind = Kind { attach: "rfi.attach", path: "rfis", files: rfi_files };
    const SUBMITTAL: Kind = Kind { attach: "submittal.attach", path: "submittals", files: submittal_files };
    const INSPECTION: Kind = Kind { attach: "inspection.attach", path: "inspections", files: inspection_files };
    const CORRESPONDENCE: Kind = Kind { attach: "correspondence.attach", path: "correspondence", files: correspondence_files };
    const DAILY_LOG: Kind = Kind { attach: "daily_log.attach", path: "daily-logs", files: daily_log_files };
    const FORM: Kind = Kind { attach: "form.attach", path: "forms", files: form_files };
    const MEETING: Kind = Kind { attach: "meeting.attach", path: "meetings", files: meeting_files };
    const TRANSMITTAL: Kind = Kind { attach: "transmittal.attach", path: "transmittals", files: transmittal_files };
    const COMMITMENT: Kind = Kind { attach: "commitment.attach", path: "commitments", files: commitment_files };
    const DRAWING: Kind = Kind { attach: "drawing.add_revision", path: "drawings", files: drawing_files };
    const DOCUMENT: Kind = Kind { attach: "document.add_version", path: "documents", files: document_files };

    fn work<T: WorkItem>() -> Kind {
        Kind { attach: T::ATTACH_CMD, path: T::PATH, files: work_files::<T> }
    }

    fn attach_command(self) -> &'static str {
        self.attach
    }

    fn path(self) -> &'static str {
        self.path
    }

    /// The files of an item the person may see.
    fn files(self, state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
        (self.files)(state, actor, project, item)
    }
}

fn correspondence_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .correspondence
        .get(Id::new(item))
        .filter(|c| c.project == Id::new(project) && buildkiosk_engine::cmd::correspondence::can_see(state, actor, c))
        .map(|c| c.attachments.clone())
}

fn form_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .form_records
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && r.deleted.is_none() && view::level(state, actor, r.project, Tool::Forms) >= Level::Read)
        .map(|r| r.attachments.clone())
}

fn meeting_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .meetings
        .get(Id::new(item))
        .filter(|m| m.project == Id::new(project) && m.deleted.is_none() && view::level(state, actor, m.project, Tool::Meetings) >= Level::Read)
        .map(|m| m.attachments.clone())
}

fn transmittal_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .transmittals
        .get(Id::new(item))
        .filter(|t| t.project == Id::new(project) && buildkiosk_engine::cmd::transmittal::can_see(state, actor, t))
        .map(|t| t.attachments.clone())
}

fn commitment_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .commitments
        .get(Id::new(item))
        .filter(|c| c.project == Id::new(project) && view::can_see_commitment(state, actor, c))
        .map(|c| c.attachments.clone())
}

fn drawing_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .drawings
        .get(Id::new(item))
        .filter(|d| d.project == Id::new(project) && d.deleted.is_none() && view::level(state, actor, d.project, Tool::Drawings) >= Level::Read)
        .map(|d| d.revisions.iter().filter_map(|r| r.file.clone()).collect())
}

fn document_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .documents
        .get(Id::new(item))
        .filter(|d| d.project == Id::new(project) && d.deleted.is_none() && view::can_see_document(state, actor, d))
        .map(|d| d.versions.iter().map(|v| v.file.clone()).collect())
}

fn rfi_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .rfis
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_rfi(state, actor, r))
        .map(|r| r.attachments.iter().chain(r.responses.iter().flat_map(|v| v.attachments.iter())).cloned().collect())
}

/// A submittal's files include those on its reviews.
fn submittal_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .submittals
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_submittal(state, actor, r))
        .map(|r| r.attachments.iter().chain(r.reviews.iter().flat_map(|v| v.attachments.iter())).cloned().collect())
}

fn daily_log_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .daily_logs
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_daily_log(state, actor, r))
        .map(|r| r.attachments.clone())
}

fn inspection_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .inspections
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_inspection(state, actor, r))
        .map(|r| r.attachments.iter().chain(r.items.iter().flat_map(|l| l.attachments.iter())).cloned().collect())
}

fn work_files<T: WorkItem>(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    T::table(state)
        .get(Id::new(item))
        .filter(|r| r.project() == Id::new(project) && workitem::can_see(state, actor, *r))
        .map(|r| r.attachments.clone())
}

async fn rfi_file(State(app): State<Shared>, headers: HeaderMap, Path((id, rid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::RFI, id, rid, sha).await
}

async fn submittal_file(State(app): State<Shared>, headers: HeaderMap, Path((id, sid, sha)): Path<(u64, u64, String)>) -> Response {
    file_download(app, headers, Kind::SUBMITTAL, id, sid, sha).await
}

async fn file_download(app: Shared, headers: HeaderMap, kind: Kind, id: u64, rid: u64, sha: String) -> Response {
    let found = {
        let inner = app.lock();
        let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
        let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
        let files = kind.files(t.engine.state(), Actor::Person(s.person), id, rid);
        files.and_then(|f| f.into_iter().find(|a| a.sha256 == sha)).and_then(|a| t.store.blobs.path(&a.sha256).map(|p| (p, a.name.clone())))
    };
    let Some((path, name)) = found else { return plain(StatusCode::NOT_FOUND, "Not found") };
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        _ => return plain(StatusCode::NOT_FOUND, "That file is missing from storage."),
    };
    let mut res = bytes.into_response();
    // Uploaded files are never rendered by the browser: always a download, never sniffed.
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    res
}

async fn rfi_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::RFI, id, rid, multipart).await
}

async fn submittal_upload(State(app): State<Shared>, headers: HeaderMap, Path((id, sid)): Path<(u64, u64)>, multipart: Multipart) -> Response {
    file_upload(app, headers, Kind::SUBMITTAL, id, sid, multipart).await
}

async fn submittal_review(State(app): State<Shared>, headers: HeaderMap, Path((id, sid)): Path<(u64, u64)>, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/submittals/{sid}");
    let (mut token, mut decision, mut comment) = (String::new(), String::new(), String::new());
    let mut files: Vec<(String, String, Bytes)> = Vec::new();
    let mut total = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, "The upload failed or the files are larger than 25 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else {
            return redirect(&with_flash(&back, false, "The upload failed or the files are larger than 25 MB."));
        };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "decision" => decision = String::from_utf8_lossy(&bytes).into_owned(),
            "comment" => comment = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if !bytes.is_empty() => {
                total = total.saturating_add(bytes.len());
                if total > MAX_UPLOAD || files.len() >= 10 {
                    return redirect(&with_flash(&back, false, "A review can carry at most 10 files and 25 MB in all."));
                }
                files.push((filename, mime, bytes));
            }
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let mut attachments = Vec::new();
    for (name, mime, bytes) in &files {
        match t.store.blobs.put(bytes) {
            Ok((sha256, size)) => attachments.push(json!({"sha256": sha256, "name": name, "size": size, "mime": mime})),
            Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
        }
    }
    let params = json!({"project": id, "id": sid, "decision": decision, "comment": comment, "attachments": attachments});
    match inner.run(Actor::Person(s.person), "submittal.review", &params) {
        Ok(_) => redirect(&with_flash(&back, true, "Review submitted.")),
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

/// Posts an RFI response with up to 10 files (a multipart form).
async fn rfi_respond(State(app): State<Shared>, headers: HeaderMap, Path((id, rid)): Path<(u64, u64)>, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/rfis/{rid}");
    let too_big = "The upload failed or the files are larger than 25 MB.";
    let (mut token, mut body) = (String::new(), String::new());
    let mut files: Vec<(String, String, Bytes)> = Vec::new();
    let mut total = 0usize;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, too_big)),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else { return redirect(&with_flash(&back, false, too_big)) };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "body" => body = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if !bytes.is_empty() => {
                total = total.saturating_add(bytes.len());
                if total > MAX_UPLOAD || files.len() >= 10 {
                    return redirect(&with_flash(&back, false, "A response can carry at most 10 files and 25 MB in all."));
                }
                files.push((filename, mime, bytes));
            }
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let mut attachments = Vec::new();
    for (name, mime, bytes) in &files {
        match t.store.blobs.put(bytes) {
            Ok((sha256, size)) => attachments.push(json!({"sha256": sha256, "name": name, "size": size, "mime": mime})),
            Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
        }
    }
    let params = json!({"project": id, "id": rid, "body": body, "attachments": attachments});
    match inner.run(Actor::Person(s.person), "rfi.respond", &params) {
        Ok(_) => redirect(&with_flash(&back, true, "Response posted.")),
        Err(e) => {
            // Keep what was typed (files can't be kept; the page says so).
            let form = [("body".to_string(), body)];
            let to = inner.draft_path(s.person.raw(), &format!("{back}/respond"), &form, &back);
            redirect(&with_flash(&to, false, &e.to_string()))
        }
    }
}

async fn file_upload(app: Shared, headers: HeaderMap, kind: Kind, id: u64, rid: u64, mut multipart: Multipart) -> Response {
    let back = format!("/projects/{id}/{}/{rid}", kind.path());
    let mut token = String::new();
    let mut file: Option<(String, String, Bytes)> = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else {
            return redirect(&with_flash(&back, false, "The upload failed or the file is larger than 25 MB."));
        };
        match name.as_str() {
            "_csrf" => token = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if file.is_none() => file = Some((filename, mime, bytes)),
            _ => {}
        }
    }
    let mut inner = app.lock();
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    if !auth::csrf_ok(app.secret(), &s.cookie, &token) {
        return plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again.");
    }
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return redirect(&with_flash(&back, false, "Choose a file to attach."));
    };
    if bytes.len() > MAX_UPLOAD {
        return redirect(&with_flash(&back, false, "Files can be at most 25 MB."));
    }
    let Some(t) = inner.tenant.as_mut() else { return redirect("/setup") };
    let (sha256, size) = match t.store.blobs.put(&bytes) {
        Ok(x) => x,
        Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
    };
    let params = json!({"project": id, "id": rid, "sha256": sha256, "name": name, "size": size, "mime": mime});
    match inner.run(Actor::Person(s.person), kind.attach_command(), &params) {
        Ok(_) => redirect(&with_flash(&back, true, "File attached.")),
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

#[cfg(test)]
mod tests;
