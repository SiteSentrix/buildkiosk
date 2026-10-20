//! The project budget: one line per cost code with the original amount, and a computed view that
//! sets it against commitments and direct costs.

use buildkiosk_core::{Id, Money, Percent, text};
use buildkiosk_model::finance::{BudgetColumn, BudgetGroup, BudgetLine, BudgetSnapshot, BudgetView, SnapshotRow};
use buildkiosk_model::project::{CostCode, ForecastMethod, Project};

use buildkiosk_model::{Choice, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
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
        "project: project id; text: lines of `cost code, amount[, notes]` (comma or tab separated), or with a first line naming the columns (`Cost code, Description, Amount, Notes`); lines for a code that already has a budget line change it; add_codes?: bool (also create the cost codes the project doesn't have yet, from the Description column; takes a project admin)",
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
        "budget.set_progress",
        "Set percent complete",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; cost_code: cost code id; percent_complete?: percent of the code's work that is done (0 to 100; blank removes it); used by the percent-complete forecasting method",
        set_progress,
    ),
    CommandSpec::mutate(
        "budget.set_method",
        "Choose forecasting method",
        Tool::Budget,
        Access::Project(Level::Admin),
        "project: project id; method: budget (a code with no forecast is assumed to spend its budget) | committed (only what is committed or spent, plus pending change events) | progress (cost so far divided by the percent complete)",
        set_method,
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
    CommandSpec::mutate(
        "budget.view_default",
        "Choose my default budget view",
        Tool::Budget,
        Access::Project(Level::Read),
        "project: project id; id?: budget view id to open by default for you (leave it out to go back to the standard layout)",
        view_default,
    ),
    CommandSpec::mutate(
        "budget.view_save",
        "Save budget view",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; id?: view id (changes that view; without it a new one is made); name: text; columns: list or comma-separated text of original, changes, revised, committed, direct, cost, pending, remaining, percent, forecast, projected, variance (shown in the order of the budget table); group?: none|division",
        view_save,
    ),
    CommandSpec::mutate(
        "budget.view_delete",
        "Delete budget view",
        Tool::Budget,
        Access::Project(Level::Standard),
        "project: project id; id: view id",
        view_delete,
    ),
    CommandSpec::query("budget.views", "List budget views", Tool::Budget, Access::Project(Level::Read), "project: project id", views),
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
                percent_complete: None,
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

/// The columns of a budget import: where the code, description, amount and notes are.
struct ImportColumns {
    code: usize,
    description: Option<usize>,
    amount: usize,
    notes: Option<usize>,
}

impl ImportColumns {
    /// Without a header line: `code, amount, notes`.
    const PLAIN: ImportColumns = ImportColumns { code: 0, description: None, amount: 1, notes: Some(2) };

    /// Reads a header line such as `Cost code, Description, Original budget, Notes`; `None` when the line isn't one.
    fn from_header(cells: &[String]) -> Option<std::result::Result<ImportColumns, &'static str>> {
        let at = |names: &[&str]| cells.iter().position(|c| names.iter().any(|n| c.trim().eq_ignore_ascii_case(n)));
        let code = at(&["code", "cost code", "cost_code"])?;
        let amount = at(&["amount", "original", "original budget", "original amount", "budget"]);
        Some(
            amount
                .map(|amount| ImportColumns {
                    code,
                    description: at(&["description", "name", "cost code description"]),
                    amount,
                    notes: at(&["notes", "note", "comments"]),
                })
                .ok_or("the header line needs an amount column (amount, original or budget)"),
        )
    }
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    const MAX_LINES: usize = 5_000;
    let project: Id<Project> = p.id("project")?;
    unlocked(ctx.state, project)?;
    let body = p.req_text("text", 1_000_000)?;
    let add_codes = p.flag_or("add_codes", false)?;
    if add_codes && ctx.level(project, Tool::ProjectAdmin) < Level::Admin {
        return Err(EngineError::forbidden("adding cost codes takes a project admin"));
    }
    let codes = view::project_cost_codes(ctx.state, project);
    let mut columns = ImportColumns::PLAIN;
    let mut parsed: Vec<(String, Money, String, String)> = Vec::new();
    let mut seen_line = false;
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells: Vec<String> = if line.contains('\t') {
            line.split('\t').map(|c| c.trim().trim_matches('"').to_string()).collect()
        } else {
            super::submittal::csv_cells(line)
        };
        if !seen_line {
            seen_line = true;
            if let Some(header) = ImportColumns::from_header(&cells) {
                columns = header.map_err(|e| p.bad("text", e))?;
                continue;
            }
        }
        let cell = |i: usize| cells.get(i).map(String::as_str).unwrap_or("");
        if parsed.len() >= MAX_LINES {
            return Err(p.bad("text", format!("has more than {MAX_LINES} lines")));
        }
        let code_text = text::required_line(cell(columns.code), 50).map_err(|e| p.bad("text", format!("line {}: cost code {e}", n + 1)))?;
        let money = Money::parse(cell(columns.amount)).map_err(|e| p.bad("text", format!("line {}: {e}", n + 1)))?;
        if money.is_negative() {
            return Err(p.bad("text", format!("line {}: amounts must not be negative", n + 1)));
        }
        let notes =
            text::line(columns.notes.map(&cell).unwrap_or(""), text::SHORT).map_err(|e| p.bad("text", format!("line {}: notes {e}", n + 1)))?;
        let description = text::line(columns.description.map(&cell).unwrap_or(""), text::SHORT)
            .map_err(|e| p.bad("text", format!("line {}: description {e}", n + 1)))?;
        let known = codes.iter().any(|c| c.code.eq_ignore_ascii_case(&code_text));
        if !known && !add_codes {
            return Err(p.bad("text", format!("line {}: no cost code `{code_text}` for this project (tick the box to add new codes)", n + 1)));
        }
        if !known && description.is_empty() && !parsed.iter().any(|(c, _, _, d)| c.eq_ignore_ascii_case(&code_text) && !d.is_empty()) {
            return Err(p.bad("text", format!("line {}: the new cost code `{code_text}` needs a description", n + 1)));
        }
        parsed.push((code_text, money, notes, description));
    }
    let count = parsed.len();
    let mut added = 0usize;
    let stamp = ctx.stamp();
    for (code_text, money, notes, description) in parsed {
        let existing = view::project_cost_codes(ctx.state, project).into_iter().find(|c| c.code.eq_ignore_ascii_case(&code_text)).map(|c| c.id);
        let code = match existing {
            Some(id) => id,
            None => {
                let id = ctx.state.cost_codes.alloc();
                ctx.state.cost_codes.put(CostCode {
                    id,
                    project: Some(project),
                    code: code_text,
                    description,
                    cost_type: None,
                    created: stamp,
                    deleted: None,
                });
                added += 1;
                id
            }
        };
        upsert(ctx, project, code, money, if notes.is_empty() { None } else { Some(notes) });
    }
    Ok(json!({ "lines": count, "created": count, "codes_added": added }))
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
        "percent_complete": r.percent_complete,
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

const MAX_VIEWS: usize = 50;

fn view_columns(p: &Params<'_>) -> Result<Vec<BudgetColumn>> {
    let mut wanted = Vec::new();
    for n in &p.words("columns", 40)? {
        let c = BudgetColumn::parse(n).ok_or_else(|| p.bad("columns", format!("`{n}` is not a budget column")))?;
        if !wanted.contains(&c) {
            wanted.push(c);
        }
    }
    if wanted.is_empty() {
        return Err(p.bad("columns", "needs at least one column"));
    }
    // Always in the order of the table, whatever order they were named in.
    Ok(BudgetColumn::ALL.iter().copied().filter(|c| wanted.contains(c)).collect())
}

fn view_default(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("a default view belongs to a person"))?;
    let chosen: Option<Id<BudgetView>> = p.opt_id("id")?;
    if let Some(id) = chosen {
        let v = in_project(&ctx.state.budget_views, project, id, "budget view")?;
        if v.deleted.is_some() {
            return Err(EngineError::not_found(format!("budget view {id} not found in this project")));
        }
    }
    let all: Vec<Id<BudgetView>> =
        ctx.state.budget_views.iter().filter(|v| v.project == project && v.default_for.contains(&me)).map(|v| v.id).collect();
    for id in all {
        ctx.state.budget_views.update(id, |v| v.default_for.retain(|x| *x != me));
    }
    if let Some(id) = chosen {
        ctx.state.budget_views.update(id, |v| v.default_for.push(me));
    }
    Ok(json!({ "default": chosen }))
}

fn view_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", text::SHORT)?;
    let columns = view_columns(p)?;
    let group: BudgetGroup = p.choice("group")?.unwrap_or_default();
    let stamp = ctx.stamp();
    let id = match p.opt_id::<BudgetView>("id")? {
        Some(id) => {
            let v = in_project(&ctx.state.budget_views, project, id, "budget view")?;
            if v.deleted.is_some() {
                return Err(EngineError::not_found(format!("budget view {id} not found in this project")));
            }
            id
        }
        None => {
            if ctx.state.budget_views.iter().filter(|v| v.project == project && v.deleted.is_none()).count() >= MAX_VIEWS {
                return Err(EngineError::conflict(format!("a project keeps at most {MAX_VIEWS} budget views")));
            }
            let id = ctx.state.budget_views.alloc();
            ctx.state.budget_views.put(BudgetView {
                id,
                project,
                name: name.clone(),
                columns: columns.clone(),
                group,
                default_for: Vec::new(),
                created: stamp,
                updated: stamp,
                deleted: None,
            });
            id
        }
    };
    if ctx.state.budget_views.iter().any(|v| v.project == project && v.deleted.is_none() && v.id != id && v.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already used by another view"));
    }
    ctx.state.budget_views.update(id, |v| {
        v.name = name;
        v.columns = columns;
        v.group = group;
        v.updated = stamp;
    });
    Ok(json!({ "id": id }))
}

fn view_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BudgetView> = p.id("id")?;
    in_project(&ctx.state.budget_views, project, id, "budget view")?;
    let stamp = ctx.stamp();
    ctx.state.budget_views.update(id, |v| v.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn views(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&BudgetView> = v.state.budget_views.iter().filter(|b| b.project == project && b.deleted.is_none()).collect();
    list.sort_by_key(|b| b.id);
    view::json(&list)
}

fn set_progress(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let code: Id<CostCode> = p.id("cost_code")?;
    usable_code(ctx.state, project, code)?;
    let percent = match p.percent("percent_complete")? {
        Some(x) if x.hundredths() < 0 || x > Percent::HUNDRED => return Err(p.bad("percent_complete", "must be between 0 and 100")),
        other => other,
    };
    let found = ctx.state.budget_lines.iter().find(|b| b.project == project && b.cost_code == code && b.deleted.is_none()).map(|b| b.id);
    let id = match found {
        Some(id) => id,
        None => upsert(ctx, project, code, Money::ZERO, None),
    };
    let stamp = ctx.stamp();
    ctx.state.budget_lines.update(id, |b| {
        b.percent_complete = percent;
        b.updated = stamp;
    });
    Ok(json!({ "id": id }))
}

fn set_method(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let method: ForecastMethod = p.choice("method")?.ok_or_else(|| p.bad("method", "is required"))?;
    if !ctx.state.projects.get(project).is_some_and(|x| x.deleted.is_none()) {
        return Err(EngineError::not_found(format!("project {project} not found")));
    }
    ctx.state.projects.update(project, |x| x.forecast_method = method);
    Ok(json!({ "method": method }))
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
            percent_complete: r.percent_complete,
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
