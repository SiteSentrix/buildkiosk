//! The contract-signing page's server side: links that open one contract for the other party without an account.
//!
//! A link is `/sign/<kind>.<id>.<generation>.<signature>`. The signature is an HMAC of the company, contract and
//! generation under the server's secret, so nothing stored can open the page, and a link stops working when the
//! contract is sent again (new generation), taken back or past its last day. The server only checks the signature; the
//! engine's `contract.portal_*` commands (System-only) re-check the contract itself, and every failure looks the same.

use std::collections::HashMap;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use buildkiosk_engine::Actor;
use buildkiosk_engine::cmd::signing::Kind;
use buildkiosk_engine::model::finance::SignatureStatus;
use serde_json::{Map, Value, json};

use super::{App, Shared, auth, flash_from, get_form, html, mail, now_ms, parse_form, plain, redirect, with_flash};

type Q = Query<HashMap<String, String>>;

pub(crate) fn routes(router: Router<Shared>) -> Router<Shared> {
    router.route("/sign/{token}", get(page)).route("/sign/{token}/sign", post(sign)).route("/sign/{token}/decline", post(decline))
}

struct Link {
    kind: Kind,
    id: u64,
    generation: u32,
    token: String,
}

fn signed(slug: &str, kind: Kind, id: u64, generation: u32) -> String {
    format!("sign-link:{slug}:{}:{id}:{generation}", kind.key())
}

/// The link's last part, for a contract and generation.
pub(crate) fn token(secret: &[u8], slug: &str, kind: Kind, id: u64, generation: u32) -> String {
    format!("{}.{id}.{generation}.{}", kind.key(), auth::sign(secret, &signed(slug, kind, id, generation)))
}

/// Checks a link's signature. `None` for anything that isn't a link this server made.
fn resolve(app: &App, slug: &str, token: &str) -> Option<Link> {
    if token.len() > 200 {
        return None;
    }
    let mut parts = token.splitn(4, '.');
    let kind = Kind::parse(parts.next()?)?;
    let id: u64 = parts.next()?.parse().ok()?;
    let generation: u32 = parts.next()?.parse().ok()?;
    let sig = parts.next()?;
    if !auth::verify(app.secret(), &signed(slug, kind, id, generation), sig) {
        return None;
    }
    Some(Link { kind, id, generation, token: token.to_string() })
}

fn base(l: &Link) -> String {
    format!("/sign/{}", l.token)
}

fn csrf(app: &App, l: &Link) -> String {
    auth::csrf_token(app.secret(), &format!("sign:{}", l.token))
}

fn args(l: &Link) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("kind".into(), json!(l.kind.key()));
    m.insert("id".into(), json!(l.id));
    m.insert("generation".into(), json!(l.generation));
    m
}

/// Every signing response: never cached, never leaks the link through a referrer, not indexed.
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
    let Some(l) = resolve(&app, &slug, &token) else { return invalid() };
    match inner.run(Actor::System, "contract.portal_view", &Value::Object(args(&l))) {
        Ok(view) => guard(html(StatusCode::OK, buildkiosk_web::sign_portal::page(&view, &base(&l), &csrf(&app, &l), flash_from(&q).as_ref()))),
        Err(_) => invalid(),
    }
}

/// Opens a form post: the link must be genuine, the form token right and the link not posting too fast.
fn open_post(app: &App, inner: &mut super::Inner, token: &str, form: &[(String, String)]) -> Result<Link, Box<Response>> {
    let Some(slug) = inner.tenant.as_ref().map(|t| t.slug.clone()) else { return Err(Box::new(invalid())) };
    let Some(l) = resolve(app, &slug, token) else { return Err(Box::new(invalid())) };
    if !auth::csrf_ok(app.secret(), &format!("sign:{}", l.token), get_form(form, "_csrf")) {
        return Err(Box::new(guard(plain(StatusCode::FORBIDDEN, "The form expired. Go back, reload the page and try again."))));
    }
    if !inner.portal_limiter.allow(&format!("sign:{}:{}", l.kind.key(), l.id)) {
        return Err(Box::new(guard(plain(StatusCode::TOO_MANY_REQUESTS, "Too many tries. Wait a few minutes and try again."))));
    }
    Ok(l)
}

fn finish(l: &Link, result: Result<Value, buildkiosk_engine::EngineError>, done: &str) -> Response {
    guard(match result {
        Ok(_) => redirect(&with_flash(&base(l), true, done)),
        Err(e) => redirect(&with_flash(&base(l), false, &e.to_string())),
    })
}

async fn sign(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("name".into(), json!(get_form(&form, "name").trim()));
    p.insert("agree".into(), json!(get_form(&form, "agree") == "true"));
    p.insert("terms".into(), json!(get_form(&form, "terms").trim()));
    let result = inner.run(Actor::System, "contract.portal_sign", &Value::Object(p));
    if result.is_ok()
        && let Some((smtp, mail)) = receipt(&inner, &l, &base(&l))
    {
        // Best effort: the signature is already recorded, so a failed email only gets logged.
        std::thread::spawn(move || {
            if let Err(e) = mail::send(&smtp, &mail) {
                log::warn!("could not email a signing receipt: {e}");
            }
        });
    }
    finish(&l, result, "Thank you. The contract is signed and the team has been told.")
}

async fn decline(State(app): State<Shared>, Path(token): Path<String>, body: Bytes) -> Response {
    let form = parse_form(&body);
    let mut inner = app.lock();
    let l = match open_post(&app, &mut inner, &token, &form) {
        Ok(l) => l,
        Err(r) => return *r,
    };
    let mut p = args(&l);
    p.insert("note".into(), json!(get_form(&form, "note").trim()));
    let result = inner.run(Actor::System, "contract.portal_decline", &Value::Object(p));
    finish(&l, result, "The team has been told that you didn't sign.")
}

/// The email that tells the signer their signature arrived, when outgoing email is set up.
fn receipt(inner: &super::Inner, l: &Link, page_path: &str) -> Option<(buildkiosk_store::creds::SmtpSettings, mail::Mail)> {
    let smtp = inner.creds.smtp.clone()?;
    let state = inner.tenant.as_ref()?.engine.state();
    let (label, title, sig) = doc_facts(state, l.kind, l.id)?;
    let to = sig.link.as_ref()?.email.clone();
    let page = if smtp.base_url.is_empty() { String::new() } else { format!("\nThe contract: {}{page_path}\n", smtp.base_url) };
    Some((
        smtp,
        mail::Mail {
            id: 0,
            to,
            subject: format!("Signed: {label}").chars().filter(|c| !c.is_control()).take(120).collect(),
            body: format!(
                "We recorded the signature of {} on {label}, \"{title}\"{}.\n{page}\nKeep this email for your records.\n",
                sig.signer,
                sig.signed_on.map(|d| format!(" on {}", d.pretty())).unwrap_or_default()
            ),
        },
    ))
}

/// A contract's label, title and signature.
fn doc_facts(
    state: &buildkiosk_engine::model::CompanyState,
    kind: Kind,
    id: u64,
) -> Option<(String, String, buildkiosk_engine::model::finance::ContractSignature)> {
    use buildkiosk_core::Id;
    match kind {
        Kind::Commitment => {
            state.commitments.get(Id::new(id)).filter(|c| c.deleted.is_none()).map(|c| (c.label(), c.title.clone(), c.signature.clone()))
        }
        Kind::PrimeContract => state
            .prime_contracts
            .get(Id::new(id))
            .filter(|c| c.deleted.is_none())
            .map(|c| (format!("Prime contract #{}", c.number), c.title.clone(), c.signature.clone())),
        Kind::PrimeChange => state
            .prime_changes
            .get(Id::new(id))
            .filter(|c| c.deleted.is_none())
            .map(|c| (format!("Prime change order #{}", c.number), c.title.clone(), c.signature.clone())),
    }
}

// ---- the team's side: emailing links ----

pub(crate) struct Outgoing {
    kind: Kind,
    id: u64,
    generation: u32,
    mail: mail::Mail,
}

/// Every contract out for signature whose link hasn't been emailed.
pub(crate) fn pending(app: &App) -> Option<(buildkiosk_store::creds::SmtpSettings, Vec<Outgoing>)> {
    let inner = app.lock();
    let smtp = inner.creds.smtp.clone().filter(|s| !s.base_url.is_empty())?;
    let t = inner.tenant.as_ref()?;
    let state = t.engine.state();
    let today = buildkiosk_core::Timestamp::from_millis(now_ms()).date_at(state.utc_offset());
    let own = state.company().map(|c| c.name.clone()).unwrap_or_default();
    let mut out = Vec::new();
    let mut add = |kind: Kind, id: u64, label: String, title: String, sig: &buildkiosk_engine::model::finance::ContractSignature| {
        let Some(link) = sig.link.as_ref().filter(|l| l.sent.is_none() && l.active(today) && sig.status == SignatureStatus::Sent) else { return };
        if out.len() >= 50 {
            return;
        }
        let url = format!("{}/sign/{}", smtp.base_url, token(app.secret(), &t.slug, kind, id, link.generation));
        let by = if own.is_empty() { String::new() } else { format!(" from {own}") };
        let until = link.expires.map(|d| format!("\nThe link works until {}.", d.pretty())).unwrap_or_default();
        out.push(Outgoing {
            kind,
            id,
            generation: link.generation,
            mail: mail::Mail {
                id: id ^ (u64::from(link.generation) << 20),
                to: link.email.clone(),
                subject: format!("For your signature: {label}").chars().filter(|c| !c.is_control()).take(120).collect(),
                body: format!(
                    "{label}, \"{title}\"{by} is ready for your signature.{until}\n\nRead it and sign it here:\n{url}\n\n\
                     This link is meant for you alone. Please don't forward it: anyone who has it can sign for you.\n"
                ),
            },
        });
    };
    for c in state.commitments.iter().filter(|c| c.deleted.is_none()) {
        add(Kind::Commitment, c.id.raw(), c.label(), c.title.clone(), &c.signature);
    }
    for c in state.prime_contracts.iter().filter(|c| c.deleted.is_none()) {
        add(Kind::PrimeContract, c.id.raw(), format!("Prime contract #{}", c.number), c.title.clone(), &c.signature);
    }
    for c in state.prime_changes.iter().filter(|c| c.deleted.is_none()) {
        add(Kind::PrimeChange, c.id.raw(), format!("Prime change order #{}", c.number), c.title.clone(), &c.signature);
    }
    Some((smtp, out))
}

/// Emails the links that haven't gone out and records each one as sent. Returns how many.
pub(crate) fn mail_pass(app: &App) -> usize {
    let Some((smtp, outgoing)) = pending(app) else { return 0 };
    let mut sent = 0;
    for o in &outgoing {
        match mail::send(&smtp, &o.mail) {
            Ok(()) => {
                sent += 1;
                let mut inner = app.lock();
                let done = json!({"kind": o.kind.key(), "id": o.id, "generation": o.generation});
                if let Err(e) = inner.run(Actor::System, "contract.link_sent", &done) {
                    log::warn!("could not record a sent signing link: {e}");
                }
            }
            Err(e) => {
                // Try again on the next pass; nothing is lost.
                log::warn!("could not email a signing link: {e}");
                return sent;
            }
        }
    }
    sent
}

/// The full link for a team page, with the server's public address in front when it is set.
pub(crate) fn team_link(app: &App, slug: &str, kind: Kind, id: u64, generation: u32) -> String {
    let inner = app.lock();
    let host = inner.creds.smtp.as_ref().map(|s| s.base_url.clone()).unwrap_or_default();
    format!("{host}/sign/{}", token(app.secret(), slug, kind, id, generation))
}
