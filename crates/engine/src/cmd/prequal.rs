//! Prequalification: a company-level record of whether each company has been vetted, with the
//! dates that make an approval lapse (the approval itself, insurance, licence).
//!
//! Anyone with Standard access to Prequalification can keep a company's record up to date and put
//! it forward; only an admin approves or rejects it. Editing an approved record needs an admin.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::prequal::{PrequalStatus, Prequalification, Standing};
use buildkiosk_model::{Level, Tool};
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
