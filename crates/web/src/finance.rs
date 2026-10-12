//! Cost pages: the budget, commitments and direct costs.

use buildkiosk_core::{Id, Money};
use buildkiosk_engine::cmd::commitment::can_edit;
use buildkiosk_engine::model::finance::{Commitment, CommitmentKind, CommitmentStatus};
use buildkiosk_engine::model::project::{CostType, Project};
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

pub fn budget(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Budget) < Level::Read {
        return not_found(req);
    }
    let rows = view::budget_rows(state, id).unwrap_or_default();
    let totals = view::budget_totals(&rows).ok();
    let here = format!("/projects/{id}/budget");
    let can_edit = req.allowed("budget.set_line", Some(id));
    let line_of = |code| state.budget_lines.iter().find(|b| b.project == id && b.cost_code == code && b.deleted.is_none()).map(|b| b.id);
    let body = html! {
        div class="page-head" {
            div { h1 { "Budget" } div class="sub" { "What each cost code was budgeted, what is committed and what has been spent" } }
            div class="actions" {
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
                @if !t.pending.is_zero() { a class="stat" href=(format!("/projects/{id}/change-events")) { div class="label" { "Pending change events" } div class="value" { (t.pending) } } }
            }
        }
        @if rows.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Budget), "No budget yet", "Add a line for a cost code, or import a list below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Cost code" } th class="num" { "Original" } th class="num" { "Changes" } th class="num" { "Revised" } th class="num" { "Committed" } th class="num" { "Direct" } th class="num" { "Total cost" } th class="num" { "Remaining" } @if can_edit { th {} } } }
                    tbody {
                        @for r in &rows {
                            @let code = state.cost_codes.get(r.cost_code);
                            tr {
                                td { strong { (code.map(|c| c.code.as_str()).unwrap_or("?")) } " " (code.map(|c| c.description.as_str()).unwrap_or("")) @if !r.budgeted { " " span class="tag" { "No budget line" } } }
                                td class="num" { (r.original) }
                                td class="num" { (r.changes) }
                                td class="num" { (r.revised()) }
                                td class="num" { (r.committed) }
                                td class="num" { (r.direct) }
                                td class="num" { (r.cost()) }
                                td class="num" { (amount(r.remaining())) }
                                @if can_edit { td class="right" { @if let Some(l) = line_of(r.cost_code) { (button_form(req, "budget.delete_line", &here, &[("project", id.to_string()), ("id", l.to_string())], "Remove", "small danger")) } } }
                            }
                        }
                    }
                    @if let Some(t) = totals {
                        tfoot { tr { td { "Total" } td class="num" { (t.original) } td class="num" { (t.changes) } td class="num" { (t.revised()) } td class="num" { (t.committed) } td class="num" { (t.direct) } td class="num" { (t.cost()) } td class="num" { (amount(t.remaining())) } @if can_edit { td {} } } }
                    }
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

pub fn budget_csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Budget) < Level::Read {
        return None;
    }
    let rows = view::budget_rows(state, id).ok()?;
    let mut out = String::from("Cost code,Description,Original,Changes,Revised,Committed,Direct,Total cost,Pending,Remaining\r\n");
    for r in &rows {
        let code = state.cost_codes.get(r.cost_code);
        let cells = [
            code.map(|c| c.code.clone()).unwrap_or_default(),
            code.map(|c| c.description.clone()).unwrap_or_default(),
            r.original.plain(),
            r.changes.plain(),
            r.revised().plain(),
            r.committed.plain(),
            r.direct.plain(),
            r.cost().plain(),
            r.pending.plain(),
            r.remaining().plain(),
        ];
        out.push_str(&cells.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

// -- commitments

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
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Commitments), "No commitments here", if all.is_empty() { "Create a subcontract or purchase order to start tracking committed cost." } else { "No commitment matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Title" } th { "Company" } th { "Type" } th { "Status" } th class="num" { "Total" } } }
                    tbody {
                        @for c in &shown {
                            tr {
                                td class="nowrap" { (c.number) }
                                td { a class="row-link" href=(format!("{base}/{}", c.id)) { (c.title) } }
                                td { @if let Some(v) = c.vendor { (view::vendor_name(state, v)) } }
                                td { (c.kind.label()) }
                                td { (status_pill(c.status)) }
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
                div class="sub" { @if let Some(v) = c.vendor { (view::vendor_name(state, v)) " · " } "Created " (req.when(c.created.at)) @if let Some(b) = c.created.by { " by " (view::person_name(state, b)) } }
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
