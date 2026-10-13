//! The HTTP server (layer 6): serves the web pages, handles sign-in with signed session cookies,
//! checks a CSRF token on every form, turns form posts into commands (`POST /do/{command}`) and
//! exposes the same commands as JSON (`POST /api/v1/commands/{id}`, bearer token).
#![forbid(unsafe_code)]

mod auth;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

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
use buildkiosk_engine::model::observation::Observation;
use buildkiosk_engine::model::punch::PunchItem;
use buildkiosk_engine::model::{CompanyState, FileRef};
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
/// Largest file upload.
const MAX_UPLOAD: usize = 25 * 1024 * 1024;

struct Inner {
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
        Ok(App { data, inner: Mutex::new(Inner { tenant, creds, limiter: RateLimit::default() }), secure: false })
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
        .route("/directory/people/{id}", get(person_edit))
        .route("/people/{id}/password", post(set_password))
        .route("/projects/{id}", get(project_home))
        .route("/projects/{id}/team", get(team))
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
        .route("/projects/{id}/commitments/{cid}/save", post(commitment_save))
        .route("/projects/{id}/inspections", get(inspection_list))
        .route("/projects/{id}/inspections.csv", get(inspection_csv))
        .route("/projects/{id}/inspections/new", get(inspection_new))
        .route("/projects/{id}/inspections/templates", get(inspection_templates))
        .route("/projects/{id}/inspections/{iid}", get(inspection_detail))
        .route("/projects/{id}/inspections/{iid}/edit", get(inspection_edit))
        .route("/projects/{id}/inspections/{iid}/answers", post(inspection_answers))
        .route("/projects/{id}/inspections/{iid}/files/{sha}", get(inspection_file))
        .route("/projects/{id}/inspections/{iid}/upload", post(inspection_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/daily-logs", get(daily_log_list))
        .route("/projects/{id}/daily-logs/new", get(daily_log_new))
        .route("/projects/{id}/daily-logs/save", post(daily_log_save_new))
        .route("/projects/{id}/daily-logs/{lid}", get(daily_log_detail))
        .route("/projects/{id}/daily-logs/{lid}/edit", get(daily_log_edit))
        .route("/projects/{id}/daily-logs/{lid}/save", post(daily_log_save))
        .route("/projects/{id}/specifications", get(specifications))
        .route("/projects/{id}/submittals.csv", get(submittal_csv))
        .route("/projects/{id}/submittals/new", get(submittal_new))
        .route("/projects/{id}/submittals/{sid}/review", post(submittal_review).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/submittals/{sid}", get(submittal_detail))
        .route("/projects/{id}/submittals/{sid}/edit", get(submittal_edit))
        .route("/projects/{id}/submittals/{sid}/files/{sha}", get(submittal_file))
        .route("/projects/{id}/submittals/{sid}/upload", post(submittal_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/rfis.csv", get(rfi_csv))
        .route("/projects/{id}/rfis/{rid}", get(rfi_detail))
        .route("/projects/{id}/rfis/{rid}/files/{sha}", get(rfi_file))
        .route("/projects/{id}/rfis/{rid}/upload", post(rfi_upload).layer(DefaultBodyLimit::max(MAX_UPLOAD + 64 * 1024)))
        .route("/projects/{id}/rfis/{rid}/edit", get(rfi_edit))
        .route("/do/{command}", post(do_command))
        .route("/api/v1/commands/{id}", post(api_command))
        .route("/static/app.css", get(css))
        .route("/static/favicon.svg", get(favicon));
    let router = work_routes::<PunchItem>(router);
    let router = work_routes::<Observation>(router);
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
    axum::serve(listener, router(app)).await.map_err(|e| e.to_string())
}

async fn security_headers(State(app): State<Shared>, mut res: Response) -> Response {
    let h = res.headers_mut();
    let mut put = |k: &'static str, v: &'static str| {
        h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
    };
    put(
        "content-security-policy",
        "default-src 'none'; script-src 'none'; style-src 'self'; img-src 'self'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
    );
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
    html(StatusCode::OK, render(&req, t))
}

type Q = Query<HashMap<String, String>>;

async fn home(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::home(r))
}

async fn directory(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| pages::directory(r))
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
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::form(r, Id::new(id), None))
}

async fn daily_log_detail(State(app): State<Shared>, headers: HeaderMap, Path((id, lid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::detail(r, Id::new(id), Id::new(lid)))
}

async fn daily_log_edit(State(app): State<Shared>, headers: HeaderMap, Path((id, lid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::daily_logs::form(r, Id::new(id), Some(Id::new(lid))))
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
        Err(e) => return redirect(&with_flash(&back, false, &e.to_string())),
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

async fn work_list<T: WorkPage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    use buildkiosk_engine::model::Choice;
    let filter = workitems::Filter {
        status: q.get("status").and_then(|s| buildkiosk_engine::model::work::WorkStatus::parse(s)),
        q: q.get("q").map(|s| s.chars().take(100).collect()).unwrap_or_default(),
        mine: q.get("mine").is_some_and(|m| m == "1"),
    };
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
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
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
        return match inner.run(actor, "inspection.complete", &json!({"project": id, "id": iid})) {
            Ok(_) => redirect(&with_flash(&back, true, "Inspection completed.")),
            Err(e) => redirect(&with_flash(&back, false, &format!("Answers saved, but not completed: {e}"))),
        };
    }
    redirect(&with_flash(&back, true, "Answers saved."))
}

async fn budget_page(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::finance::budget(r, Id::new(id)))
}

async fn budget_csv(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>) -> Response {
    let inner = app.lock();
    let Some(t) = inner.tenant.as_ref() else { return redirect("/setup") };
    let Some(s) = session(&app, &inner, &headers) else { return redirect("/login") };
    let csrf = auth::csrf_token(app.secret(), &s.cookie);
    let req = Req { engine: &t.engine, person: s.person, csrf: &csrf, now: Timestamp::from_millis(now_ms()), flash: None };
    let Some(csv) = buildkiosk_web::finance::budget_csv(&req, Id::new(id)) else { return plain(StatusCode::NOT_FOUND, "Not found") };
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
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

async fn invoice_list<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| invoicing::list::<T>(r, Id::new(id)))
}

async fn invoice_detail<T: InvoicePage>(State(app): State<Shared>, headers: HeaderMap, Path((id, iid)): Path<(u64, u64)>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, t| invoicing::detail::<T>(r, Id::new(id), Id::new(iid), &t.store.history(T::TABLE, iid).unwrap_or_default()))
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
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
    }
}

/// Registers the pages of an invoice kind under `/projects/{id}/<path>`.
fn invoice_routes<T: InvoicePage>(router: Router<Shared>) -> Router<Shared> {
    let base = format!("/projects/{{id}}/{}", T::PATH);
    router
        .route(&base, get(invoice_list::<T>))
        .route(&format!("{base}/{{iid}}"), get(invoice_detail::<T>))
        .route(&format!("{base}/{{iid}}/save"), post(invoice_save::<T>))
}

async fn notifications(State(app): State<Shared>, headers: HeaderMap, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::inbox::page(r))
}

async fn specifications(State(app): State<Shared>, headers: HeaderMap, Path(id): Path<u64>, Query(q): Q) -> Response {
    page(&app, &headers, &q, |r, _| buildkiosk_web::specs::page(r, Id::new(id)))
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

// -- commands

/// Turns the posted fields into command parameters: a field named `x[]` collects all its values
/// into a list (blank ones dropped); any other field takes its last value. Fields starting with
/// `_` belong to the form machinery.
fn form_params(form: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for (k, v) in form {
        if k.starts_with('_') {
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
        Err(e) => redirect(&with_flash(&back, false, &e.to_string())),
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

fn rfi_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state.rfis.get(Id::new(item)).filter(|r| r.project == Id::new(project) && view::can_see_rfi(state, actor, r)).map(|r| r.attachments.clone())
}

/// A submittal's files include those on its reviews.
fn submittal_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .submittals
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_submittal(state, actor, r))
        .map(|r| r.attachments.iter().chain(r.reviews.iter().flat_map(|v| v.attachments.iter())).cloned().collect())
}

fn inspection_files(state: &CompanyState, actor: Actor, project: u64, item: u64) -> Option<Vec<FileRef>> {
    state
        .inspections
        .get(Id::new(item))
        .filter(|r| r.project == Id::new(project) && view::can_see_inspection(state, actor, r))
        .map(|r| r.attachments.clone())
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
