//! The bidder portal's server side: links that open one package for one company without an account.
//!
//! A link is `/bid/<package>.<number>.<generation>.<signature>`. The signature is an HMAC of the company, package,
//! invitation number and generation under the server's secret, so nothing stored (state, journal, backups) can open
//! the portal, and a link stops working when the invitation is cancelled, sent again (new generation) or past its
//! last day. The server only checks the signature; the engine's `portal_*` commands (System-only) re-check the
//! invitation itself, and every failure looks the same.

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Multipart, Path, Query, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use buildkiosk_engine::Actor;
use buildkiosk_engine::model::bidding::BidStatus;
use serde_json::{Map, Value, json};

use super::{App, Shared, auth, flash_from, get_form, html, mail, now_ms, parse_form, plain, redirect, safe_filename, with_flash};

type Q = Query<HashMap<String, String>>;

pub(crate) fn routes(router: Router<Shared>) -> Router<Shared> {
    router
        .route("/bid/{token}", get(page))
        .route("/bid/{token}/submit", post(submit))
        .route("/bid/{token}/withdraw", post(withdraw))
        .route("/bid/{token}/ack", post(ack))
        .route("/bid/{token}/ask", post(ask))
        .route("/bid/{token}/upload", post(upload).layer(axum::extract::DefaultBodyLimit::max(super::MAX_UPLOAD + 64 * 1024)))
        .route("/bid/{token}/remove", post(remove))
        .route("/bid/open/{token}", get(open_page))
        .route("/bid/open/{token}/register", post(register))
        .route("/bid/{token}/files/{sha}", get(file))
}

/// What a link says once its signature has checked out.
struct Link {
    project: u64,
    package: u64,
    number: u32,
    generation: u32,
    /// The whole link, for building paths and the form token.
    token: String,
}

fn signed(slug: &str, package: u64, number: u32, generation: u32) -> String {
    format!("bid-link:{slug}:{package}:{number}:{generation}")
}

/// The link's last part, for a package, invitation number and generation.
pub(crate) fn token(secret: &[u8], slug: &str, package: u64, number: u32, generation: u32) -> String {
    format!("{package}.{number}.{generation}.{}", auth::sign(secret, &signed(slug, package, number, generation)))
}

/// Checks a link's signature and finds its package. `None` for anything that isn't a link this server made.
fn resolve(app: &App, slug: &str, state: &buildkiosk_engine::model::CompanyState, token: &str) -> Option<Link> {
    if token.len() > 200 {
        return None;
    }
    let mut parts = token.splitn(4, '.');
    let package: u64 = parts.next()?.parse().ok()?;
    let number: u32 = parts.next()?.parse().ok()?;
    let generation: u32 = parts.next()?.parse().ok()?;
    let sig = parts.next()?;
    if !auth::verify(app.secret(), &signed(slug, package, number, generation), sig) {
        return None;
    }
    let project = state.bid_packages.get(buildkiosk_core::Id::new(package))?.project.raw();
    Some(Link { project, package, number, generation, token: token.to_string() })
}

fn base(l: &Link) -> String {
    format!("/bid/{}", l.token)
}

fn csrf(app: &App, l: &Link) -> String {
    auth::csrf_token(app.secret(), &format!("bid:{}", l.token))
}

fn args(l: &Link) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("project".into(), json!(l.project));
    m.insert("id".into(), json!(l.package));
    m.insert("invitation".into(), json!(l.number));
    m.insert("generation".into(), json!(l.generation));
    m
}

/// Every portal response: never cached, never leaks the link through a referrer, not indexed.
fn guard(mut res: Response) -> Response {
    let h = res.headers_mut();
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("x-robots-tag", HeaderValue::from_static("noindex, nofollow"));
    res
}

fn invalid() -> Response {
    guard(html(StatusCode::NOT_FOUND, buildkiosk_web::portal::bad_link()))
}

async fn page(State(app): State<Shared>, Path(token): Path<String>, Query(q): Q) -> Response {
    let mut inner = app.lock();
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return invalid() };
    let Some(l) = inner.tenant.as_ref().and_then(|t| resolve(&app, &slug, t.engine.state(), &token)) else { return invalid() };
    match inner.run(Actor::System, "bid_package.portal_view", &Value::Object(args(&l))) {
        Ok(view) => guard(html(StatusCode::OK, buildkiosk_web::portal::page(&view, &base(&l), &csrf(&app, &l), flash_from(&q).as_ref()))),
        Err(_) => invalid(),
    }
}

/// Opens a form post: the link must be genuine, the form token right and the link not posting too fast.
fn open_post(app: &App, inner: &mut super::Inner, token: &str, form: &[(String, String)]) -> Result<Link, Box<Response>> {
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return Err(Box::new(invalid())) };
    let Some(l) = inner.tenant.as_ref().and_then(|t| resolve(app, &slug, t.engine.state(), token)) else { return Err(Box::new(invalid())) };
    if !auth::csrf_ok(app.secret(), &format!("bid:{}", l.token), get_form(form, "_csrf")) {
        return Err(Box::new(guard(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again."))));
    }
    if !inner.portal_limiter.allow(&format!("{}:{}", l.package, l.number)) {
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

/// The bid the form describes, as `bid_package.portal_submit` parameters.
fn submit_params(l: &Link, form: &[(String, String)]) -> Value {
    let mut p = args(l);
    let mut lines = Vec::new();
    let mut alternates = Vec::new();
    let mut scope = Vec::new();
    for (key, value) in form {
        let value = value.trim();
        let index = |prefix: &str| key.strip_prefix(prefix).and_then(|n| n.parse::<u32>().ok()).filter(|n| *n < 1_000);
        if let Some(i) = index("line_") {
            if !value.is_empty() {
                lines.push(json!({"item": i, "unit_price": value}));
            }
        } else if let Some(i) = index("alt_") {
            if !value.is_empty() {
                alternates.push(json!({"index": i, "amount": value}));
            }
        } else if let Some(i) = index("scope_") {
            let adjustment = get_form(form, &format!("adj_{i}")).trim();
            scope.push(json!({"item": i, "answer": value, "adjustment": adjustment}));
        }
    }
    p.insert("lines".into(), Value::Array(lines));
    p.insert("alternates".into(), Value::Array(alternates));
    p.insert("scope".into(), Value::Array(scope));
    p.insert("amount".into(), json!(get_form(form, "amount").trim()));
    p.insert("notes".into(), json!(get_form(form, "notes")));
    Value::Object(p)
}

async fn submit(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let result = inner.run(Actor::System, "bid_package.portal_submit", &submit_params(&l, &form));
    if let Ok(out) = &result
        && let Some((smtp, mail)) = receipt(&inner, l.package, l.number, &base(&l), out)
    {
        // Best effort: the bid is already recorded, so a failed email only gets logged.
        std::thread::spawn(move || {
            if let Err(e) = mail::send(&smtp, &mail) {
                log::warn!("could not email a bid receipt: {e}");
            }
        });
    }
    finish(&l, result, |out| format!("Your bid was received. Reference {}.", out.get("reference").and_then(Value::as_str).unwrap_or("")))
}

async fn withdraw(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let result = inner.run(Actor::System, "bid_package.portal_withdraw", &Value::Object(args(&l)));
    finish(&l, result, |_| "Your bid was withdrawn.".to_string())
}

async fn ack(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("number".into(), json!(get_form(&form, "number")));
    let result = inner.run(Actor::System, "bid_package.portal_ack", &Value::Object(p));
    finish(&l, result, |_| "Thank you. That addendum is marked as read.".to_string())
}

async fn ask(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("text".into(), json!(get_form(&form, "question")));
    let result = inner.run(Actor::System, "bid_package.portal_ask", &Value::Object(p));
    finish(&l, result, |_| "Your question was sent. The answer will appear on this page.".to_string())
}

/// The email that tells a bidder their bid arrived, when outgoing email is set up.
pub(crate) fn receipt(
    inner: &super::Inner,
    package: u64,
    number: u32,
    page_path: &str,
    out: &Value,
) -> Option<(buildkiosk_store::creds::SmtpSettings, mail::Mail)> {
    let smtp = inner.creds.smtp.clone()?;
    let state = inner.tenant.as_ref()?.engine.state();
    let b = state.bid_packages.get(buildkiosk_core::Id::new(package))?;
    let inv = b.invitations.iter().find(|i| i.number == number)?;
    let company = buildkiosk_engine::view::vendor_name(state, inv.vendor);
    let reference = out.get("reference").and_then(Value::as_str).unwrap_or("");
    let amount = out.get("amount").and_then(Value::as_str).unwrap_or("");
    let received = out.get("received").and_then(Value::as_str).unwrap_or("");
    let page = if smtp.base_url.is_empty() { String::new() } else { format!("\nYour bid page: {}{page_path}\n", smtp.base_url) };
    let due = b.due.map(|d| format!("You can change or withdraw your bid on your bid page until {}.", d.pretty())).unwrap_or_default();
    Some((
        smtp,
        mail::Mail {
            id: 0,
            to: inv.email.clone(),
            subject: format!("Bid received: {}", b.title).chars().filter(|c| !c.is_control()).take(120).collect(),
            body: format!(
                "We received the bid from {company} for \"{}\".\n\nReference: {reference}\nAmount: {amount}\nReceived: {received}\n\n{due}{page}\nKeep this email for your records.\n",
                b.title
            ),
        },
    ))
}

/// Attaches a file to the company's own bid (a proposal, a bond). Multipart: `_csrf` and `file`.
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
    let result = inner.run(Actor::System, "bid_package.portal_attach", &Value::Object(p));
    finish(&l, result, |_| "File attached to your bid.".to_string())
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
    let result = inner.run(Actor::System, "bid_package.portal_detach", &Value::Object(p));
    finish(&l, result, |_| "File removed from your bid.".to_string())
}

// ---- the open link: companies ask to be invited ----

struct OpenLink {
    project: u64,
    package: u64,
    generation: u32,
    token: String,
}

fn open_signed(slug: &str, package: u64, generation: u32) -> String {
    format!("bid-open:{slug}:{package}:{generation}")
}

/// The open link's last part.
pub(crate) fn open_token(secret: &[u8], slug: &str, package: u64, generation: u32) -> String {
    format!("{package}.{generation}.{}", auth::sign(secret, &open_signed(slug, package, generation)))
}

fn resolve_open(app: &App, slug: &str, state: &buildkiosk_engine::model::CompanyState, token: &str) -> Option<OpenLink> {
    if token.len() > 200 {
        return None;
    }
    let mut parts = token.splitn(3, '.');
    let package: u64 = parts.next()?.parse().ok()?;
    let generation: u32 = parts.next()?.parse().ok()?;
    let sig = parts.next()?;
    if !auth::verify(app.secret(), &open_signed(slug, package, generation), sig) {
        return None;
    }
    let project = state.bid_packages.get(buildkiosk_core::Id::new(package))?.project.raw();
    Some(OpenLink { project, package, generation, token: token.to_string() })
}

fn open_args(l: &OpenLink) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("project".into(), json!(l.project));
    m.insert("id".into(), json!(l.package));
    m.insert("generation".into(), json!(l.generation));
    m
}

fn open_base(l: &OpenLink) -> String {
    format!("/bid/open/{}", l.token)
}

async fn open_page(State(app): State<Shared>, Path(token): Path<String>, Query(q): Q) -> Response {
    let mut inner = app.lock();
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return invalid() };
    let Some(l) = inner.tenant.as_ref().and_then(|t| resolve_open(&app, &slug, t.engine.state(), &token)) else { return invalid() };
    match inner.run(Actor::System, "bid_package.portal_open_view", &Value::Object(open_args(&l))) {
        Ok(view) => {
            let csrf = auth::csrf_token(app.secret(), &format!("bidopen:{}", l.token));
            guard(html(StatusCode::OK, buildkiosk_web::portal::open_page(&view, &open_base(&l), &csrf, flash_from(&q).as_ref())))
        }
        Err(_) => invalid(),
    }
}

async fn register(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return invalid() };
    let Some(l) = inner.tenant.as_ref().and_then(|t| resolve_open(&app, &slug, t.engine.state(), &token)) else { return invalid() };
    if !auth::csrf_ok(app.secret(), &format!("bidopen:{}", l.token), get_form(&form, "_csrf")) {
        return guard(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again."));
    }
    if !inner.portal_limiter.allow(&format!("open:{}", l.package)) {
        return guard(plain(StatusCode::TOO_MANY_REQUESTS, "Too many requests. Wait a few minutes and try again."));
    }
    let done = "Thank you. The team will review your request and email you a link to the bid.";
    // A field people can't see: a program that fills it in is not a company, and gets the same thank-you.
    if !get_form(&form, "website").trim().is_empty() {
        return guard(redirect(&with_flash(&open_base(&l), true, done)));
    }
    let mut p = open_args(&l);
    for k in ["company", "contact", "email", "phone"] {
        p.insert(k.into(), json!(get_form(&form, k).trim()));
    }
    let result = inner.run(Actor::System, "bid_package.portal_register", &Value::Object(p));
    guard(match result {
        Ok(_) => redirect(&with_flash(&open_base(&l), true, done)),
        Err(e) => redirect(&with_flash(&open_base(&l), false, &e.to_string())),
    })
}

/// The open link for the team page.
pub(crate) fn team_open_link(app: &App, slug: &str, package: u64, generation: u32) -> String {
    let inner = app.lock();
    let host = inner.creds.smtp.as_ref().map(|s| s.base_url.clone()).unwrap_or_default();
    format!("{host}/bid/open/{}", open_token(app.secret(), slug, package, generation))
}

/// A bid document, downloaded as an attachment. Only files the link's own view lists.
async fn file(State(app): State<Shared>, Path((token, sha)): Path<(String, String)>) -> Response {
    let found = {
        let mut inner = app.lock();
        let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return invalid() };
        let Some(l) = inner.tenant.as_ref().and_then(|t| resolve(&app, &slug, t.engine.state(), &token)) else { return invalid() };
        let Ok(view) = inner.run(Actor::System, "bid_package.portal_view", &Value::Object(args(&l))) else { return invalid() };
        // The package's bid documents and the files on the company's own bid, nothing else.
        let own = view.get("bid").and_then(|b| b.get("files")).and_then(Value::as_array).into_iter().flatten();
        let docs = view.get("documents").and_then(Value::as_array).into_iter().flatten();
        let doc = docs.chain(own).find(|f| f.get("sha256").and_then(Value::as_str) == Some(sha.as_str()));
        let name = doc.and_then(|f| f.get("name").and_then(Value::as_str)).unwrap_or("file").to_string();
        let path = doc.and_then(|_| inner.tenant.as_ref()).and_then(|t| t.store.blobs.path(&sha));
        path.map(|p| (p, name))
    };
    let Some((path, name)) = found else { return invalid() };
    let bytes = match tokio::task::spawn_blocking(move || std::fs::read(path)).await {
        Ok(Ok(b)) => b,
        _ => return guard(plain(StatusCode::NOT_FOUND, "That file is missing from storage.")),
    };
    let mut res = bytes.into_response();
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{}\"", safe_filename(&name))) {
        res.headers_mut().insert("content-disposition", v);
    }
    guard(res)
}

// ---- the team's side: emailing links ----

/// One link waiting to be emailed.
pub(crate) struct Outgoing {
    pub(crate) project: u64,
    pub(crate) package: u64,
    pub(crate) number: u32,
    pub(crate) generation: u32,
    pub(crate) mail: mail::Mail,
}

/// Links not yet emailed, for packages that are out for bid. Needs outgoing email and the server's public address.
pub(crate) fn pending(app: &App) -> Option<(buildkiosk_store::creds::SmtpSettings, Vec<Outgoing>)> {
    let inner = app.lock();
    let smtp = inner.creds.smtp.clone().filter(|s| !s.base_url.is_empty())?;
    let t = inner.tenant.as_ref()?;
    let state = t.engine.state();
    let today = buildkiosk_core::Timestamp::from_millis(now_ms()).date_at(state.utc_offset());
    let mut out = Vec::new();
    for b in state.bid_packages.iter().filter(|b| b.deleted.is_none() && b.status == BidStatus::Open) {
        let project = state.projects.get(b.project).map(|p| p.name.clone()).unwrap_or_default();
        for i in b.invitations.iter().filter(|i| i.sent.is_none() && i.active(today)) {
            if out.len() >= 50 {
                break;
            }
            let link = format!("{}/bid/{}", smtp.base_url, token(app.secret(), &t.slug, b.id.raw(), i.number, i.generation));
            let company = buildkiosk_engine::view::vendor_name(state, i.vendor);
            let due = b.due.map(|d| format!("\nBids are due {}.", d.pretty())).unwrap_or_default();
            out.push(Outgoing {
                project: b.project.raw(),
                package: b.id.raw(),
                number: i.number,
                generation: i.generation,
                mail: mail::Mail {
                    id: u64::from(i.number),
                    to: i.email.clone(),
                    subject: format!("Invitation to bid: {}", b.title).chars().filter(|c| !c.is_control()).take(120).collect(),
                    body: format!(
                        "{company} is invited to bid on \"{}\" ({project}).{due}\n\nOpen your bid page, read the documents and send your bid here:\n{link}\n\n\
                         This link is meant for you alone. Please don't forward it: anyone who has it can send or change a bid for {company}.\n",
                        b.title
                    ),
                },
            });
        }
    }
    Some((smtp, out))
}

/// Emails the links that haven't gone out and records each one as sent. Returns how many.
pub(crate) fn invite_pass(app: &App) -> usize {
    let Some((smtp, outgoing)) = pending(app) else { return 0 };
    let mut sent = 0;
    for o in &outgoing {
        match mail::send(&smtp, &o.mail) {
            Ok(()) => {
                sent += 1;
                let mut inner = app.lock();
                let done = json!({"project": o.project, "id": o.package, "number": o.number, "generation": o.generation});
                if let Err(e) = inner.run(Actor::System, "bid_package.invite_sent", &done) {
                    log::warn!("could not record a sent bid link: {e}");
                }
            }
            Err(e) => {
                // Try again on the next pass; nothing is lost.
                log::warn!("could not email a bid link: {e}");
                return sent;
            }
        }
    }
    sent
}

/// The link for a team page (`bid_package` page shows it so it can be copied when email isn't set up).
pub(crate) fn team_link(app: &App, slug: &str, package: u64, number: u32, generation: u32) -> String {
    let inner = app.lock();
    let host = inner.creds.smtp.as_ref().map(|s| s.base_url.clone()).unwrap_or_default();
    format!("{host}/bid/{}", token(app.secret(), slug, package, number, generation))
}
