//! Prequalification: a company-level record of whether each company has been vetted, with the
//! dates that make an approval lapse (the approval itself, insurance, licence).
//!
//! Anyone with Standard access to Prequalification can keep a company's record up to date, record the
//! company's answers to a questionnaire, attach certificates and put it forward; only an admin writes the
//! questionnaires, awards points (which make the score), approves or rejects. Editing an approved record
//! needs an admin. A daily pass warns the admins before anything lapses, and the company can require a
//! current prequalification before a commitment is approved or a bid awarded.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::prequal::{
    PrequalAnswer, PrequalForm, PrequalQuestion, PrequalSection, PrequalStatus, Prequalification, ReminderSent, Standing,
};
use buildkiosk_model::{Level, Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "prequal.save",
        "Save prequalification",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; valid_until?, insurance_expires?, license_expires?: YYYY-MM-DD; emr?: experience modification rate such as 0.85; bonding_capacity?: amount; notes?: text (creates the company's record or changes it; blank clears a field)",
        save,
    ),
    CommandSpec::mutate(
        "prequal.submit",
        "Put forward for review",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id",
        submit,
    ),
    CommandSpec::mutate(
        "prequal.review",
        "Approve or reject",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "vendor: company id; decision: approved|rejected; score?: 0-100; valid_until?: YYYY-MM-DD; notes?: text",
        review,
    ),
    CommandSpec::mutate(
        "prequal.form_save",
        "Save questionnaire",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "id?: questionnaire id (changes it; without it a new one is made); name: text; text: lines, `# Section title` then `Question | points` under it (points default 10); or sections: [{title, questions: [{text, points}]}]",
        form_save,
    ),
    CommandSpec::mutate(
        "prequal.form_delete",
        "Delete questionnaire",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "id: questionnaire id",
        form_delete,
    ),
    CommandSpec::query("prequal.forms", "List questionnaires", Tool::Prequalification, Access::Company(Level::Read), "(none)", forms),
    CommandSpec::mutate(
        "prequal.answer",
        "Record questionnaire answers",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; form?: questionnaire id (choosing a different one clears the answers); replies?: [{section, question: numbers from 0, text}] (the company's answers to questions; blank clears one)",
        answer,
    ),
    CommandSpec::mutate(
        "prequal.grade",
        "Score questionnaire answers",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "vendor: company id; points: [{section, question: numbers from 0, points}] (no more than the question is worth; blank clears). The score becomes the share of the questionnaire's points earned",
        grade,
    ),
    CommandSpec::mutate(
        "prequal.attach",
        "Attach file to prequalification",
        Tool::Prequalification,
        Access::Company(Level::Standard),
        "vendor: company id; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type (insurance certificate, financial statement)",
        attach,
    ),
    CommandSpec::mutate(
        "prequal.detach",
        "Remove file from prequalification",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "vendor: company id; sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "prequal.require",
        "Require prequalification",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "required: bool (when on, a commitment can only be approved, and a bid awarded, to a company whose prequalification is approved and not lapsed)",
        require,
    ),
    CommandSpec::mutate(
        "prequal.remind",
        "Warn before approvals lapse",
        Tool::Prequalification,
        Access::System,
        "(none) (tells the Prequalification admins, once for each, when an approval, insurance or licence is 60, 30 or 7 days from lapsing, or has lapsed; the server runs it daily)",
        remind,
    ),
    CommandSpec::mutate("prequal.reset", "Reset to pending", Tool::Prequalification, Access::Company(Level::Admin), "vendor: company id", reset),
    CommandSpec::mutate(
        "prequal.delete",
        "Delete prequalification",
        Tool::Prequalification,
        Access::Company(Level::Admin),
        "vendor: company id",
        delete,
    ),
    CommandSpec::query(
        "prequal.list",
        "List prequalifications",
        Tool::Prequalification,
        Access::Company(Level::Read),
        "status?: pending|submitted|approved|rejected; attention?: bool (approved records that lapse within 60 days or already have); q?: text",
        list,
    ),
    CommandSpec::query("prequal.get", "Show prequalification", Tool::Prequalification, Access::Company(Level::Read), "vendor: company id", get),
];

fn live(state: &buildkiosk_model::CompanyState, vendor: Id<Vendor>) -> Option<&Prequalification> {
    state.prequalifications.iter().find(|r| r.vendor == vendor && r.deleted.is_none())
}

fn record(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Prequalification> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    live(ctx.state, vendor).cloned().ok_or_else(|| EngineError::not_found("that company has no prequalification record yet"))
}

/// "0.85" → 85; between 0 and 9.99.
fn parse_emr(s: &str) -> Option<u32> {
    let s = s.trim();
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    if (whole.is_empty() && frac.is_empty())
        || frac.len() > 2
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: u32 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: u32 = if frac.is_empty() { 0 } else { format!("{frac:0<2}").parse().ok()? };
    let v = w.checked_mul(100)?.checked_add(f)?;
    (v <= 999).then_some(v)
}

fn save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    let admin = ctx.company_level(Tool::Prequalification) == Level::Admin;
    let mut r = match live(ctx.state, vendor).cloned() {
        Some(r) => {
            if r.status == PrequalStatus::Approved && !admin {
                return Err(EngineError::forbidden("only an admin can change an approved record"));
            }
            r
        }
        None => {
            let stamp = ctx.stamp();
            Prequalification {
                id: ctx.state.prequalifications.peek_next(),
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
                reviewed: None,
                created: stamp,
                updated: stamp,
                deleted: None,
            }
        }
    };
    for (key, field) in
        [("valid_until", &mut r.valid_until), ("insurance_expires", &mut r.insurance_expires), ("license_expires", &mut r.license_expires)]
    {
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
    if p.present("notes") {
        r.notes = p.text_or_empty("notes", text::LONG)?;
    }
    r.updated = ctx.stamp();
    let new = live(ctx.state, vendor).is_none();
    let id = if new { ctx.state.prequalifications.alloc() } else { r.id };
    r.id = id;
    ctx.state.prequalifications.put(r);
    Ok(json!({ "id": id }))
}

fn change(ctx: &mut Ctx<'_>, mut r: Prequalification) -> Value {
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.prequalifications.put(r);
    json!({ "id": id })
}

fn submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    if r.status != PrequalStatus::Pending {
        return Err(EngineError::conflict(format!("the record is {}", r.status)));
    }
    r.status = PrequalStatus::Submitted;
    Ok(change(ctx, r))
}

fn review(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    let decision = p
        .choice::<PrequalStatus>("decision")?
        .filter(|d| matches!(d, PrequalStatus::Approved | PrequalStatus::Rejected))
        .ok_or_else(|| p.bad("decision", "must be approved or rejected"))?;
    if p.present("score") {
        r.score = p.int("score", 0..=100)?.and_then(|v| u8::try_from(v).ok());
    }
    if p.present("valid_until") {
        r.valid_until = p.date("valid_until")?;
    }
    if p.present("notes") {
        r.notes = p.text_or_empty("notes", text::LONG)?;
    }
    if decision == PrequalStatus::Approved && r.valid_until.is_some_and(|d| d < ctx.today()) {
        return Err(p.bad("valid_until", "is already past"));
    }
    r.status = decision;
    r.reviewed = Some(ctx.stamp());
    Ok(change(ctx, r))
}

fn reset(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    r.status = PrequalStatus::Pending;
    r.reviewed = None;
    Ok(change(ctx, r))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let r = record(ctx, p)?;
    let stamp = ctx.stamp();
    ctx.state.prequalifications.update(r.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": r.id }))
}

fn standing_json(s: Standing) -> &'static str {
    match s {
        Standing::Current => "current",
        Standing::ExpiringSoon => "expiring_soon",
        Standing::Lapsed => "lapsed",
        Standing::NotApproved => "not_approved",
    }
}

/// The records, by company name.
pub fn visible(state: &buildkiosk_model::CompanyState) -> Vec<&Prequalification> {
    let mut v: Vec<&Prequalification> = state.prequalifications.iter().filter(|r| r.deleted.is_none()).collect();
    v.sort_by_key(|r| view_name(state, r.vendor).to_lowercase());
    v
}

fn view_name(state: &buildkiosk_model::CompanyState, v: Id<Vendor>) -> String {
    crate::view::vendor_name(state, v)
}

fn row(state: &buildkiosk_model::CompanyState, r: &Prequalification, today: Date) -> Value {
    let mut out = serde_json::to_value(r).unwrap_or(Value::Null);
    if let Some(o) = out.as_object_mut() {
        o.insert("company".into(), json!(view_name(state, r.vendor)));
        o.insert("standing".into(), json!(standing_json(r.standing(today))));
    }
    out
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let status = p.choice::<PrequalStatus>("status")?;
    let attention = p.flag_or("attention", false)?;
    let q = p.line("q", 100)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let today = v.today();
    let list: Vec<Value> = visible(v.state)
        .into_iter()
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| !attention || matches!(r.standing(today), Standing::ExpiringSoon | Standing::Lapsed))
        .filter(|r| q.as_ref().is_none_or(|q| view_name(v.state, r.vendor).to_lowercase().contains(q.as_str())))
        .map(|r| row(v.state, r, today))
        .collect();
    Ok(Value::Array(list))
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let vendor: Id<Vendor> = p.id("vendor")?;
    let r = live(v.state, vendor).ok_or_else(|| EngineError::not_found("that company has no prequalification record"))?;
    Ok(row(v.state, r, v.today()))
}

const MAX_SECTIONS: usize = 30;
const MAX_QUESTIONS: usize = 60;
const MAX_FORMS: usize = 50;
const MAX_ATTACHMENTS: usize = 50;

fn parse_form_text(p: &Params<'_>, body: &str) -> Result<Vec<PrequalSection>> {
    let mut sections: Vec<PrequalSection> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(title) = line.strip_prefix('#') {
            let title = text::required_line(title.trim(), text::SHORT).map_err(|e| p.bad("text", format!("line {}: section title {e}", n + 1)))?;
            sections.push(PrequalSection { title, questions: Vec::new() });
            continue;
        }
        let (q, points) = line.rsplit_once('|').map_or((line, None), |(q, pts)| (q, Some(pts.trim())));
        let max_points = match points {
            Some(pts) => pts
                .parse::<u32>()
                .ok()
                .filter(|v| (1..=1000).contains(v))
                .ok_or_else(|| p.bad("text", format!("line {}: points must be a whole number from 1 to 1000", n + 1)))?,
            None => 10,
        };
        let text = text::required_line(q.trim(), text::SHORT).map_err(|e| p.bad("text", format!("line {}: question {e}", n + 1)))?;
        let Some(section) = sections.last_mut() else {
            return Err(p.bad("text", format!("line {}: start with a `# Section title` line", n + 1)));
        };
        section.questions.push(PrequalQuestion { text, max_points });
    }
    Ok(sections)
}

fn parse_form_sections(p: &Params<'_>) -> Result<Vec<PrequalSection>> {
    let mut out = Vec::new();
    for s in p.items("sections")? {
        let title = s.req_line("title", text::SHORT)?;
        let mut questions = Vec::new();
        for q in s.items("questions")? {
            let text = q.req_line("text", text::SHORT)?;
            let max_points = q.int("points", 1..=1000)?.and_then(|v| u32::try_from(v).ok()).unwrap_or(10);
            questions.push(PrequalQuestion { text, max_points });
        }
        out.push(PrequalSection { title, questions });
    }
    Ok(out)
}

fn form_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let name = p.req_line("name", text::SHORT)?;
    let sections = match p.text("text", 50_000)? {
        Some(body) if !body.trim().is_empty() => parse_form_text(p, &body)?,
        _ => parse_form_sections(p)?,
    };
    if sections.is_empty() || sections.iter().any(|s| s.questions.is_empty()) {
        return Err(p.bad("text", "needs at least one section, and every section at least one question"));
    }
    if sections.len() > MAX_SECTIONS || sections.iter().any(|s| s.questions.len() > MAX_QUESTIONS) {
        return Err(p.bad("text", format!("has more than {MAX_SECTIONS} sections or {MAX_QUESTIONS} questions in one")));
    }
    let stamp = ctx.stamp();
    let existing = p.opt_id::<PrequalForm>("id")?;
    if ctx.state.prequal_forms.iter().any(|f| f.deleted.is_none() && Some(f.id) != existing && f.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already used by another questionnaire"));
    }
    let id = match existing {
        Some(id) => {
            if !ctx.state.prequal_forms.get(id).is_some_and(|f| f.deleted.is_none()) {
                return Err(EngineError::not_found(format!("questionnaire {id} not found")));
            }
            ctx.state.prequal_forms.update(id, |f| {
                f.name = name;
                f.sections = sections;
                f.updated = stamp;
            });
            id
        }
        None => {
            if ctx.state.prequal_forms.iter().filter(|f| f.deleted.is_none()).count() >= MAX_FORMS {
                return Err(EngineError::conflict(format!("at most {MAX_FORMS} questionnaires")));
            }
            let id = ctx.state.prequal_forms.alloc();
            ctx.state.prequal_forms.put(PrequalForm { id, name, sections, created: stamp, updated: stamp, deleted: None });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn form_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<PrequalForm> = p.id("id")?;
    if !ctx.state.prequal_forms.get(id).is_some_and(|f| f.deleted.is_none()) {
        return Err(EngineError::not_found(format!("questionnaire {id} not found")));
    }
    if ctx.state.prequalifications.iter().any(|r| r.deleted.is_none() && r.form == Some(id)) {
        return Err(EngineError::conflict("companies are answering this questionnaire; change theirs first"));
    }
    let stamp = ctx.stamp();
    ctx.state.prequal_forms.update(id, |f| f.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// The questionnaires, by name.
pub fn forms_list(state: &buildkiosk_model::CompanyState) -> Vec<&PrequalForm> {
    let mut v: Vec<&PrequalForm> = state.prequal_forms.iter().filter(|f| f.deleted.is_none()).collect();
    v.sort_by_key(|f| f.name.to_lowercase());
    v
}

fn forms(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    crate::view::json(&forms_list(v.state))
}

/// A questionnaire cell: section and question numbers that exist on the form.
fn cell(form: &PrequalForm, row: &Params<'_>) -> Result<(u32, u32)> {
    let section = row.int("section", 0..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| row.bad("section", "is required"))?;
    let question = row.int("question", 0..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| row.bad("question", "is required"))?;
    let exists = form
        .sections
        .get(usize::try_from(section).unwrap_or(usize::MAX))
        .and_then(|s| s.questions.get(usize::try_from(question).unwrap_or(usize::MAX)))
        .is_some();
    if !exists {
        return Err(row.bad("question", format!("section {section}, question {question} is not on the questionnaire")));
    }
    Ok((section, question))
}

fn set_answer(r: &mut Prequalification, section: u32, question: u32, f: impl FnOnce(&mut PrequalAnswer)) {
    let at = r.answers.iter().position(|a| a.section == section && a.question == question);
    let i = at.unwrap_or_else(|| {
        r.answers.push(PrequalAnswer { section, question, text: String::new(), points: None });
        r.answers.len() - 1
    });
    if let Some(a) = r.answers.get_mut(i) {
        f(a);
    }
    r.answers.retain(|a| !a.text.is_empty() || a.points.is_some());
    r.answers.sort_by_key(|a| (a.section, a.question));
}

fn answer(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    if r.status == PrequalStatus::Approved && ctx.company_level(Tool::Prequalification) != Level::Admin {
        return Err(EngineError::forbidden("only an admin can change an approved record"));
    }
    if let Some(f) = p.opt_id::<PrequalForm>("form")? {
        if !ctx.state.prequal_forms.get(f).is_some_and(|x| x.deleted.is_none()) {
            return Err(EngineError::not_found(format!("questionnaire {f} not found")));
        }
        if r.form != Some(f) {
            r.form = Some(f);
            r.answers.clear();
        }
    }
    let form = r.form.and_then(|f| ctx.state.prequal_forms.get(f)).cloned().ok_or_else(|| EngineError::conflict("choose a questionnaire first"))?;
    for row in p.items("replies")? {
        let (section, question) = cell(&form, &row)?;
        let answer = row.text_or_empty("text", text::LONG)?;
        set_answer(&mut r, section, question, |a| a.text = answer);
    }
    Ok(change(ctx, r))
}

fn grade(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    let form = r
        .form
        .and_then(|f| ctx.state.prequal_forms.get(f))
        .cloned()
        .ok_or_else(|| EngineError::conflict("the company has no questionnaire to score"))?;
    for row in p.items("points")? {
        let (section, question) = cell(&form, &row)?;
        let max = form.sections.get(section as usize).and_then(|s| s.questions.get(question as usize)).map_or(0, |q| q.max_points);
        let points = match row.line("points", 10)?.filter(|s| !s.is_empty()) {
            Some(s) => Some(
                s.parse::<u32>().ok().filter(|v| *v <= max).ok_or_else(|| row.bad("points", format!("must be a whole number from 0 to {max}")))?,
            ),
            None => None,
        };
        set_answer(&mut r, section, question, |a| a.points = points);
    }
    let (earned, possible) = r.points(&form, None);
    r.score = (possible > 0).then(|| u8::try_from((u64::from(earned) * 100 + u64::from(possible) / 2) / u64::from(possible)).unwrap_or(100));
    let score = r.score;
    change(ctx, r);
    Ok(json!({ "earned": earned, "possible": possible, "score": score }))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    if r.status == PrequalStatus::Approved && ctx.company_level(Tool::Prequalification) != Level::Admin {
        return Err(EngineError::forbidden("only an admin can change an approved record"));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if r.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": r.id }));
    }
    if r.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("the record already has {MAX_ATTACHMENTS} files")));
    }
    r.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    Ok(change(ctx, r))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = record(ctx, p)?;
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = r.attachments.len();
    r.attachments.retain(|a| a.sha256 != sha256);
    if r.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this record"));
    }
    Ok(change(ctx, r))
}

fn require(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let required = p.flag("required")?.ok_or_else(|| p.bad("required", "is required"))?;
    ctx.state.company.update(buildkiosk_model::directory::Company::ID, |c| c.require_prequalification = required);
    Ok(json!({ "required": required }))
}

/// How a company stands today: not approved when it has no record.
pub fn vendor_standing(state: &buildkiosk_model::CompanyState, vendor: Id<Vendor>, today: Date) -> Standing {
    live(state, vendor).map_or(Standing::NotApproved, |r| r.standing(today))
}

/// Refuses a company that isn't prequalified, when the company requires it (used before approving a commitment
/// or awarding a bid).
pub fn check_prequalified(ctx: &Ctx<'_>, vendor: Id<Vendor>) -> Result<()> {
    if !ctx.state.company().is_some_and(|c| c.require_prequalification) {
        return Ok(());
    }
    let name = crate::view::vendor_name(ctx.state, vendor);
    match vendor_standing(ctx.state, vendor, ctx.today()) {
        Standing::Current | Standing::ExpiringSoon => Ok(()),
        Standing::Lapsed => {
            Err(EngineError::conflict(format!("{name}'s prequalification has lapsed; renew it first (this company requires a current one)")))
        }
        Standing::NotApproved => {
            Err(EngineError::conflict(format!("{name} is not prequalified (this company requires an approved prequalification)")))
        }
    }
}

/// The reminder bucket for a date: 60, 30 or 7 days ahead, or 0 once it has lapsed; none while it is further off.
fn bucket(days_left: i64) -> Option<u32> {
    match days_left {
        d if d < 0 => Some(0),
        0..=7 => Some(7),
        8..=30 => Some(30),
        31..=60 => Some(60),
        _ => None,
    }
}

fn remind(ctx: &mut Ctx<'_>, _p: &Params<'_>) -> Result<Value> {
    let today = ctx.today();
    let admins: Vec<Id<buildkiosk_model::directory::Person>> = ctx
        .state
        .people
        .iter()
        .filter(|x| x.is_enabled() && x.is_user && perm::company_level(ctx.state, x.id, Tool::Prequalification) == Level::Admin)
        .map(|x| x.id)
        .collect();
    /// A warning to send: the record, company, what it is about, the date and the bucket.
    type Warning = (Id<Prequalification>, Id<Vendor>, &'static str, Date, u32);
    let due: Vec<Warning> = ctx
        .state
        .prequalifications
        .iter()
        .filter(|r| r.deleted.is_none() && r.status == PrequalStatus::Approved)
        .flat_map(|r| {
            [("approval", r.valid_until), ("insurance", r.insurance_expires), ("license", r.license_expires)].into_iter().filter_map(
                move |(what, d)| {
                    let d = d?;
                    let b = bucket(today.days_until(d))?;
                    (!r.reminders.iter().any(|x| x.what == what && x.date == d && x.days == b)).then_some((r.id, r.vendor, what, d, b))
                },
            )
        })
        .collect();
    let mut sent = 0usize;
    for (id, vendor, what, date, b) in due {
        let name = crate::view::vendor_name(ctx.state, vendor);
        let when = if b == 0 { format!("lapsed on {}", date.pretty()) } else { format!("lapses on {}", date.pretty()) };
        let what_text = match what {
            "approval" => "prequalification approval",
            "insurance" => "insurance",
            _ => "license",
        };
        for person in &admins {
            let n = ctx.state.notifications.alloc();
            let stamp = ctx.stamp();
            ctx.state.notifications.put(buildkiosk_model::notification::Notification {
                id: n,
                person: *person,
                project: None,
                text: format!("{name}: {what_text} {when}"),
                link: format!("/prequalification/{vendor}"),
                created: stamp,
                read: false,
            });
        }
        ctx.state.prequalifications.update(id, |r| {
            r.reminders.push(ReminderSent { what: what.to_string(), date, days: b });
            // Keep the list short: only the newest are needed to avoid repeating.
            if r.reminders.len() > 60 {
                let drop = r.reminders.len() - 60;
                r.reminders.drain(..drop);
            }
        });
        sent += 1;
    }
    Ok(json!({ "reminded": sent }))
}
