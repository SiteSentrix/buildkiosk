//! Cost pages: the budget, commitments and direct costs.

use buildkiosk_core::{Id, Money};
use buildkiosk_engine::cmd::commitment::can_edit;
use buildkiosk_engine::model::finance::{BudgetColumn, BudgetGroup, BudgetView, Commitment, CommitmentKind, CommitmentStatus, SignatureStatus};
use buildkiosk_engine::model::project::{CostType, ForecastMethod, Project};
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, not_found, project_for};

fn crumbs<'a>(p: &'a Project, title: &'a str, list: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match list {
        Some((name, href)) => {
            c.push((name, Some(href)));
            c.push((title, None));
        }
        None => c.push((title, None)),
    }
    c
}

/// An amount cell: right-aligned, red when negative.
fn amount(m: Money) -> Markup {
    html! { span class=(if m.is_negative() { "overdue" } else { "" }) { (m) } }
}

fn code_options(req: &Req<'_>, project: Id<Project>, selected: Option<Id<buildkiosk_engine::model::project::CostCode>>, blank: bool) -> Markup {
    let codes = view::project_cost_codes(req.engine.state(), project);
    html! {
        @if blank { option value="" { "—" } }
        @for c in codes { option value=(c.id) selected[Some(c.id) == selected] { (c.code) " " (c.description) } }
    }
}

// -- budget

/// The columns and grouping a budget is shown with: the chosen saved view, else the standard layout (which adds
/// the percent-complete column when the project forecasts by progress).
fn budget_layout(
    state: &buildkiosk_engine::model::CompanyState,
    id: Id<Project>,
    chosen: Option<Id<BudgetView>>,
) -> (Option<&BudgetView>, Vec<BudgetColumn>, BudgetGroup) {
    let saved = chosen.and_then(|v| state.budget_views.get(v)).filter(|v| v.project == id && v.deleted.is_none());
    match saved {
        Some(v) => (Some(v), v.columns.clone(), v.group),
        None => {
            let progress = state.projects.get(id).is_some_and(|p| p.forecast_method == ForecastMethod::Progress);
            let columns = BudgetColumn::DEFAULT
                .iter()
                .copied()
                .flat_map(|c| if progress && c == BudgetColumn::Remaining { vec![c, BudgetColumn::Percent] } else { vec![c] })
                .collect();
            (None, columns, BudgetGroup::None)
        }
    }
}

/// One cell of the budget table.
fn budget_cell(row: &view::BudgetRow, column: BudgetColumn) -> Markup {
    match column {
        BudgetColumn::Percent => html! { td class="num" { @if let Some(x) = row.percent_complete { (x) "%" } @else { span class="muted" { "—" } } } },
        BudgetColumn::Forecast => {
            html! { td class="num" { @if let Some(x) = row.forecast_to_complete { (x) } @else { span class="muted" { "—" } } } }
        }
        BudgetColumn::Remaining | BudgetColumn::Variance => {
            html! { td class="num" { (amount(view::column_amount(row, column).unwrap_or_default())) } }
        }
        _ => html! { td class="num" { (view::column_amount(row, column).unwrap_or_default()) } },
    }
}

/// One cell of a total or subtotal row (blank where a total makes no sense).
fn total_cell(t: &view::BudgetTotals, column: BudgetColumn) -> Markup {
    match view::total_amount(t, column) {
        Some(m) if matches!(column, BudgetColumn::Remaining | BudgetColumn::Variance) => html! { td class="num" { (amount(m)) } },
        Some(m) => html! { td class="num" { (m) } },
        None => html! { td {} },
    }
}

pub fn budget(
    req: &Req<'_>,
    id: Id<Project>,
    compare: Option<Id<buildkiosk_engine::model::finance::BudgetSnapshot>>,
    chosen: Option<Id<BudgetView>>,
) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Budget) < Level::Read {
        return not_found(req);
    }
    let rows = view::budget_rows(state, id).unwrap_or_default();
    let totals = view::budget_totals(&rows).ok();
    let here = format!("/projects/{id}/budget");
    let locked = p.budget_locked;
    let can_forecast = req.allowed("budget.set_forecast", Some(id));
    let can_edit = req.allowed("budget.set_line", Some(id)) && !locked;
    let can_lock = req.allowed("budget.lock", Some(id));
    let mut snapshots: Vec<_> = state.budget_snapshots.iter().filter(|s| s.project == id && s.deleted.is_none()).collect();
    snapshots.sort_by_key(|s| std::cmp::Reverse(s.id));
    let compared = compare.and_then(|c| snapshots.iter().copied().find(|s| s.id == c));
    let (current, columns, group) = budget_layout(state, id, chosen);
    let mut saved_views: Vec<&BudgetView> = state.budget_views.iter().filter(|v| v.project == id && v.deleted.is_none()).collect();
    saved_views.sort_by_key(|v| v.id);
    let can_views = req.allowed("budget.view_save", Some(id));
    let method = p.forecast_method;
    let line_of = |code| state.budget_lines.iter().find(|b| b.project == id && b.cost_code == code && b.deleted.is_none()).map(|b| b.id);
    let body = html! {
        div class="page-head" {
            div { h1 { "Budget" } div class="sub" { "What each cost code was budgeted, what is committed and what has been spent" } }
            div class="actions" {
                a class="btn" href=(format!("{here}.pdf")) { "PDF" }
                a class="btn" href=(format!("/projects/{id}/budget-changes")) { "Budget changes" }
                a class="btn" href=(format!("{here}.csv")) { (Icon::Download.svg()) "Export CSV" }
            }
        }
        @if let Some(t) = totals {
            div class="stat-row" {
                div class="stat" { div class="label" { "Revised budget" } div class="value" { (t.revised()) } @if !t.changes.is_zero() { div class="muted small" { "Original " (t.original) } } }
                div class="stat" { div class="label" { "Committed" } div class="value" { (t.committed) } }
                div class="stat" { div class="label" { "Direct costs" } div class="value" { (t.direct) } }
                div class=(if t.remaining().is_negative() { "stat alert" } else { "stat good" }) { div class="label" { "Remaining" } div class="value" { (t.remaining()) } }
                div class="stat" { div class="label" { "Projected cost" } div class="value" { (t.projected) } }
                div class=(if t.variance().is_negative() { "stat alert" } else { "stat good" }) { div class="label" { (if t.variance().is_negative() { "Projected overrun" } else { "Projected under budget" }) } div class="value" { (if t.variance().is_negative() { buildkiosk_core::Money::ZERO.checked_sub(t.variance()).unwrap_or(buildkiosk_core::Money::ZERO) } else { t.variance() }) } }
                @if !t.pending.is_zero() { a class="stat" href=(format!("/projects/{id}/change-events")) { div class="label" { "Pending change events" } div class="value" { (t.pending) } } }
            }
        }
        @if locked { div class="flash ok" role="status" { "The original budget is locked. Change it with a budget change; forecasts can still be edited." } }
        @if !saved_views.is_empty() || can_views {
            div class="tabs" {
                a href=(here.clone()) class=(if current.is_none() { "active" } else { "" }) { "Standard" }
                @for v in &saved_views { a href=(format!("{here}?view={}", v.id)) class=(if current.is_some_and(|c| c.id == v.id) { "active" } else { "" }) { (v.name) } }
            }
        }
        @if rows.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Budget), "No budget yet", "Add a line for a cost code, or import a list below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Cost code" } @for c in &columns { th class="num" { (c.label()) } } @if can_edit { th {} } } }
                    @let groups: Vec<(Option<String>, Vec<&view::BudgetRow>)> = {
                        let code_of = |r: &view::BudgetRow| state.cost_codes.get(r.cost_code).map(|c| c.code.clone()).unwrap_or_default();
                        let mut out: Vec<(Option<String>, Vec<&view::BudgetRow>)> = Vec::new();
                        for r in &rows {
                            let key = (group == BudgetGroup::Division).then(|| view::code_division(&code_of(r)).to_string());
                            match out.last_mut() {
                                Some((k, list)) if *k == key => list.push(r),
                                _ => out.push((key, vec![r])),
                            }
                        }
                        out
                    };
                    @for (key, list) in &groups {
                        tbody {
                            @if let Some(k) = key { tr class="group-row" { th colspan=(columns.len() + 1 + usize::from(can_edit)) { "Division " (k) } } }
                            @for r in list {
                                @let code = state.cost_codes.get(r.cost_code);
                                tr {
                                    td { strong { (code.map(|c| c.code.as_str()).unwrap_or("?")) } " " (code.map(|c| c.description.as_str()).unwrap_or("")) @if !r.budgeted { " " span class="tag" { "No budget line" } } }
                                    @for c in &columns { (budget_cell(r, *c)) }
                                    @if can_edit { td class="right" { @if let Some(l) = line_of(r.cost_code) { (button_form(req, "budget.delete_line", &here, &[("project", id.to_string()), ("id", l.to_string())], "Remove", "small danger")) } } }
                                }
                            }
                            @if key.is_some() {
                                @let sub: Vec<view::BudgetRow> = list.iter().map(|r| (*r).clone()).collect();
                                @if let Ok(t) = view::budget_totals(&sub) { tr class="subtotal" { td { "Subtotal" } @for c in &columns { (total_cell(&t, *c)) } @if can_edit { td {} } } }
                            }
                        }
                    }
                    @if let Some(t) = totals {
                        tfoot { tr { td { "Total" } @for c in &columns { (total_cell(&t, *c)) } @if can_edit { td {} } } }
                    }
                }
            }
        }
        @if can_forecast {
            div class="card" {
                h2 { "Forecast the cost still to come" }
                p class="muted small" { "With a forecast, the projected cost is the cost to date plus the forecast. Leave the amount blank to remove a forecast. A code with no forecast is projected by the project's method: " strong { (method.label()) } " (" (match method {
                    ForecastMethod::Budget => "the budget is assumed to be spent",
                    ForecastMethod::Committed => "only what is committed or spent, plus pending change events",
                    ForecastMethod::Progress => "cost so far divided by the percent complete, or the committed cost where none is entered",
                }) ")." }
                (command_form(req, "budget.set_forecast", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Cost code", "fc_code", true, false, html! { select id="fc_code" name="cost_code" required { (code_options(req, id, None, false)) } }))
                        (field("Forecast to complete", "fc_amount", false, false, html! { input type="text" id="fc_amount" name="forecast_to_complete" inputmode="decimal" placeholder="0.00"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save forecast" } }
                }))
                @if method == ForecastMethod::Progress {
                    h3 { "Percent complete" }
                    p class="muted small" { "How much of a code's work is done. Leave blank to remove it." }
                    (command_form(req, "budget.set_progress", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field("Cost code", "pc_code", true, false, html! { select id="pc_code" name="cost_code" required { (code_options(req, id, None, false)) } }))
                            (field("Percent complete", "pc_amount", false, false, html! { input type="text" id="pc_amount" name="percent_complete" inputmode="decimal" placeholder="0"; }))
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Save percent complete" } }
                    }))
                }
                @if req.allowed("budget.set_method", Some(id)) {
                    h3 { "Forecasting method" }
                    (command_form(req, "budget.set_method", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field("Method", "fc_method", true, false, html! { select id="fc_method" name="method" { @for m in ForecastMethod::ALL { option value=(m.key()) selected[*m == method] { (m.label()) } } } }))
                        }
                        div class="form-actions" { button type="submit" class="btn" { "Use this method" } }
                    }))
                }
            }
        }
        @if can_views {
            div class="card" {
                h2 { "Budget views" }
                p class="muted small" { "A view keeps the columns you want to see and whether cost codes are grouped by division. It changes the table above and the CSV export, nothing else." }
                @if !saved_views.is_empty() {
                    ul class="plain" {
                        @for v in &saved_views {
                            li { a href=(format!("{here}?view={}", v.id)) { strong { (v.name) } } " · " (v.columns.len()) " columns" @if v.group == BudgetGroup::Division { ", by division" } " " (button_form(req, "budget.view_delete", &here, &[("project", id.to_string()), ("id", v.id.to_string())], "Delete", "small danger")) }
                        }
                    }
                }
                (command_form(req, "budget.view_save", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    @if let Some(v) = current { input type="hidden" name="id" value=(v.id); }
                    div class="form-grid" {
                        (field("Name", "bv_name", true, false, html! { input type="text" id="bv_name" name="name" required value=[current.map(|v| v.name.as_str())] placeholder="Cash position"; }))
                        (field("Grouping", "bv_group", false, false, html! { select id="bv_group" name="group" { @for g in BudgetGroup::ALL { option value=(g.key()) selected[*g == group] { (g.label()) } } } }))
                    }
                    fieldset { legend { "Columns" }
                        div class="options" { @for c in BudgetColumn::ALL { label { input type="checkbox" name="columns[]" value=(c.key()) checked[columns.contains(c)]; (c.label()) } } }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { (if current.is_some() { "Save changes to this view" } else { "Save as a new view" }) } }
                }))
            }
        }
        @if req.allowed("budget.snapshot", Some(id)) || !snapshots.is_empty() {
            div class="card" {
                h2 { "Snapshots" }
                @if snapshots.is_empty() { p class="muted" { "No snapshots yet. A snapshot keeps today's figures so the budget can be compared with them later." } } @else {
                    ul class="plain" {
                        @for s in &snapshots {
                            li {
                                strong { (s.name) } " · " (req.when(s.created.at))
                                @if !s.note.is_empty() { " · " (s.note) }
                                " " a href=(format!("{here}?compare={}#compare", s.id)) { "Compare with now" }
                                @if req.allowed("budget.delete_snapshot", Some(id)) { " " (button_form(req, "budget.delete_snapshot", &here, &[("project", id.to_string()), ("id", s.id.to_string())], "Delete", "small danger")) }
                            }
                        }
                    }
                }
                @if req.allowed("budget.snapshot", Some(id)) {
                    (command_form(req, "budget.snapshot", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field("Name", "snap_name", true, false, html! { input type="text" id="snap_name" name="name" required placeholder="Start of construction"; }))
                            (field("Note", "snap_note", false, false, html! { input type="text" id="snap_note" name="note"; }))
                        }
                        div class="form-actions" { button type="submit" class="btn" { "Save snapshot" } }
                    }))
                }
            }
        }
        @if let Some(s) = compared {
            div class="card" id="compare" {
                h2 { "Now compared with “" (s.name) "”" }
                div class="table-wrap" {
                    table class="data" {
                        thead { tr { th { "Cost code" } th class="num" { "Revised then" } th class="num" { "Revised now" } th class="num" { "Cost then" } th class="num" { "Cost now" } th class="num" { "Projected then" } th class="num" { "Projected now" } th class="num" { "Projected change" } } }
                        tbody {
                            @for (code, then, now) in compare_rows(&s.rows, &rows, method) {
                                @let c = state.cost_codes.get(code);
                                tr {
                                    td { strong { (c.map(|c| c.code.as_str()).unwrap_or("?")) } " " (c.map(|c| c.description.as_str()).unwrap_or("")) }
                                    td class="num" { (then.revised()) } td class="num" { (now.revised()) }
                                    td class="num" { (then.cost()) } td class="num" { (now.cost()) }
                                    td class="num" { (then.projected()) } td class="num" { (now.projected()) }
                                    td class="num" { (amount(then.projected().checked_sub(now.projected()).unwrap_or(buildkiosk_core::Money::ZERO))) }
                                }
                            }
                        }
                    }
                }
                p class="muted small" { "Projected change is shown as a gain when projected cost fell, and as a loss (red) when it rose." }
            }
        }
        @if can_lock {
            div class="card" {
                h2 { "Original budget" }
                @if locked {
                    p { "Locked: lines can't be edited, added or imported." }
                    (button_form(req, "budget.unlock", &here, &[("project", id.to_string())], "Unlock the budget", "small"))
                } @else {
                    p { "Locking the original budget makes budget changes the only way to alter it." }
                    (button_form(req, "budget.lock", &here, &[("project", id.to_string())], "Lock the budget", "small"))
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Set a budget line" }
                p class="muted small" { "Setting a cost code that already has a line changes its amount." }
                (command_form(req, "budget.set_line", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Cost code", "cost_code", true, false, html! { select id="cost_code" name="cost_code" required { (code_options(req, id, None, false)) } }))
                        (field("Original amount", "original", true, false, html! { input type="text" id="original" name="original" required inputmode="decimal" placeholder="0.00"; }))
                        (field("Notes", "notes", false, true, html! { input type="text" id="notes" name="notes"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save line" } }
                }))
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a budget from a file" }
                    p class="muted small" { "A spreadsheet saved as CSV. Put the column names on the first line: Cost code, Amount (or Original budget), and optionally Description and Notes, in any order. Without a first line of names, the columns are code, amount, notes. A code that already has a line changes it. Up to 1 MB." }
                    form method="post" action=(format!("{here}/import-file")) enctype="multipart/form-data" {
                        input type="hidden" name="_csrf" value=(req.csrf);
                        (field("Budget file", "budget_file", true, true, html! { input type="file" id="budget_file" name="file" accept=".csv,text/csv,text/plain" required; }))
                        @if req.allowed("cost_code.create", Some(id)) {
                            label class="check" { input type="checkbox" name="add_codes" value="true"; " Add the cost codes this project doesn't have yet (they take their names from the Description column)" }
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Import file" } }
                    }
                }
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a budget" }
                    p class="muted small" { "One line per cost code: code, amount, notes (optional), separated by commas or tabs. Codes must already be in the project's cost code list." }
                    (command_form(req, "budget.import", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        (field("Lines", "import_text", true, true, html! { textarea id="import_text" name="text" required placeholder="03-300, 120000, Concrete" {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
    };
    layout::app(req, "Budget", Nav::Budget(id), &crumbs(p, "Budget", None).into_iter().take(2).chain([("Budget", None)]).collect::<Vec<_>>(), body)
}

/// Each cost code in a snapshot or in the current budget, with its figures then and now.
fn compare_rows(
    then: &[buildkiosk_engine::model::finance::SnapshotRow],
    now: &[view::BudgetRow],
    method: ForecastMethod,
) -> Vec<(Id<buildkiosk_engine::model::project::CostCode>, view::BudgetRow, view::BudgetRow)> {
    let blank = |code| view::BudgetRow {
        cost_code: code,
        budgeted: false,
        original: buildkiosk_core::Money::ZERO,
        changes: buildkiosk_core::Money::ZERO,
        committed: buildkiosk_core::Money::ZERO,
        direct: buildkiosk_core::Money::ZERO,
        pending: buildkiosk_core::Money::ZERO,
        forecast_to_complete: None,
        percent_complete: None,
        method,
    };
    let mut codes: Vec<_> = then.iter().map(|r| r.cost_code).chain(now.iter().map(|r| r.cost_code)).collect();
    codes.sort();
    codes.dedup();
    codes
        .into_iter()
        .map(|c| {
            let old = then.iter().find(|r| r.cost_code == c).map(|r| view::BudgetRow {
                cost_code: c,
                budgeted: true,
                original: r.original,
                changes: r.changes,
                committed: r.committed,
                direct: r.direct,
                pending: r.pending,
                forecast_to_complete: r.forecast_to_complete,
                percent_complete: r.percent_complete,
                method,
            });
            let new = now.iter().find(|r| r.cost_code == c).cloned();
            (c, old.unwrap_or_else(|| blank(c)), new.unwrap_or_else(|| blank(c)))
        })
        .collect()
}

pub fn budget_csv(req: &Req<'_>, id: Id<Project>, chosen: Option<Id<BudgetView>>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Budget) < Level::Read {
        return None;
    }
    let rows = view::budget_rows(state, id).ok()?;
    // The chosen view's columns, in table order; with none, every column (the full export).
    let columns: Vec<BudgetColumn> = match chosen.and_then(|v| state.budget_views.get(v)).filter(|v| v.project == id && v.deleted.is_none()) {
        Some(v) => v.columns.clone(),
        None => BudgetColumn::ALL.to_vec(),
    };
    let mut out = format!("Cost code,Description,{}\r\n", columns.iter().map(|c| layout::csv_cell(c.label())).collect::<Vec<_>>().join(","));
    for r in &rows {
        let code = state.cost_codes.get(r.cost_code);
        let mut cells = vec![code.map(|c| c.code.clone()).unwrap_or_default(), code.map(|c| c.description.clone()).unwrap_or_default()];
        for c in &columns {
            cells.push(match c {
                BudgetColumn::Percent => r.percent_complete.map(|p| p.plain()).unwrap_or_default(),
                _ => view::column_amount(r, *c).map(|m| m.plain()).unwrap_or_default(),
            });
        }
        out.push_str(&cells.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

// -- commitments

fn signature_pill(s: SignatureStatus) -> Markup {
    let class = match s {
        SignatureStatus::NotSent => "grey",
        SignatureStatus::Sent => "amber",
        SignatureStatus::Signed => "green",
        SignatureStatus::Declined => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn status_pill(s: CommitmentStatus) -> Markup {
    let class = match s {
        CommitmentStatus::Draft => "violet",
        CommitmentStatus::Approved => "blue",
        CommitmentStatus::Complete => "green",
        CommitmentStatus::Void => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

/// Filters of the commitments list.
#[derive(Default)]
pub struct CommitmentFilter {
    pub status: Option<CommitmentStatus>,
    pub q: String,
}

pub fn commitments(req: &Req<'_>, id: Id<Project>, filter: &CommitmentFilter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Commitments) < Level::Read {
        return not_found(req);
    }
    let all = view::commitments_for(state, req.actor(), id);
    let q = filter.q.to_lowercase();
    let shown: Vec<&Commitment> = all
        .iter()
        .copied()
        .filter(|c| filter.status.is_none_or(|s| c.status == s))
        .filter(|c| q.is_empty() || c.title.to_lowercase().contains(&q) || c.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/commitments");
    let count = |s: CommitmentStatus| all.iter().filter(|c| c.status == s).count();
    let tab = |label: &str, status: Option<CommitmentStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let approved_total = Money::sum(all.iter().filter(|c| c.status.counts()).filter_map(|c| c.total())).unwrap_or(Money::ZERO);
    let body = html! {
        div class="page-head" {
            div { h1 { "Commitments" } div class="sub" { "Subcontracts and purchase orders · " (approved_total) " committed" } }
            div class="actions" { @if req.allowed("commitment.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "New commitment" } } }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Draft", Some(CommitmentStatus::Draft), count(CommitmentStatus::Draft)))
            (tab("Approved", Some(CommitmentStatus::Approved), count(CommitmentStatus::Approved)))
            (tab("Complete", Some(CommitmentStatus::Complete), count(CommitmentStatus::Complete)))
            (tab("Void", Some(CommitmentStatus::Void), count(CommitmentStatus::Void)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title or number" aria-label="Search commitments";
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if req.allowed("commitment.require_signed", Some(id)) {
            div class="card" {
                @let required = p.require_signed_contracts;
                p { "Signed contracts: " strong { (if required { "required before a commitment is approved" } else { "not required" }) } }
                (command_form(req, "commitment.require_signed", &base, &base, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="required" value=(if required { "false" } else { "true" });
                    div class="form-actions" { button type="submit" class="btn small" { (if required { "Stop requiring signatures" } else { "Require a signed contract before approval" }) } }
                }))
            }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Commitments), "No commitments here", if all.is_empty() { "Create a subcontract or purchase order to start tracking committed cost." } else { "No commitment matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Title" } th { "Company" } th { "Type" } th { "Status" } th { "Contract" } th class="num" { "Total" } } }
                    tbody {
                        @for c in &shown {
                            tr {
                                td class="nowrap" { (c.number) }
                                td { a class="row-link" href=(format!("{base}/{}", c.id)) { (c.title) } }
                                td { @if let Some(v) = c.vendor { (view::vendor_name(state, v)) } }
                                td { (c.kind.label()) }
                                td { (status_pill(c.status)) }
                                td { @if c.signature.status != SignatureStatus::NotSent || p.require_signed_contracts { (signature_pill(c.signature.status)) } }
                                td class="num" { @if let Some(t) = c.total() { (t) } }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(
        req,
        "Commitments",
        Nav::Commitments(id),
        &crumbs(p, "Commitments", None).into_iter().take(2).chain([("Commitments", None)]).collect::<Vec<_>>(),
        body,
    )
}

pub fn commitment(req: &Req<'_>, id: Id<Project>, cid: Id<Commitment>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(c) = state.commitments.get(cid).filter(|c| c.project == id && view::can_see_commitment(state, req.actor(), c)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/commitments");
    let here = format!("{base}/{}", c.id);
    let ids = [("project", id.to_string()), ("id", c.id.to_string())];
    let level = view::level(state, req.actor(), id, Tool::Commitments);
    let admin = level == Level::Admin;
    let name = c.label();
    let body = html! {
        div class="page-head" {
            div {
                h1 { (name) ": " (c.title) " " (status_pill(c.status)) }
                div class="sub" { @if let Some(v) = c.vendor { (view::vendor_name(state, v)) " " @if buildkiosk_engine::model::perm::company_level(state, req.person, Tool::Prequalification) >= Level::Read { a href=(format!("/prequalification/{v}")) { (crate::prequal::standing_pill(buildkiosk_engine::cmd::prequal::vendor_standing(state, v, req.today()))) } " " } " · " } "Created " (req.when(c.created.at)) @if let Some(b) = c.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if can_edit(state, req.actor(), c) { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if admin && c.status == CommitmentStatus::Draft { (button_form(req, "commitment.approve", &here, &ids, "Approve", "primary")) }
                @if admin && c.status == CommitmentStatus::Approved { (button_form(req, "commitment.complete", &here, &ids, "Mark complete", "")) }
                @if admin && c.status != CommitmentStatus::Void { (button_form(req, "commitment.void", &here, &ids, "Void", "danger")) }
                @if level >= Level::Standard && (admin || (c.status == CommitmentStatus::Draft && c.created.by == Some(req.person))) { (button_form(req, "commitment.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if !c.description.is_empty() { div class="card" { h2 { "Description" } p class="prose" { (c.description) } } }
        div class="card" {
            h2 { "Schedule of values" }
            @if c.lines.is_empty() { p class="muted" { "No lines yet." } }
            @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "#" } th { "Cost code" } th { "Description" } th class="num" { "Amount" } } }
                    tbody { @for (n, l) in c.lines.iter().enumerate() {
                        @let code = state.cost_codes.get(l.cost_code);
                        tr { td { (n + 1) } td { (code.map(|x| x.display()).unwrap_or_default()) } td { (l.description) } td class="num" { (l.amount) } }
                    } }
                    tfoot { tr { td colspan="3" { "Total" } td class="num" { @if let Some(t) = c.total() { (t) } } } }
                } }
            }
            @if !c.status.counts() { p class="muted small" { "Only approved and complete commitments count as committed cost in the budget." } }
        }
        @let sig = &c.signature;
        @let can_sign = !matches!(c.status, CommitmentStatus::Complete | CommitmentStatus::Void) && req.allowed("commitment.send_for_signature", Some(id));
        @if sig.status != SignatureStatus::NotSent || can_sign || p.require_signed_contracts {
            div class="card" {
                div class="card-head" { h2 { "Contract signature" } (signature_pill(sig.status)) }
                @if p.require_signed_contracts && sig.status != SignatureStatus::Signed && c.status == CommitmentStatus::Draft {
                    p class="muted small" { "This project needs a signed contract before a commitment can be approved." }
                }
                @if sig.status != SignatureStatus::NotSent {
                    dl class="facts" {
                        @if let Some(d) = sig.sent_on { dt { "Sent" } dd { (d.pretty()) @if !sig.sent_to.is_empty() { " to " (sig.sent_to) } } }
                        @if let Some(d) = sig.signed_on { dt { "Signed" } dd { (d.pretty()) @if !sig.signer.is_empty() { " by " (sig.signer) } } }
                        @if !sig.note.is_empty() { dt { "Note" } dd { (sig.note) } }
                    }
                }
                @if sig.status == SignatureStatus::Signed { p class="muted small" { "Attach the signed copy under Files if you haven't." } }
                @if can_sign {
                    @if sig.status != SignatureStatus::Signed {
                        @if sig.status != SignatureStatus::Sent {
                            (command_form(req, "commitment.send_for_signature", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(c.id);
                                div class="form-grid" {
                                    (field("Sent to", "sig_to", false, false, html! { input type="text" id="sig_to" name="to" placeholder="Name or email"; }))
                                    (field("Date sent", "sig_on", false, false, html! { input type="date" id="sig_on" name="on"; }))
                                }
                                div class="form-actions" { button type="submit" class="btn" { "Mark as sent for signature" } }
                            }))
                        }
                        details class="disclosure" {
                            summary { "Record the signed contract" }
                            (command_form(req, "commitment.record_signature", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(c.id);
                                div class="form-grid" {
                                    (field("Signed by", "sig_by", true, false, html! { input type="text" id="sig_by" name="signer" required placeholder="Who signed for the other party"; }))
                                    (field("Date signed", "sig_when", false, false, html! { input type="date" id="sig_when" name="on"; }))
                                    (field("Note", "sig_note", false, true, html! { input type="text" id="sig_note" name="note"; }))
                                }
                                div class="form-actions" { button type="submit" class="btn primary" { "Record signature" } }
                            }))
                        }
                    }
                    @if sig.status == SignatureStatus::Sent {
                        details class="disclosure" {
                            summary { "It came back unsigned" }
                            (command_form(req, "commitment.decline_signature", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(c.id);
                                (field("Why", "sig_why", false, true, html! { input type="text" id="sig_why" name="note"; }))
                                div class="form-actions" { button type="submit" class="btn" { "Record as declined" } }
                            }))
                        }
                    }
                    @if sig.status != SignatureStatus::NotSent && (sig.status != SignatureStatus::Signed || admin) {
                        (button_form(req, "commitment.reset_signature", &here, &ids, if sig.status == SignatureStatus::Signed { "Clear the signature" } else { "Take back" }, "small"))
                    }
                }
            }
        }
        @if c.status.counts() && view::level(state, req.actor(), id, Tool::Invoicing) >= Level::Read {
            @if let Ok(b) = view::commitment_billing(state, c) {
                div class="card" {
                    div class="card-head" { h2 { "Billing" } a class="btn small" href=(format!("/projects/{id}/commitment-invoices")) { "Commitment invoices" } }
                    dl class="facts" {
                        dt { "Retainage" } dd { @if let Some(r) = c.retainage { (r) } @else { "None" } }
                        dt { "Revised commitment" } dd { (b.revised) }
                        dt { "Billed to date" } dd { (b.billed) }
                        dt { "Retainage held" } dd { (b.retainage_held) }
                        dt { "Paid (net)" } dd { (b.paid) }
                        dt { "Balance to finish" } dd { (b.balance_to_finish) }
                    }
                }
            }
        }
        @let changes: Vec<_> = state.commitment_changes.iter().filter(|x| x.commitment == c.id && x.deleted.is_none() && view::can_see_commitment_change(state, req.actor(), x)).collect();
        @if !changes.is_empty() || (c.status.counts() && req.allowed("commitment_change.create", Some(id))) {
            div class="card" {
                div class="card-head" {
                    h2 { "Change orders" }
                    @if c.status.counts() && req.allowed("commitment_change.create", Some(id)) { a class="btn small" href=(format!("/projects/{id}/change-orders/new")) { "New change order" } }
                }
                @if changes.is_empty() { p class="muted" { "No change orders." } }
                @else {
                    div class="table-wrap" { table class="data" {
                        thead { tr { th { "#" } th { "Title" } th { "Status" } th class="num" { "Amount" } } }
                        tbody { @for x in &changes {
                            tr { td { (x.number) } td { a class="row-link" href=(format!("/projects/{id}/change-orders/{}", x.id)) { (x.title) } } td { (x.status.label()) } td class="num" { @if let Some(t) = Money::sum(x.lines.iter().map(|l| l.amount)) { (t) } } }
                        } }
                        @if let Some(r) = view::commitment_revised_total(state, c) { tfoot { tr { td colspan="3" { "Revised total (approved changes included)" } td class="num" { (r) } } } }
                    } }
                }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if c.attachments.is_empty() { p class="muted" { "No files yet. Attach the signed contract, insurance certificates or the scope of work." } } @else {
                ul class="plain" {
                    @for f in &c.attachments {
                        li {
                            a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (crate::pages::human_size(f.size)) }
                            @if req.allowed("commitment.detach", Some(id)) && (c.status == CommitmentStatus::Draft || admin) {
                                " " (button_form(req, "commitment.detach", &here, &[("project", id.to_string()), ("id", c.id.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger"))
                            }
                        }
                    }
                }
            }
            @if req.allowed("commitment.attach", Some(id)) && c.status != CommitmentStatus::Void {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
        div class="card" {
            h2 { "History" }
            ul class="timeline" {
                @for e in history.iter().rev() {
                    li {
                        div { (find_command(&e.cmd).map(|x| x.label).unwrap_or(e.cmd.as_str())) " by " @if let Some(a) = e.actor { (view::person_name(state, a)) } @else { "the system" } }
                        div class="when" { (req.when(e.at)) }
                    }
                }
            }
        }
    };
    layout::app(req, &name, Nav::Commitments(id), &crumbs(p, &name, Some(("Commitments", base.clone()))), body)
}

pub fn commitment_form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<Commitment>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(cid) => match state
            .commitments
            .get(cid)
            .filter(|c| c.project == id && view::can_see_commitment(state, req.actor(), c) && can_edit(state, req.actor(), c))
        {
            Some(c) => Some(c),
            None => return not_found(req),
        },
        None => {
            if !req.allowed("commitment.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let base = format!("/projects/{id}/commitments");
    let action = match current {
        Some(c) => format!("{base}/{}/save", c.id),
        None => format!("{base}/save"),
    };
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let lines = current.map(|c| c.lines.as_slice()).unwrap_or(&[]);
    let kind = current.map(|c| c.kind).unwrap_or_default();
    let title = if current.is_some() { "Edit commitment" } else { "New commitment" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        form method="post" action=(action) {
            input type="hidden" name="_csrf" value=(req.csrf);
            div class="card" {
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(current.map(|c| c.title.clone()).unwrap_or_default()); }))
                    (field("Type", "kind", false, false, html! { select id="kind" name="kind" { @for k in [CommitmentKind::Subcontract, CommitmentKind::PurchaseOrder] { option value=(k.key()) selected[k == kind] { (k.label()) } } } }))
                    (field("Company", "vendor", false, false, html! { select id="vendor" name="vendor" { option value="" { "—" } @for v in &vendors { option value=(v.id) selected[current.and_then(|c| c.vendor) == Some(v.id)] { (v.name) } } } }))
                    (field("Retainage (%)", "retainage", false, false, html! { input type="text" id="retainage" name="retainage" inputmode="decimal" placeholder="5" value=(current.and_then(|c| c.retainage).map(|r| r.to_string().trim_end_matches('%').to_string()).unwrap_or_default()); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (current.map(|c| c.description.clone()).unwrap_or_default()) } }))
                }
            }
            div class="card" {
                h2 { "Schedule of values" }
                p class="muted small" { "One line per cost code. Leave spare rows blank." }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Cost code" } th { "Description" } th { "Amount" } } }
                    tbody { @for i in 0..(lines.len() + 3).max(6) {
                        @let l = lines.get(i);
                        tr {
                            td { select name=(format!("lines.{i}.cost_code")) aria-label=(format!("Cost code, line {}", i + 1)) { (code_options(req, id, l.map(|l| l.cost_code), true)) } }
                            td { input type="text" name=(format!("lines.{i}.description")) value=(l.map(|l| l.description.clone()).unwrap_or_default()) aria-label=(format!("Description, line {}", i + 1)); }
                            td { input type="text" name=(format!("lines.{i}.amount")) value=(l.map(|l| l.amount.plain()).unwrap_or_default()) inputmode="decimal" aria-label=(format!("Amount, line {}", i + 1)); }
                        }
                    } }
                } }
            }
            div class="form-actions" {
                a class="btn" href=(current.map(|c| format!("{base}/{}", c.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { "Save draft" }) }
            }
        }
    };
    layout::app(req, title, Nav::Commitments(id), &crumbs(p, title, Some(("Commitments", base.clone()))), body)
}

// -- direct costs

pub fn direct_costs(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::DirectCosts) < Level::Read {
        return not_found(req);
    }
    let mut list: Vec<_> = state.direct_costs.iter().filter(|d| d.project == id && d.deleted.is_none()).collect();
    list.sort_by_key(|d| std::cmp::Reverse((d.date, d.id)));
    let total = Money::sum(list.iter().map(|d| d.amount)).unwrap_or(Money::ZERO);
    let here = format!("/projects/{id}/direct-costs");
    let can_add = req.allowed("direct_cost.create", Some(id));
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let body = html! {
        div class="page-head" { div { h1 { "Direct costs" } div class="sub" { "Costs paid without a commitment · " (total) " in all" } } }
        @if list.is_empty() {
            div class="card" { (empty(tool_icon(Tool::DirectCosts), "No direct costs yet", "Record a purchase, a rental or another cost below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Date" } th { "Cost code" } th { "Description" } th { "Type" } th { "Company" } th class="num" { "Amount" } @if can_add { th {} } } }
                    tbody {
                        @for d in list.iter().take(300) {
                            tr {
                                td class="nowrap" { (d.date.pretty()) }
                                td { (state.cost_codes.get(d.cost_code).map(|c| c.display()).unwrap_or_default()) }
                                td { (d.description) }
                                td { (d.kind.label()) }
                                td { @if let Some(v) = d.vendor { (view::vendor_name(state, v)) } @else { (dash("")) } }
                                td class="num" { (amount(d.amount)) }
                                @if can_add { td class="right" { @if d.created.by == Some(req.person) || req.allowed("direct_cost.restore", Some(id)) { (button_form(req, "direct_cost.delete", &here, &[("project", id.to_string()), ("id", d.id.to_string())], "Delete", "small danger")) } } }
                            }
                        }
                    }
                }
            }
        }
        @if can_add {
            div class="card" {
                h2 { "Record a direct cost" }
                (command_form(req, "direct_cost.create", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Description", "description", true, true, html! { input type="text" id="description" name="description" required maxlength="200"; }))
                        (field("Cost code", "cost_code", true, false, html! { select id="cost_code" name="cost_code" required { (code_options(req, id, None, false)) } }))
                        (field("Amount", "amount", true, false, html! { input type="text" id="amount" name="amount" required inputmode="decimal" placeholder="0.00"; }))
                        (field("Date", "date", false, false, html! { input type="date" id="date" name="date" value=(req.today().to_string()); }))
                        (field("Type", "kind", false, false, html! { select id="kind" name="kind" { @for k in [CostType::Material, CostType::Labor, CostType::Equipment, CostType::Subcontract, CostType::Other] { option value=(k.key()) { (k.label()) } } } }))
                        (field("Company", "vendor", false, false, html! { select id="vendor" name="vendor" { option value="" { "—" } @for v in &vendors { option value=(v.id) { (v.name) } } } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Record cost" } }
                }))
            }
        }
    };
    layout::app(
        req,
        "Direct costs",
        Nav::DirectCosts(id),
        &crumbs(p, "Direct costs", None).into_iter().take(2).chain([("Direct costs", None)]).collect::<Vec<_>>(),
        body,
    )
}
