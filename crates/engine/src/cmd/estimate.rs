//! Estimating: price a project by cost code (quantity × unit cost), add markups, issue it, and
//! put the result into the budget.

use buildkiosk_core::{Id, Money, Percent, text};
use buildkiosk_model::estimate::{Estimate, EstimateLine, EstimateStatus, Markup};
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::{upsert, usable_code};
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "estimate.create",
        "Create estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project: project id; name: text; notes?: text; lines?: [{cost_code: cost code id, description: text, quantity: number (up to 3 decimals), unit?: text, unit_cost: amount}]; markups?: [{name: text, percent: percent of the subtotal}]",
        create,
    ),
    CommandSpec::mutate(
        "estimate.update",
        "Edit estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids; then any estimate.create field (lists replace the old ones; drafts only)",
        update,
    ),
    CommandSpec::mutate(
        "estimate.finalize",
        "Issue estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (needs at least one line)",
        finalize,
    ),
    CommandSpec::mutate("estimate.reopen", "Reopen estimate", Tool::Estimating, Access::Project(Level::Standard), "project, id: ids", reopen),
    CommandSpec::mutate(
        "estimate.apply_to_budget",
        "Put estimate in budget",
        Tool::Estimating,
        Access::Project(Level::Admin),
        "project, id: ids; markup_cost_code?: cost code id (the budget line that holds the markups; without it the markups are left out) (sets each cost code's original budget to the estimate's amount; the budget must be unlocked)",
        apply_to_budget,
    ),
    CommandSpec::mutate("estimate.delete", "Delete estimate", Tool::Estimating, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("estimate.restore", "Restore estimate", Tool::Estimating, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "estimate.list",
        "List estimates",
        Tool::Estimating,
        Access::Project(Level::Read),
        "project: project id; status?: draft|final; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("estimate.get", "Show estimate", Tool::Estimating, Access::Project(Level::Read), "project, id: ids (with its totals)", get),
];

const MAX_LINES: usize = 2_000;
const MAX_MARKUPS: usize = 20;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Estimate> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&ctx.state.estimates, project, id, "estimate")?;
    if e.deleted.is_some() {
        return Err(EngineError::not_found(format!("estimate {id} not found in this project")));
    }
    Ok(e.clone())
}

/// "2.5" → 2500 thousandths; at most three decimals.
pub fn parse_quantity(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', "");
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, s),
    };
    let (whole, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    if (whole.is_empty() && frac.is_empty())
        || frac.len() > 3
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: i64 = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse().ok()? };
    let v = w.checked_mul(1000)?.checked_add(f)?;
    (v <= 1_000_000_000_000).then_some(if neg { -v } else { v })
}

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, e: &mut Estimate) -> Result<()> {
    if p.present("name") {
        e.name = p.req_line("name", text::SHORT)?;
    }
    if p.present("notes") {
        e.notes = p.text_or_empty("notes", text::LONG)?;
    }
    if p.present("lines") {
        let rows = p.items("lines")?;
        if rows.len() > MAX_LINES {
            return Err(p.bad("lines", format!("holds more than {MAX_LINES} lines")));
        }
        let mut out = Vec::new();
        for r in rows {
            let code: Option<Id<CostCode>> = r.opt_id("cost_code")?;
            let description = r.line_or_empty("description", text::SHORT)?;
            let qty_text = r.line_or_empty("quantity", 30)?;
            let cost: Option<Money> = r.money("unit_cost")?;
            if code.is_none() && description.is_empty() && qty_text.is_empty() && cost.is_none() {
                continue; // a spare blank row
            }
            let code = code.ok_or_else(|| r.bad("cost_code", "is required on every line"))?;
            usable_code(ctx.state, e.project, code)?;
            let quantity_milli = if qty_text.is_empty() {
                1000
            } else {
                parse_quantity(&qty_text).ok_or_else(|| r.bad("quantity", format!("`{qty_text}` is not a quantity (up to three decimals)")))?
            };
            let unit_cost = cost.ok_or_else(|| r.bad("unit_cost", "is required on every line"))?;
            let line = EstimateLine { cost_code: code, description, quantity_milli, unit: r.line_or_empty("unit", 20)?, unit_cost };
            if line.total().is_none() {
                return Err(r.bad("unit_cost", "the line total is too large"));
            }
            out.push(line);
        }
        e.lines = out;
    }
    if p.present("markups") {
        let rows = p.items("markups")?;
        if rows.len() > MAX_MARKUPS {
            return Err(p.bad("markups", format!("holds more than {MAX_MARKUPS} markups")));
        }
        let mut out = Vec::new();
        for r in rows {
            let name = r.line_or_empty("name", 100)?;
            let pct: Option<Percent> = r.percent("percent")?;
            if name.is_empty() && pct.is_none() {
                continue;
            }
            if name.is_empty() {
                return Err(r.bad("name", "is required on every markup"));
            }
            out.push(Markup { name, percent: pct.unwrap_or(Percent::ZERO) });
        }
        e.markups = out;
    }
    if e.totals().is_none() {
        return Err(p.bad("lines", "the totals are too large to add up"));
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.estimates.iter().filter(|e| e.project == project).map(|e| e.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut e = Estimate {
        id: ctx.state.estimates.peek_next(),
        project,
        number,
        name: String::new(),
        notes: String::new(),
        status: EstimateStatus::Draft,
        lines: Vec::new(),
        markups: Vec::new(),
        applied: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut e)?;
    if e.name.is_empty() {
        return Err(p.bad("name", "is required"));
    }
    let id = ctx.state.estimates.alloc();
    e.id = id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Draft {
        return Err(EngineError::conflict(format!("{} was issued; reopen it to edit it", e.label())));
    }
    apply(ctx, p, &mut e)?;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn finalize(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already issued", e.label())));
    }
    if e.lines.is_empty() {
        return Err(EngineError::conflict("add at least one line first"));
    }
    e.status = EstimateStatus::Final;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Final {
        return Err(EngineError::conflict(format!("{} is a draft", e.label())));
    }
    e.status = EstimateStatus::Draft;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn apply_to_budget(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Final {
        return Err(EngineError::conflict("issue the estimate before putting it in the budget"));
    }
    if ctx.state.projects.get(e.project).is_some_and(|x| x.budget_locked) {
        return Err(EngineError::conflict("the original budget is locked; unlock it first"));
    }
    let t = e.totals().ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?;
    // Sum by cost code, in code order of first use.
    let mut by_code: Vec<(Id<CostCode>, Money)> = Vec::new();
    for l in &e.lines {
        let amt = l.total().unwrap_or(Money::ZERO);
        match by_code.iter_mut().find(|(c, _)| *c == l.cost_code) {
            Some((_, m)) => *m = m.checked_add(amt).ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?,
            None => by_code.push((l.cost_code, amt)),
        }
    }
    let markup_code: Option<Id<CostCode>> = p.opt_id("markup_cost_code")?;
    if let Some(mc) = markup_code {
        usable_code(ctx.state, e.project, mc)?;
        let extra = t.total.checked_sub(t.subtotal).unwrap_or(Money::ZERO);
        if !extra.is_zero() {
            match by_code.iter_mut().find(|(c, _)| *c == mc) {
                Some((_, m)) => *m = m.checked_add(extra).ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?,
                None => by_code.push((mc, extra)),
            }
        }
    }
    let lines = by_code.len();
    for (code, amount) in by_code {
        upsert(ctx, e.project, code, amount, None);
    }
    e.applied = Some(ctx.stamp());
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id, "budget_lines": lines }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e = load(ctx, p)?;
    let stamp = ctx.stamp();
    ctx.state.estimates.update(e.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": e.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&ctx.state.estimates, project, id, "estimate")?;
    if e.deleted.is_none() {
        return Err(EngineError::conflict("the estimate is not in the recycle bin"));
    }
    ctx.state.estimates.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// The project's estimates, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&Estimate> {
    let mut v: Vec<&Estimate> = state.estimates.iter().filter(|e| e.project == project && e.deleted.is_none()).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Estimating) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Estimate> = v.state.estimates.iter().filter(|e| e.project == project && e.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<EstimateStatus>("status")?;
    let list: Vec<Value> = visible(v.state, project)
        .into_iter()
        .filter(|e| status.is_none_or(|s| e.status == s))
        .map(|e| json!({ "id": e.id, "number": e.number, "name": e.name, "status": e.status, "lines": e.lines.len(), "total": e.totals().map(|t| t.total), "applied": e.applied }))
        .collect();
    Ok(Value::Array(list))
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&v.state.estimates, project, id, "estimate")?;
    if e.deleted.is_some() {
        return Err(EngineError::not_found(format!("estimate {id} not found in this project")));
    }
    let mut out = serde_json::to_value(e).map_err(|x| EngineError::Internal { cmd: "estimate.get".into(), msg: x.to_string() })?;
    if let (Some(o), Some(t)) = (out.as_object_mut(), e.totals()) {
        o.insert("totals".into(), json!({ "subtotal": t.subtotal, "markups": t.markups, "total": t.total }));
    }
    Ok(out)
}
