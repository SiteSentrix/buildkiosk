//! The project budget: one line per cost code with the original amount, and a computed view that
//! sets it against commitments and direct costs.

use buildkiosk_core::{Id, Money, text};
use buildkiosk_model::finance::{BudgetLine, BudgetSnapshot, SnapshotRow};
use buildkiosk_model::project::{CostCode, Project};

use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, BudgetRow};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "budget.set_line",
        "Set budget line",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; cost_code: cost code id; original: amount (not negative); notes?: text (creates the line or changes the existing one for that code)",
        set_line,
    ),
    CommandSpec::mutate(
        "budget.delete_line",
        "Delete budget line",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; id: budget line id",
        delete_line,
    ),
    CommandSpec::mutate(
        "budget.import",
        "Import budget",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; text: lines of `cost code, amount[, notes]` (comma or tab separated; a first line starting with `code` is skipped); lines for a code that already has a budget line change it",
        import,
    ),
    CommandSpec::mutate(
        "budget.set_forecast",
        "Set forecast to complete",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; cost_code: cost code id; forecast_to_complete?: amount still to spend (not negative; blank removes the forecast and the budget is assumed to be spent)",
        set_forecast,
    ),
    CommandSpec::mutate(
        "budget.lock",
        "Lock original budget",
        Tool::Budget,
        Access::Project(Level::Admin),
        "project: project id (afterwards only budget changes alter the original budget)",
        lock,
    ),
    CommandSpec::mutate("budget.unlock", "Unlock original budget", Tool::Budget, Access::Project(Level::Admin), "project: project id", unlock),
    CommandSpec::mutate(
        "budget.snapshot",
        "Save budget snapshot",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; name: text; note?: text (keeps today's figures for every cost code)",
        snapshot,
    ),
    CommandSpec::mutate(
        "budget.delete_snapshot",
        "Delete budget snapshot",
        Tool::Budget,
        Access::Project(Level::Admin),
        "project: project id; id: snapshot id",
        delete_snapshot,
    ),
    CommandSpec::query(
        "budget.snapshots",
        "List budget snapshots",
        Tool::Budget,
        Access::Project(Level::Read),
        "project: project id; id?: snapshot id (with it, that snapshot's rows)",
        snapshots,
    ),
    CommandSpec::query("budget.get", "Show budget", Tool::Budget, Access::Project(Level::Read), "project: project id", get),
];

/// A cost code a project may use: its own or a company-wide one.
pub(crate) fn usable_code(ctx_state: &buildkiosk_model::CompanyState, project: Id<Project>, id: Id<CostCode>) -> Result<()> {
    match ctx_state.cost_codes.get(id) {
        Some(c) if c.deleted.is_none() && (c.project.is_none() || c.project == Some(project)) => Ok(()),
        _ => Err(EngineError::not_found(format!("cost code {id} not found for this project"))),
    }
}

/// The original budget can only be edited while it is unlocked.
fn unlocked(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Result<()> {
    if state.projects.get(project).is_some_and(|p| p.budget_locked) {
        return Err(EngineError::conflict("the original budget is locked; change it with a budget change, or unlock it first"));
    }
    Ok(())
}

fn amount(p: &Params<'_>, key: &str) -> Result<Money> {
    let m = p.money(key)?.ok_or_else(|| p.bad(key, "is required"))?;
    if m.is_negative() {
        return Err(p.bad(key, "must not be negative"));
    }
    Ok(m)
}

/// Creates or changes the project's line for `code`.
pub(crate) fn upsert(ctx: &mut Ctx<'_>, project: Id<Project>, code: Id<CostCode>, original: Money, notes: Option<String>) -> Id<BudgetLine> {
    let existing = ctx.state.budget_lines.iter().find(|b| b.project == project && b.cost_code == code && b.deleted.is_none()).map(|b| b.id);
    let stamp = ctx.stamp();
    match existing {
        Some(id) => {
            ctx.state.budget_lines.update(id, |b| {
                if b.original != original || notes.as_ref().is_some_and(|n| *n != b.notes) {
                    b.original = original;
                    if let Some(n) = &notes {
                        b.notes = n.clone();
                    }
                    b.updated = stamp;
                }
            });
            id
        }
        None => {
            let id = ctx.state.budget_lines.alloc();
            ctx.state.budget_lines.put(BudgetLine {
                id,
                project,
                cost_code: code,
                original,
                notes: notes.unwrap_or_default(),
                forecast_to_complete: None,
                created: stamp,
                updated: stamp,
                deleted: None,
            });
            id
        }
    }
}

fn set_line(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    unlocked(ctx.state, project)?;
    let code: Id<CostCode> = p.id("cost_code")?;
    usable_code(ctx.state, project, code)?;
    let original = amount(p, "original")?;
    let notes = if p.present("notes") { Some(p.line_or_empty("notes", text::SHORT)?) } else { None };
    let id = upsert(ctx, project, code, original, notes);
    Ok(json!({ "id": id }))
}

fn delete_line(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    unlocked(ctx.state, project)?;
    let id: Id<BudgetLine> = p.id("id")?;
    in_project(&ctx.state.budget_lines, project, id, "budget line")?;
    let stamp = ctx.stamp();
    ctx.state.budget_lines.update(id, |b| b.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    const MAX_LINES: usize = 5_000;
    let project: Id<Project> = p.id("project")?;
    unlocked(ctx.state, project)?;
    let body = p.req_text("text", 500_000)?;
    let codes = view::project_cost_codes(ctx.state, project);
    let mut parsed: Vec<(Id<CostCode>, Money, String)> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<&str> = if line.contains('\t') { line.split('\t').collect() } else { line.split(',').collect() };
        let code_text = cells.first().map(|c| c.trim().trim_matches('"')).unwrap_or("");
        if parsed.is_empty() && code_text.eq_ignore_ascii_case("code") || code_text.eq_ignore_ascii_case("cost code") {
            continue;
        }
        if parsed.len() >= MAX_LINES {
            return Err(p.bad("text", format!("has more than {MAX_LINES} lines")));
        }
        let code = codes
            .iter()
            .find(|c| c.code.eq_ignore_ascii_case(code_text))
            .ok_or_else(|| p.bad("text", format!("line {}: no cost code `{code_text}` for this project", n + 1)))?;
        let amount_text = cells.get(1).map(|c| c.trim().trim_matches('"')).unwrap_or("");
        let money = Money::parse(amount_text).map_err(|e| p.bad("text", format!("line {}: {e}", n + 1)))?;
        if money.is_negative() {
            return Err(p.bad("text", format!("line {}: amounts must not be negative", n + 1)));
        }
        let notes = text::line(cells.get(2).map(|c| c.trim().trim_matches('"')).unwrap_or(""), text::SHORT)
            .map_err(|e| p.bad("text", format!("line {}: notes {e}", n + 1)))?;
        parsed.push((code.id, money, notes));
    }
    let count = parsed.len();
    for (code, money, notes) in parsed {
        upsert(ctx, project, code, money, if notes.is_empty() { None } else { Some(notes) });
    }
    Ok(json!({ "lines": count }))
}

fn row_json(v: &View<'_>, r: &BudgetRow) -> Value {
    let code = v.state.cost_codes.get(r.cost_code);
    json!({
        "cost_code": r.cost_code,
        "code": code.map(|c| c.code.clone()).unwrap_or_default(),
        "description": code.map(|c| c.description.clone()).unwrap_or_default(),
        "budgeted": r.budgeted,
        "original": r.original,
        "changes": r.changes,
        "revised": r.revised(),
        "committed": r.committed,
        "direct": r.direct,
        "cost": r.cost(),
        "pending": r.pending,
        "remaining": r.remaining(),
        "forecast_to_complete": r.forecast_to_complete,
        "projected": r.projected(),
        "variance": r.variance(),
    })
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let rows = view::budget_rows(v.state, project)?;
    let t = view::budget_totals(&rows)?;
    Ok(json!({
        "rows": rows.iter().map(|r| row_json(v, r)).collect::<Vec<_>>(),
        "totals": { "original": t.original, "changes": t.changes, "revised": t.revised(), "committed": t.committed, "direct": t.direct, "cost": t.cost(), "pending": t.pending, "remaining": t.remaining(), "projected": t.projected, "variance": t.variance() },
        "locked": v.state.projects.get(project).is_some_and(|p| p.budget_locked),
    }))
}

fn set_forecast(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let code: Id<CostCode> = p.id("cost_code")?;
    usable_code(ctx.state, project, code)?;
    let forecast = match p.money("forecast_to_complete")? {
        Some(m) if m.is_negative() => return Err(p.bad("forecast_to_complete", "must not be negative")),
        other => other,
    };
    // The forecast lives on the code's budget line; a code with costs but no line gets one at zero.
    let found = ctx.state.budget_lines.iter().find(|b| b.project == project && b.cost_code == code && b.deleted.is_none()).map(|b| b.id);
    let id = match found {
        Some(id) => id,
        None => upsert(ctx, project, code, Money::ZERO, None),
    };
    let stamp = ctx.stamp();
    ctx.state.budget_lines.update(id, |b| {
        b.forecast_to_complete = forecast;
        b.updated = stamp;
    });
    Ok(json!({ "id": id }))
}

fn set_locked(ctx: &mut Ctx<'_>, p: &Params<'_>, locked: bool) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let current = ctx.state.projects.get(project).ok_or_else(|| EngineError::not_found(format!("project {project} not found")))?.budget_locked;
    if current == locked {
        return Err(EngineError::conflict(if locked { "the budget is already locked" } else { "the budget is not locked" }));
    }
    ctx.state.projects.update(project, |x| x.budget_locked = locked);
    Ok(json!({ "locked": locked }))
}

fn lock(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    set_locked(ctx, p, true)
}

fn unlock(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    set_locked(ctx, p, false)
}

const MAX_SNAPSHOTS: usize = 200;

fn snapshot(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", text::SHORT)?;
    if ctx.state.budget_snapshots.iter().filter(|s| s.project == project && s.deleted.is_none()).count() >= MAX_SNAPSHOTS {
        return Err(EngineError::conflict(format!("a project keeps at most {MAX_SNAPSHOTS} snapshots")));
    }
    let rows = view::budget_rows(ctx.state, project)?
        .into_iter()
        .map(|r| SnapshotRow {
            cost_code: r.cost_code,
            original: r.original,
            changes: r.changes,
            committed: r.committed,
            direct: r.direct,
            pending: r.pending,
            forecast_to_complete: r.forecast_to_complete,
        })
        .collect();
    let id = ctx.state.budget_snapshots.alloc();
    let created = ctx.stamp();
    ctx.state.budget_snapshots.put(BudgetSnapshot { id, project, name, note: p.text_or_empty("note", text::SHORT)?, rows, created, deleted: None });
    Ok(json!({ "id": id }))
}

fn delete_snapshot(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BudgetSnapshot> = p.id("id")?;
    in_project(&ctx.state.budget_snapshots, project, id, "snapshot")?;
    let stamp = ctx.stamp();
    ctx.state.budget_snapshots.update(id, |s| s.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn snapshots(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if let Some(id) = p.opt_id::<BudgetSnapshot>("id")? {
        let s = in_project(&v.state.budget_snapshots, project, id, "snapshot")?;
        if s.deleted.is_some() {
            return Err(EngineError::not_found(format!("snapshot {id} not found in this project")));
        }
        return view::json(s);
    }
    let list: Vec<Value> = v
        .state
        .budget_snapshots
        .iter()
        .filter(|s| s.project == project && s.deleted.is_none())
        .map(|s| json!({ "id": s.id, "name": s.name, "note": s.note, "created": s.created }))
        .collect();
    Ok(Value::Array(list))
}
