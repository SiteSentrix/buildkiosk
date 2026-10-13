//! Estimating pages: the estimates list, one estimate with its totals, and the form.

use buildkiosk_core::{Id, Money};
use buildkiosk_engine::cmd::estimate::visible;
use buildkiosk_engine::model::estimate::{Estimate, EstimateLine, EstimateStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, empty, field};
use crate::pages::{not_found, project_for};

fn qty(l: &EstimateLine) -> String {
    let (w, f) = (l.quantity_milli / 1000, (l.quantity_milli % 1000).abs());
    if f == 0 { format!("{w}") } else { format!("{w}.{}", format!("{f:03}").trim_end_matches('0')) }
}

fn status_pill(s: EstimateStatus) -> Markup {
    html! { span class=(format!("pill {}", if s == EstimateStatus::Final { "green" } else { "violet" })) { (match s { EstimateStatus::Draft => "Draft", EstimateStatus::Final => "Issued" }) } }
}

pub fn list(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Estimating);
    if lvl < Level::Read {
        return not_found(req);
    }
    let list = visible(state, id);
    let base = format!("/projects/{id}/estimating");
    let body = html! {
        div class="page-head" {
            div { h1 { "Estimating" } div class="sub" { "Price the work by cost code, add markups, and put the result in the budget" } }
            div class="actions" { @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New estimate" } } }
        }
        @if list.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Estimating), "No estimates", "Start an estimate and price each cost code.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Estimate" } th class="num" { "Lines" } th class="num" { "Total with markups" } th { "In the budget" } th { "Status" } } }
                tbody { @for e in &list { tr {
                    td { (e.number) }
                    td { a href=(format!("{base}/{}", e.id)) { (e.name) } }
                    td class="num" { (e.lines.len()) }
                    td class="num" { @if let Some(t) = e.totals() { (t.total) } }
                    td { @if e.applied.is_some() { span class="pill blue" { "Applied" } } }
                    td { (status_pill(e.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Estimating",
        Nav::Estimating(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Estimating", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, eid: Id<Estimate>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Estimating);
    let Some(e) = state.estimates.get(eid).filter(|e| e.project == id && e.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/estimating");
    let here = format!("{base}/{eid}");
    let ids = [("project", id.to_string()), ("id", eid.to_string())];
    let totals = e.totals();
    let admin = lvl == Level::Admin;
    let code = |c| state.cost_codes.get(c).map(|c| format!("{} {}", c.code, c.description)).unwrap_or_default();
    let codes = view::project_cost_codes(state, id);
    let body = html! {
        div class="page-head" {
            div { h1 { (e.label()) ": " (e.name) " " (status_pill(e.status)) } div class="sub" { @if e.applied.is_some() { "Put in the budget " (e.applied.map(|a| req.when(a.at)).unwrap_or_default()) } @else { "Not in the budget yet" } } }
            div class="actions" {
                a class="btn" href=(format!("{here}/csv")) { "Export CSV" }
                @if lvl >= Level::Standard && e.status == EstimateStatus::Draft { a class="btn" href=(format!("{here}/edit")) { "Edit" } (button_form(req, "estimate.finalize", &here, &ids, "Issue", "primary")) }
                @if lvl >= Level::Standard && e.status == EstimateStatus::Final { (button_form(req, "estimate.reopen", &here, &ids, "Reopen", "")) }
                @if lvl >= Level::Standard { (button_form(req, "estimate.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if !e.notes.is_empty() { div class="card" { p class="prose" { (e.notes) } } }
        div class="card" {
            @if e.lines.is_empty() { p class="muted" { "No lines yet." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Cost code" } th { "Description" } th class="num" { "Quantity" } th { "Unit" } th class="num" { "Unit cost" } th class="num" { "Amount" } } }
                    tbody { @for l in &e.lines { tr {
                        td { (code(l.cost_code)) } td { (l.description) } td class="num" { (qty(l)) } td { (l.unit) } td class="num" { (l.unit_cost) } td class="num" { (l.total().unwrap_or(Money::ZERO)) }
                    } } }
                    @if let Some(t) = &totals { tfoot {
                        tr { td colspan="5" { "Subtotal" } td class="num" { (t.subtotal) } }
                        @for (m, amt) in e.markups.iter().zip(&t.markups) { tr { td colspan="5" { (m.name) " (" (m.percent) ")" } td class="num" { (amt) } } }
                        tr { td colspan="5" { "Total" } td class="num" { (t.total) } }
                    } }
                } }
            }
        }
        @if admin && e.status == EstimateStatus::Final {
            div class="card" {
                h2 { "Put in the budget" }
                p class="muted small" { "Sets each cost code's original budget to its amount in this estimate. Choose a cost code to hold the markups, or leave them out. The original budget must be unlocked." }
                (command_form(req, "estimate.apply_to_budget", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(eid);
                    div class="form-grid" { (field("Markups go to", "markup_cost_code", false, false, html! { select id="markup_cost_code" name="markup_cost_code" { option value="" { "Leave the markups out" } @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } })) }
                    div class="form-actions" { button type="submit" class="btn primary" { (if e.applied.is_some() { "Put in the budget again" } else { "Put in the budget" }) } }
                }))
            }
        }
    };
    layout::app(
        req,
        &e.label(),
        Nav::Estimating(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Estimating", Some(base)), (&e.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<Estimate>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.estimates.get(c).filter(|e| e.project == id && e.deleted.is_none() && e.status == EstimateStatus::Draft) {
            Some(e) => Some(e),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "estimate.update" } else { "estimate.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/estimating");
    let back = cur.map(|e| format!("{base}/{}/edit", e.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|e| format!("{base}/{}", e.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let codes = view::project_cost_codes(state, id);
    let line_rows = cur.map(|e| e.lines.len()).unwrap_or(0) + 5;
    let markup_rows = cur.map(|e| e.markups.len()).unwrap_or(0) + 2;
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit estimate" } else { "New estimate" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(e) = cur { input type="hidden" name="id" value=(e.id); }
                div class="form-grid" {
                    (field("Name", "name", true, true, html! { input type="text" id="name" name="name" required value=(cur.map(|e| e.name.as_str()).unwrap_or_default()); }))
                    (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (cur.map(|e| e.notes.as_str()).unwrap_or_default()) } }))
                }
                h2 { "Lines" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Cost code" } th { "Description" } th { "Quantity" } th { "Unit" } th { "Unit cost" } } }
                    tbody { @for i in 0..line_rows { @let l = cur.and_then(|e| e.lines.get(i)); tr {
                        td { select name=(format!("lines.{i}.cost_code")) aria-label=(format!("Cost code of line {}", i + 1)) { option value="" { "—" } @for c in &codes { option value=(c.id) selected[l.is_some_and(|x| x.cost_code == c.id)] { (c.code) " " (c.description) } } } }
                        td { input type="text" name=(format!("lines.{i}.description")) value=(l.map(|x| x.description.as_str()).unwrap_or_default()) aria-label=(format!("Description of line {}", i + 1)); }
                        td { input type="text" inputmode="decimal" name=(format!("lines.{i}.quantity")) value=(l.map(qty).unwrap_or_default()) aria-label=(format!("Quantity of line {}", i + 1)); }
                        td { input type="text" name=(format!("lines.{i}.unit")) value=(l.map(|x| x.unit.as_str()).unwrap_or_default()) aria-label=(format!("Unit of line {}", i + 1)); }
                        td { input type="text" inputmode="decimal" name=(format!("lines.{i}.unit_cost")) value=(l.map(|x| x.unit_cost.plain()).unwrap_or_default()) aria-label=(format!("Unit cost of line {}", i + 1)); }
                    } } }
                } }
                h2 { "Markups (a percentage of the subtotal)" }
                table class="data" {
                    thead { tr { th { "Name" } th { "Percent" } } }
                    tbody { @for i in 0..markup_rows { @let m = cur.and_then(|e| e.markups.get(i)); tr {
                        td { input type="text" name=(format!("markups.{i}.name")) value=(m.map(|x| x.name.as_str()).unwrap_or_default()) placeholder="Overhead" aria-label=(format!("Markup {} name", i + 1)); }
                        td { input type="text" inputmode="decimal" name=(format!("markups.{i}.percent")) value=(m.map(|x| x.percent.plain()).unwrap_or_default()) aria-label=(format!("Markup {} percent", i + 1)); }
                    } } }
                }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|e| format!("{base}/{}", e.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Estimate",
        Nav::Estimating(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Estimating", Some(base)), ("Estimate", None)],
        body,
    )
}

pub fn csv(req: &Req<'_>, id: Id<Project>, eid: Id<Estimate>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Estimating) < Level::Read {
        return None;
    }
    let e = state.estimates.get(eid).filter(|e| e.project == id && e.deleted.is_none())?;
    let mut out = String::from("Cost code,Description,Quantity,Unit,Unit cost,Amount\r\n");
    for l in &e.lines {
        let c = state.cost_codes.get(l.cost_code).map(|c| c.code.clone()).unwrap_or_default();
        let cells = [c, l.description.clone(), qty(l), l.unit.clone(), l.unit_cost.plain(), l.total().unwrap_or(Money::ZERO).plain()];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    if let Some(t) = e.totals() {
        out.push_str(&format!("Subtotal,,,,,{}\r\n", t.subtotal.plain()));
        for (m, a) in e.markups.iter().zip(&t.markups) {
            out.push_str(&format!("{},,,,,{}\r\n", csv_cell(&m.name), a.plain()));
        }
        out.push_str(&format!("Total,,,,,{}\r\n", t.total.plain()));
    }
    Some(out)
}
