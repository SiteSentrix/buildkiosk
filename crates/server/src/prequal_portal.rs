//! The prequalification link's server side: a company's contact fills in the record without an account.
//!
//! A link is `/prequal/<company>.<number>.<generation>.<signature>`, built the way the bidder portal's are: the signature
//! is an HMAC of the company's slug, record and link number and generation under the server's secret, so nothing stored
//! can open it. The server only checks the signature; the engine's `prequal.portal_*` commands (system-only) re-check the
//! invitation, and every failure looks the same.

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use buildkiosk_engine::Actor;
use serde_json::{Map, Value, json};

use super::portal::{guard, invalid};
use super::{App, Shared, auth, flash_from, get_form, html, mail, now_ms, parse_form, plain, redirect, with_flash};

type Q = Query<HashMap<String, String>>;

pub(crate) fn routes(router: Router<Shared>) -> Router<Shared> {
    router
        .route("/prequal/{token}", get(page))
        .route("/prequal/{token}/save", post(save))
        .route("/prequal/{token}/submit", post(submit))
        .route("/prequal/{token}/message", post(message))
        .route("/prequal/{token}/upload", post(upload).layer(axum::extract::DefaultBodyLimit::max(super::MAX_UPLOAD + 64 * 1024)))
        .route("/prequal/{token}/remove", post(remove))
}

/// What a link says once its signature has checked out.
struct Link {
    vendor: u64,
    number: u32,
    generation: u32,
    token: String,
}

fn signed(slug: &str, vendor: u64, number: u32, generation: u32) -> String {
    format!("prequal-link:{slug}:{vendor}:{number}:{generation}")
}

/// The link's last part, for a company, link number and generation.
pub(crate) fn token(secret: &[u8], slug: &str, vendor: u64, number: u32, generation: u32) -> String {
    format!("{vendor}.{number}.{generation}.{}", auth::sign(secret, &signed(slug, vendor, number, generation)))
}

/// Checks a link's signature. `None` for anything that isn't a link this server made.
fn resolve(app: &App, slug: &str, token: &str) -> Option<Link> {
    if token.len() > 200 {
        return None;
    }
    let mut parts = token.splitn(4, '.');
    let vendor: u64 = parts.next()?.parse().ok()?;
    let number: u32 = parts.next()?.parse().ok()?;
    let generation: u32 = parts.next()?.parse().ok()?;
    let sig = parts.next()?;
    if !auth::verify(app.secret(), &signed(slug, vendor, number, generation), sig) {
        return None;
    }
    Some(Link { vendor, number, generation, token: token.to_string() })
}

fn base(l: &Link) -> String {
    format!("/prequal/{}", l.token)
}

fn csrf(app: &App, l: &Link) -> String {
    auth::csrf_token(app.secret(), &format!("prequal:{}", l.token))
}

fn args(l: &Link) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("vendor".into(), json!(l.vendor));
    m.insert("invitation".into(), json!(l.number));
    m.insert("generation".into(), json!(l.generation));
    m
}

async fn page(State(app): State<Shared>, Path(token): Path<String>, Query(q): Q) -> Response {
    let mut inner = app.lock();
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return invalid() };
    let Some(l) = resolve(&app, &slug, &token) else { return invalid() };
    match inner.run(Actor::System, "prequal.portal_view", &Value::Object(args(&l))) {
        Ok(view) => guard(html(StatusCode::OK, buildkiosk_web::prequal_portal::page(&view, &base(&l), &csrf(&app, &l), flash_from(&q).as_ref()))),
        Err(_) => invalid(),
    }
}

/// Opens a form post: the link must be genuine, the form token right and the link not posting too fast.
fn open_post(app: &App, inner: &mut super::Inner, token: &str, form: &[(String, String)]) -> Result<Link, Box<Response>> {
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return Err(Box::new(invalid())) };
    let Some(l) = resolve(app, &slug, token) else { return Err(Box::new(invalid())) };
    if !auth::csrf_ok(app.secret(), &format!("prequal:{}", l.token), get_form(form, "_csrf")) {
        return Err(Box::new(guard(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again."))));
    }
    if !inner.portal_limiter.allow(&format!("pq:{}:{}", l.vendor, l.number)) {
        return Err(Box::new(guard(plain(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Wait a few minutes and try again."))));
    }
    Ok(l)
}

fn finish(l: &Link, result: Result<Value, buildkiosk_engine::EngineError>, done: impl FnOnce(&Value) -> String) -> Response {
    guard(match result {
        Ok(out) => redirect(&with_flash(&base(l), true, &done(&out))),
        Err(e) => redirect(&with_flash(&base(l), false, &e.to_string())),
    })
}

/// What the page's form saves: the answers (`reply_<section>_<question>`) and the company's dates and figures.
fn save_params(l: &Link, form: &[(String, String)]) -> Value {
    let mut p = args(l);
    let mut replies = Vec::new();
    for (key, value) in form {
        let Some(rest) = key.strip_prefix("reply_") else { continue };
        let Some((s, q)) = rest.split_once('_') else { continue };
        let (Ok(s), Ok(q)) = (s.parse::<u32>(), q.parse::<u32>()) else { continue };
        if s < 1_000 && q < 1_000 {
            replies.push(json!({"section": s, "question": q, "text": value}));
        }
    }
    if !replies.is_empty() {
        p.insert("replies".into(), Value::Array(replies));
    }
    for key in ["insurance_expires", "license_expires", "emr", "bonding_capacity"] {
        if form.iter().any(|(k, _)| k == key) {
            p.insert(key.into(), json!(get_form(form, key).trim()));
        }
    }
    Value::Object(p)
}

async fn save(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let result = inner.run(Actor::System, "prequal.portal_save", &save_params(&l, &form));
    finish(&l, result, |_| "Your answers were saved. Come back to this page any time to finish them.".to_string())
}

async fn submit(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    // What is on the page is saved first, so nothing typed is lost if a question is still open.
    if let Err(e) = inner.run(Actor::System, "prequal.portal_save", &save_params(&l, &form)) {
        return finish(&l, Err(e), |_| String::new());
    }
    let mut p = args(&l);
    p.insert("message".into(), json!(get_form(&form, "message")));
    let result = inner.run(Actor::System, "prequal.portal_submit", &Value::Object(p));
    finish(&l, result, |_| "Thank you. Your record was sent for review.".to_string())
}

async fn message(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("message".into(), json!(get_form(&form, "message")));
    let result = inner.run(Actor::System, "prequal.portal_message", &Value::Object(p));
    finish(&l, result, |_| "Your message was sent to the reviewers.".to_string())
}

/// Attaches a document. Multipart: `_csrf` and `file`.
async fn upload(State(app): State<Shared>, Path(token): Path<String>, mut multipart: Multipart) -> Response {
    let (mut csrf_field, mut file) = (String::new(), None);
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return guard(plain(StatusCode::PAYLOAD_TOO_LARGE, "The upload failed or the file is larger than 25 MB.")),
        };
        let name = field.name().unwrap_or("").to_string();
        let filename = field.file_name().unwrap_or("file").to_string();
        let mime = field.content_type().unwrap_or("application/octet-stream").to_string();
        let Ok(bytes) = field.bytes().await else {
            return guard(plain(StatusCode::PAYLOAD_TOO_LARGE, "The upload failed or the file is larger than 25 MB."));
        };
        match name.as_str() {
            "_csrf" => csrf_field = String::from_utf8_lossy(&bytes).into_owned(),
            "file" if file.is_none() => file = Some((filename, mime, bytes)),
            _ => {}
        }
    }
    let mut inner = app.lock();
    let form = vec![("_csrf".to_string(), csrf_field)];
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let Some((name, mime, bytes)) = file.filter(|(_, _, b)| !b.is_empty()) else {
        return finish(&l, Err(buildkiosk_engine::EngineError::conflict("Choose a file to attach.")), |_| String::new());
    };
    if bytes.len() > super::MAX_UPLOAD {
        return finish(&l, Err(buildkiosk_engine::EngineError::conflict("Files can be at most 25 MB.")), |_| String::new());
    }
    let Some(t) = inner.tenant.as_mut() else { return invalid() };
    let (sha256, size) = match t.store.blobs.put(&bytes) {
        Ok(x) => x,
        Err(e) => return finish(&l, Err(buildkiosk_engine::EngineError::conflict(e.to_string())), |_| String::new()),
    };
    let mut p = args(&l);
    p.extend([
        ("sha256".to_string(), json!(sha256)),
        ("name".to_string(), json!(name)),
        ("size".to_string(), json!(size)),
        ("mime".to_string(), json!(mime)),
    ]);
    let result = inner.run(Actor::System, "prequal.portal_attach", &Value::Object(p));
    finish(&l, result, |_| "Document attached.".to_string())
}

async fn remove(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("sha256".into(), json!(get_form(&form, "sha256")));
    let result = inner.run(Actor::System, "prequal.portal_detach", &Value::Object(p));
    finish(&l, result, |_| "Document removed.".to_string())
}

// ---- the team's side: emailing links and warnings ----

/// The link for a team page, so it can be copied when email isn't set up.
pub(crate) fn team_link(app: &App, slug: &str, vendor: u64, number: u32, generation: u32) -> String {
    let inner = app.lock();
    let host = inner.creds.smtp.as_ref().map(|s| s.base_url.clone()).unwrap_or_default();
    format!("{host}/prequal/{}", token(app.secret(), slug, vendor, number, generation))
}

/// One email waiting to go out, and what to record once it has.
struct Outgoing {
    mail: mail::Mail,
    /// The system command that records it as sent, and its parameters.
    done: (&'static str, Value),
}

/// Links not yet emailed, and warnings for companies whose approval, insurance or licence is about to lapse. Needs
/// outgoing email and the server's public address.
fn pending(app: &App) -> Option<(buildkiosk_store::creds::SmtpSettings, Vec<Outgoing>)> {
    let mut inner = app.lock();
    let smtp = inner.creds.smtp.clone().filter(|s| !s.base_url.is_empty())?;
    let slug = inner.tenant.as_ref()?.slug.clone();
    let mut out = Vec::new();
    {
        let state = inner.tenant.as_ref()?.engine.state();
        let today = buildkiosk_core::Timestamp::from_millis(now_ms()).date_at(state.utc_offset());
        let own = state.company().map(|c| c.name.clone()).unwrap_or_default();
        for r in state.prequalifications.iter().filter(|r| r.deleted.is_none()) {
            let company = buildkiosk_engine::view::vendor_name(state, r.vendor);
            for i in r.invitations.iter().filter(|i| i.sent.is_none() && i.active(today)) {
                if out.len() >= 50 {
                    break;
                }
                let link = format!("{}/prequal/{}", smtp.base_url, token(app.secret(), &slug, r.vendor.raw(), i.number, i.generation));
                out.push(Outgoing {
                    mail: mail::Mail {
                        id: u64::from(i.number),
                        to: i.email.clone(),
                        subject: format!("{own}: prequalification for {company}").chars().filter(|c| !c.is_control()).take(120).collect(),
                        body: format!(
                            "{own} asks {company} to complete its prequalification.\n\nOpen your page, answer the questions, give your insurance and licence dates and attach your documents here:\n{link}\n\n\
                             This link is meant for you alone. Please don't forward it: anyone who has it can change {company}'s record.\n"
                        ),
                    },
                    done: ("prequal.invite_sent", json!({"vendor": r.vendor.raw(), "number": i.number, "generation": i.generation})),
                });
            }
        }
    }
    // Warnings to the company's own contact, from the engine's list of what is due.
    let due = inner.run(Actor::System, "prequal.company_due", &json!({})).ok();
    let state = inner.tenant.as_ref()?.engine.state();
    let own = state.company().map(|c| c.name.clone()).unwrap_or_default();
    for d in due.as_ref().and_then(Value::as_array).into_iter().flatten().take(50) {
        let (Some(vendor), Some(to), Some(number), Some(generation), Some(what), Some(date), Some(days)) = (
            d.get("vendor").and_then(Value::as_u64),
            d.get("email").and_then(Value::as_str),
            d.get("number").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
            d.get("generation").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()),
            d.get("what").and_then(Value::as_str),
            d.get("date").and_then(Value::as_str),
            d.get("days").and_then(Value::as_u64),
        ) else {
            continue;
        };
        let company = d.get("company").and_then(Value::as_str).unwrap_or("");
        let thing = match what {
            "approval" => "prequalification approval",
            "insurance" => "insurance",
            _ => "license",
        };
        let when = if days == 0 { format!("lapsed on {date}") } else { format!("lapses on {date}") };
        let link = format!("{}/prequal/{}", smtp.base_url, token(app.secret(), &slug, vendor, number, generation));
        out.push(Outgoing {
            mail: mail::Mail {
                id: 0,
                to: to.to_string(),
                subject: format!("{own}: {company}'s {thing} {when}").chars().filter(|c| !c.is_control()).take(120).collect(),
                body: format!(
                    "{company}'s {thing} {when}.\n\nPlease send a renewed document and leave a message for the reviewers on your page:\n{link}\n\n\
                     This link is meant for you alone. Please don't forward it.\n"
                ),
            },
            done: ("prequal.company_reminded", json!({"vendor": vendor, "what": what, "date": date, "days": days})),
        });
    }
    Some((smtp, out))
}

/// Emails the links and warnings that haven't gone out and records each as sent. Returns how many.
pub(crate) fn mail_pass(app: &App) -> usize {
    let Some((smtp, outgoing)) = pending(app) else { return 0 };
    let mut sent = 0;
    for o in &outgoing {
        match mail::send(&smtp, &o.mail) {
            Ok(()) => {
                sent += 1;
                let mut inner = app.lock();
                if let Err(e) = inner.run(Actor::System, o.done.0, &o.done.1) {
                    log::warn!("could not record a sent prequalification email: {e}");
                }
            }
            Err(e) => {
                // Try again on the next pass; nothing is lost.
                log::warn!("could not email a prequalification message: {e}");
                return sent;
            }
        }
    }
    sent
}
