//! Prequalification links: a company's contact fills in its own questionnaire, dates and files without an account.
//!
//! The same design as the bidder portal (`docs/bidder-portal.md`): the team gives a contact a link (`prequal.invite`);
//! the server works the link out from its secret (nothing stored can open it), emails it, and runs the `portal_*`
//! commands below on the contact's behalf. Those commands are system-only, so a person, API token or agent can't call
//! them, and each re-checks the invitation (the right company, the generation the server verified, not cancelled, not past
//! its last day); every failure looks the same.
//!
//! What the company can do: answer the questionnaire and give its insurance and licence dates, experience rate and
//! bonding capacity while its record is pending or was rejected; attach files at any time (an insurance certificate
//! for a renewal); put the record forward; and leave the reviewers a message. It can never approve, score, change the
//! approval date, see the team's notes or score, or reach any other company's record. Putting a record forward tells the
//! Prequalification admins.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::prequal::{PrequalInvitation, PrequalStatus, Prequalification, ReminderSent};
use buildkiosk_model::{Level, Tool, perm};
use serde_json::{Value, json};

use super::prequal::{MAX_ATTACHMENTS, bucket, cell, live, parse_emr, set_answer};
use super::{Access, CommandSpec};
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "prequal.invite",
        "Send a company a prequalification link",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; email: the contact's address; expires?: YYYY-MM-DD (the link stops working after that day; blank means until cancelled) (the server emails the link, and the company fills in its questionnaire, dates and files itself; the record is made if the company has none)",
        invite,
    ),
    CommandSpec::mutate(
        "prequal.resend_invite",
        "Send a prequalification link again",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; number: link number; email?: a new address; expires?: YYYY-MM-DD (a new link; the earlier one stops working)",
        resend_invite,
    ),
    CommandSpec::mutate(
        "prequal.revoke_invite",
        "Cancel a prequalification link",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; number: link number",
        revoke_invite,
    ),
    CommandSpec::mutate(
        "prequal.invite_sent",
        "Record that a prequalification link was emailed",
        Tool::Prequalification,
        Access::System,
        "vendor: company id; number: link number; generation: the link's generation that was sent",
        invite_sent,
    ),
    CommandSpec::query(
        "prequal.invitations",
        "List a company's prequalification links",
        Tool::Prequalification,
        Access::Company(Level::Read),
        "vendor: company id (who each link was sent to, when, and whether it still works; never the link itself)",
        invitations,
    ),
    CommandSpec::query(
        "prequal.company_due",
        "Company warnings to email",
        Tool::Prequalification,
        Access::System,
        "(none) (approvals, insurance or licences 60, 30 or 7 days from lapsing, or lapsed, whose company has a working link that was sent, with the link to put in the email; record each with prequal.company_reminded once sent)",
        company_due,
    ),
    CommandSpec::mutate(
        "prequal.company_reminded",
        "Record a warning emailed to a company",
        Tool::Prequalification,
        Access::System,
        "vendor: company id; what: approval|insurance|license; date: YYYY-MM-DD; days: 60|30|7|0",
        company_reminded,
    ),
    CommandSpec::query(
        "prequal.portal_view",
        "Company's own prequalification page",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation (what the company sees through its link; run by the server after it checked the link)",
        portal_view,
    ),
    CommandSpec::mutate(
        "prequal.portal_save",
        "Company saves its answers",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation; replies?: [{section, question, text}]; insurance_expires?, license_expires?: YYYY-MM-DD; emr?: rate such as 0.85; bonding_capacity?: amount (only while the record is pending or was rejected)",
        portal_save,
    ),
    CommandSpec::mutate(
        "prequal.portal_submit",
        "Company puts its record forward",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation; message?: text for the reviewers (every question must be answered; tells the Prequalification admins)",
        portal_submit,
    ),
    CommandSpec::mutate(
        "prequal.portal_attach",
        "Company attaches a file",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation; sha256, name, size, mime?",
        portal_attach,
    ),
    CommandSpec::mutate(
        "prequal.portal_detach",
        "Company removes a file it attached",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation; sha256 (only a file the company attached itself, while the record is pending or was rejected)",
        portal_detach,
    ),
    CommandSpec::mutate(
        "prequal.portal_message",
        "Company leaves a message",
        Tool::Prequalification,
        Access::System,
        "vendor, invitation, generation; message: text (tells the Prequalification admins; for a renewal, say what was attached)",
        portal_message,
    ),
];

/// Most links one record keeps, live or cancelled.
const MAX_INVITATIONS: usize = 20;

fn bad_link() -> EngineError {
    EngineError::not_found("this link is not valid")
}

fn invitation_number(p: &Params<'_>, key: &str) -> Result<u32> {
    p.int(key, 1..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad(key, "is required"))
}

fn expiry(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<Date>> {
    let day = p.date("expires")?;
    if day.is_some_and(|d| d < ctx.today()) {
        return Err(p.bad("expires", "is in the past"));
    }
    Ok(day)
}

/// The record a team command names, made if the company has none yet.
fn record_or_new(ctx: &mut Ctx<'_>, vendor: Id<Vendor>) -> Prequalification {
    if let Some(r) = live(ctx.state, vendor) {
        return r.clone();
    }
    let stamp = ctx.stamp();
    Prequalification {
        id: ctx.state.prequalifications.alloc(),
        vendor,
        status: PrequalStatus::Pending,
        score: None,
        valid_until: None,
        insurance_expires: None,
        license_expires: None,
        emr_hundredths: None,
        bonding_capacity: None,
        notes: String::new(),
        form: None,
        answers: Vec::new(),
        attachments: Vec::new(),
        reminders: Vec::new(),
        invitations: Vec::new(),
        company_reminders: Vec::new(),
        submitted_via: None,
        portal_files: Vec::new(),
        company_message: String::new(),
        reviewed: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    }
}

fn existing(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Prequalification> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    live(ctx.state, vendor).cloned().ok_or_else(|| EngineError::not_found("that company has no prequalification record yet"))
}

fn invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    let email = p.email("email")?.ok_or_else(|| p.bad("email", "is required"))?;
    let expires = expiry(ctx, p)?;
    let mut r = record_or_new(ctx, vendor);
    if r.status == PrequalStatus::Approved && ctx.company_level(Tool::Prequalification) != Level::Admin {
        return Err(EngineError::forbidden("only an admin can send a link for an approved record"));
    }
    if r.invitations.iter().any(|i| i.email == email && i.revoked.is_none()) {
        return Err(EngineError::conflict("that address already has a link; send it again instead"));
    }
    if r.invitations.len() >= MAX_INVITATIONS {
        return Err(EngineError::conflict(format!("a record can have at most {MAX_INVITATIONS} links")));
    }
    let number = r.invitations.iter().map(|i| i.number).max().unwrap_or(0).checked_add(1).ok_or_else(|| EngineError::conflict("too many links"))?;
    let stamp = ctx.stamp();
    r.invitations.push(PrequalInvitation { number, email, generation: 0, expires, created: stamp, sent: None, revoked: None });
    r.updated = stamp;
    let id = r.id;
    ctx.state.prequalifications.put(r);
    Ok(json!({ "id": id, "number": number }))
}

fn resend_invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = existing(ctx, p)?;
    let number = invitation_number(p, "number")?;
    let email = p.email("email")?;
    let expires_given = p.present("expires");
    let expires = expiry(ctx, p)?;
    let stamp = ctx.stamp();
    let inv =
        r.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this record")))?;
    if inv.revoked.is_some() {
        return Err(EngineError::conflict("that link was cancelled; send a new one with prequal.invite"));
    }
    if let Some(email) = email {
        inv.email = email;
    }
    if expires_given {
        inv.expires = expires;
    }
    inv.generation = inv.generation.checked_add(1).ok_or_else(|| EngineError::conflict("that link was sent too many times"))?;
    inv.sent = None;
    let generation = inv.generation;
    r.updated = stamp;
    let id = r.id;
    ctx.state.prequalifications.put(r);
    Ok(json!({ "id": id, "number": number, "generation": generation }))
}

fn revoke_invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = existing(ctx, p)?;
    let number = invitation_number(p, "number")?;
    let stamp = ctx.stamp();
    let inv =
        r.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this record")))?;
    if inv.revoked.is_some() {
        return Err(EngineError::conflict("that link is already cancelled"));
    }
    inv.revoked = Some(stamp);
    r.updated = stamp;
    let id = r.id;
    ctx.state.prequalifications.put(r);
    Ok(json!({ "id": id, "number": number }))
}

fn invite_sent(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = existing(ctx, p)?;
    let number = invitation_number(p, "number")?;
    let generation: u32 =
        p.int("generation", 0..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("generation", "is required"))?;
    let stamp = ctx.stamp();
    let inv =
        r.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this record")))?;
    // A link sent again while the first email was on its way stays unsent, so the newer one goes out.
    if inv.generation == generation && inv.revoked.is_none() {
        inv.sent = Some(stamp);
    }
    let id = r.id;
    ctx.state.prequalifications.put(r);
    Ok(json!({ "id": id, "number": number }))
}

fn invitations(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    let today = v.today();
    let list: Vec<Value> = live(v.state, vendor)
        .map(|r| {
            r.invitations
                .iter()
                .map(|i| {
                    json!({
                        "number": i.number, "email": i.email, "generation": i.generation, "expires": i.expires,
                        "created": i.created, "sent": i.sent, "revoked": i.revoked, "active": i.active(today),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Value::Array(list))
}

/// The record and the invitation a checked link names: it must belong to the company, be the generation the server
/// verified, and not be cancelled or past its last day.
fn linked(state: &buildkiosk_model::CompanyState, p: &Params<'_>, today: Date) -> Result<(Prequalification, PrequalInvitation)> {
    let vendor: Id<Vendor> = p.id("vendor").map_err(|_| bad_link())?;
    let number = invitation_number(p, "invitation").map_err(|_| bad_link())?;
    let generation: u32 = p.int("generation", 0..=1_000_000).ok().flatten().and_then(|v| u32::try_from(v).ok()).ok_or_else(bad_link)?;
    let r = live(state, vendor).ok_or_else(bad_link)?;
    let inv = r.invitations.iter().find(|i| i.number == number && i.generation == generation && i.active(today)).ok_or_else(bad_link)?;
    Ok((r.clone(), inv.clone()))
}

/// Whether the company may still change its answers and dates itself.
fn editable(r: &Prequalification) -> bool {
    matches!(r.status, PrequalStatus::Pending | PrequalStatus::Rejected)
}

fn need_editable(r: &Prequalification) -> Result<()> {
    if editable(r) {
        Ok(())
    } else {
        Err(EngineError::conflict(format!("your record is {}; it can't be changed now. Leave a message for the reviewers instead", r.status)))
    }
}

fn portal_view(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let (r, inv) = linked(v.state, p, v.today())?;
    let form = r.form.and_then(|f| v.state.prequal_forms.get(f)).filter(|f| f.deleted.is_none());
    let sections: Vec<Value> = form
        .map(|f| {
            f.sections
                .iter()
                .enumerate()
                .map(|(si, s)| {
                    json!({
                        "title": s.title,
                        "questions": s.questions.iter().enumerate().map(|(qi, q)| {
                            let answer = r.answers.iter().find(|a| usize::try_from(a.section).ok() == Some(si) && usize::try_from(a.question).ok() == Some(qi));
                            json!({ "text": q.text, "answer": answer.map(|a| a.text.clone()).unwrap_or_default() })
                        }).collect::<Vec<_>>(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let files: Vec<Value> = r
        .attachments
        .iter()
        .map(|f| json!({ "name": f.name, "sha256": f.sha256, "size": f.size, "removable": r.portal_files.contains(&f.sha256) && editable(&r) }))
        .collect();
    Ok(json!({
        "company": crate::view::vendor_name(v.state, r.vendor),
        "email": inv.email,
        "status": r.status,
        "editable": editable(&r),
        "valid_until": r.valid_until.filter(|_| r.status == PrequalStatus::Approved),
        "form": form.map(|f| f.name.clone()),
        "sections": sections,
        "insurance_expires": r.insurance_expires,
        "license_expires": r.license_expires,
        "emr": r.emr_hundredths.map(|e| format!("{}.{:02}", e / 100, e % 100)),
        "bonding_capacity": r.bonding_capacity,
        "files": files,
        "message": r.company_message,
    }))
}

/// Tells the Prequalification admins something about a company.
fn tell_admins(ctx: &mut Ctx<'_>, vendor: Id<Vendor>, what: &str) {
    let admins: Vec<Id<buildkiosk_model::directory::Person>> = ctx
        .state
        .people
        .iter()
        .filter(|x| x.is_enabled() && x.is_user && perm::company_level(ctx.state, x.id, Tool::Prequalification) == Level::Admin)
        .map(|x| x.id)
        .collect();
    let name = crate::view::vendor_name(ctx.state, vendor);
    for person in admins {
        let n = ctx.state.notifications.alloc();
        let stamp = ctx.stamp();
        ctx.state.notifications.put(buildkiosk_model::notification::Notification {
            id: n,
            person,
            project: None,
            text: format!("{name}: {what}"),
            link: format!("/prequalification/{vendor}"),
            created: stamp,
            read: false,
        });
    }
}

fn put(ctx: &mut Ctx<'_>, mut r: Prequalification) -> Value {
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.prequalifications.put(r);
    json!({ "id": id })
}

fn portal_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (mut r, _) = linked(ctx.state, p, ctx.today())?;
    need_editable(&r)?;
    for (key, field) in [("insurance_expires", &mut r.insurance_expires), ("license_expires", &mut r.license_expires)] {
        if p.present(key) {
            *field = p.date(key)?;
        }
    }
    if p.present("emr") {
        r.emr_hundredths = match p.line("emr", 10)?.filter(|s| !s.is_empty()) {
            Some(s) => Some(parse_emr(&s).ok_or_else(|| p.bad("emr", format!("`{s}` is not a rate such as 0.85 (up to 9.99)")))?),
            None => None,
        };
    }
    if p.present("bonding_capacity") {
        let m: Option<Money> = p.money("bonding_capacity")?;
        if m.is_some_and(Money::is_negative) {
            return Err(p.bad("bonding_capacity", "must not be negative"));
        }
        r.bonding_capacity = m;
    }
    if p.present("replies") {
        let form = r
            .form
            .and_then(|f| ctx.state.prequal_forms.get(f))
            .cloned()
            .ok_or_else(|| EngineError::conflict("there is no questionnaire to answer"))?;
        for row in p.items("replies")? {
            let (section, question) = cell(&form, &row)?;
            let answer = row.text_or_empty("text", text::LONG)?;
            set_answer(&mut r, section, question, |a| a.text = answer);
        }
    }
    Ok(put(ctx, r))
}

fn portal_submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (mut r, inv) = linked(ctx.state, p, ctx.today())?;
    need_editable(&r)?;
    if let Some(form) = r.form.and_then(|f| ctx.state.prequal_forms.get(f)) {
        for (si, s) in form.sections.iter().enumerate() {
            for (qi, q) in s.questions.iter().enumerate() {
                let answered = r.answers.iter().any(|a| {
                    usize::try_from(a.section).ok() == Some(si) && usize::try_from(a.question).ok() == Some(qi) && !a.text.trim().is_empty()
                });
                if !answered {
                    return Err(EngineError::conflict(format!(
                        "answer every question first; \"{}\" has no answer",
                        q.text.chars().take(80).collect::<String>()
                    )));
                }
            }
        }
    } else if r.insurance_expires.is_none() && r.license_expires.is_none() && r.attachments.is_empty() {
        return Err(EngineError::conflict("give your insurance or licence date, or attach a file, before sending your record"));
    }
    if p.present("message") {
        r.company_message = p.text_or_empty("message", text::LONG)?;
    }
    r.status = PrequalStatus::Submitted;
    r.reviewed = None;
    r.submitted_via = Some(inv.number);
    let vendor = r.vendor;
    let out = put(ctx, r);
    tell_admins(ctx, vendor, "sent its prequalification for review");
    Ok(out)
}

fn file_of(p: &Params<'_>) -> Result<FileRef> {
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    Ok(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime })
}

fn portal_attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (mut r, _) = linked(ctx.state, p, ctx.today())?;
    let file = file_of(p)?;
    if r.attachments.iter().any(|a| a.sha256 == file.sha256) {
        return Ok(json!({ "files": r.attachments.len() }));
    }
    if r.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("the record already has {MAX_ATTACHMENTS} files")));
    }
    let (sha, vendor, approved) = (file.sha256.clone(), r.vendor, r.status == PrequalStatus::Approved);
    r.attachments.push(file);
    r.portal_files.push(sha);
    let n = r.attachments.len();
    put(ctx, r);
    // A file on an approved record is usually a renewal: say so, since nothing else tells the reviewers.
    if approved {
        tell_admins(ctx, vendor, "attached a file to its approved record");
    }
    Ok(json!({ "files": n }))
}

fn portal_detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (mut r, _) = linked(ctx.state, p, ctx.today())?;
    need_editable(&r)?;
    let sha = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if !r.portal_files.contains(&sha) {
        return Err(EngineError::not_found("that file is not one you attached"));
    }
    r.attachments.retain(|a| a.sha256 != sha);
    r.portal_files.retain(|s| *s != sha);
    Ok(put(ctx, r))
}

fn portal_message(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (mut r, _) = linked(ctx.state, p, ctx.today())?;
    let message = p.req_text("message", text::LONG)?;
    r.company_message = message;
    let vendor = r.vendor;
    let out = put(ctx, r);
    tell_admins(ctx, vendor, "left a message on its prequalification");
    Ok(out)
}

/// A warning to email a company's contact: the record's vendor, what it is about, the date and the bucket.
fn company_due(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    let today = v.today();
    let mut out = Vec::new();
    for r in v.state.prequalifications.iter().filter(|r| r.deleted.is_none() && r.status == PrequalStatus::Approved) {
        // The first link that works and was emailed: the one that reaches the company.
        let Some(inv) = r.invitations.iter().find(|i| i.active(today) && i.sent.is_some()) else { continue };
        for (what, date) in [("approval", r.valid_until), ("insurance", r.insurance_expires), ("license", r.license_expires)] {
            let Some(date) = date else { continue };
            let Some(days) = bucket(today.days_until(date)) else { continue };
            if r.company_reminders.iter().any(|x| x.what == what && x.date == date && x.days == days) {
                continue;
            }
            out.push(json!({
                "vendor": r.vendor, "company": crate::view::vendor_name(v.state, r.vendor), "email": inv.email,
                "number": inv.number, "generation": inv.generation, "what": what, "date": date, "days": days,
            }));
        }
    }
    Ok(Value::Array(out))
}

fn company_reminded(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = existing(ctx, p)?;
    let what = p.req_line("what", 20)?;
    if !matches!(what.as_str(), "approval" | "insurance" | "license") {
        return Err(p.bad("what", "must be approval, insurance or license"));
    }
    let date = p.req_date("date")?;
    let days = p.int("days", 0..=60)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("days", "is required"))?;
    if !r.company_reminders.iter().any(|x| x.what == what && x.date == date && x.days == days) {
        r.company_reminders.push(ReminderSent { what, date, days });
        if r.company_reminders.len() > 60 {
            let drop = r.company_reminders.len() - 60;
            r.company_reminders.drain(..drop);
        }
    }
    Ok(put(ctx, r))
}
