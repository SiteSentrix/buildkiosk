//! Estimating pages: the estimates list, one estimate with its totals, and the form.

use buildkiosk_core::{Id, Money};
use buildkiosk_engine::cmd::estimate::{can_read_templates, takeoff_kind, templates, visible};
use buildkiosk_engine::model::drawing::{Drawing, MarkupKind};
use buildkiosk_engine::model::estimate::{Estimate, EstimateLine, EstimateStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
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
            div class="actions" {
                @if buildkiosk_engine::cmd::catalog::can_read(state, req.actor()) { a class="btn" href="/catalog" { "Catalogue" } }
                @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New estimate" } }
            }
        }
        @if list.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Estimating), "No estimates", "Start an estimate and price each cost code.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Estimate" } th class="num" { "Lines" } th class="num" { "Total with markups" } th { "In the budget" } th { "Status" } } }
                tbody { @for e in &list { tr {
                    td { (e.number) }
                    td { a href=(format!("{base}/{}", e.id)) { (e.name) } @if let Some(b) = e.based_on.and_then(|b| state.estimates.get(b)) { " " span class="muted small" { "(from #" (b.number) ")" } } }
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

pub fn detail(req: &Req<'_>, id: Id<Project>, eid: Id<Estimate>, takeoff: Option<Id<Drawing>>) -> Markup {
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
            div { h1 { (e.label()) ": " (e.name) " " (status_pill(e.status)) } div class="sub" { @if let Some(b) = e.based_on.and_then(|b| state.estimates.get(b)) { "A new version of " a href=(format!("{base}/{}", b.id)) { (b.label()) } " · " } @if e.applied.is_some() { "Put in the budget " (e.applied.map(|a| req.when(a.at)).unwrap_or_default()) } @else { "Not in the budget yet" } } }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                a class="btn" href=(format!("{here}/csv")) { "Export CSV" }
                @if lvl >= Level::Standard && e.status == EstimateStatus::Draft { a class="btn" href=(format!("{here}/edit")) { "Edit" } (button_form(req, "estimate.finalize", &here, &ids, "Issue", "primary")) }
                @if lvl >= Level::Standard && e.status == EstimateStatus::Final { (button_form(req, "estimate.reopen", &here, &ids, "Reopen", "")) }
                @if lvl >= Level::Standard { (button_form(req, "estimate.duplicate", &here, &ids, "New version", "")) }
                @if lvl >= Level::Standard { (button_form(req, "estimate.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if lvl >= Level::Standard && !e.lines.is_empty() && req.allowed("estimate.template_save", Some(id)) {
            div class="card" {
                details class="disclosure" {
                    summary { "Save as a template" }
                    (command_form(req, "estimate.template_save", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(eid);
                        div class="form-grid" { (field("Template name", "tpl_name", true, false, html! { input type="text" id="tpl_name" name="name" required value=(e.name); })) }
                        p class="muted small" { "Keeps the lines, markups and alternates for any project to start from. Cost codes are kept by number, so a project needs the same codes to use it." }
                        div class="form-actions" { button type="submit" class="btn primary small" { "Save template" } }
                    }))
                }
            }
        }
        @if state.estimates.iter().filter(|x| x.project == id && x.deleted.is_none() && x.id != eid).count() > 0 {
            form method="get" action=(format!("{base}/compare")) class="filters" {
                input type="hidden" name="a" value=(eid);
                label for="cmp" { "Compare with" }
                select id="cmp" name="b" { @for x in visible(state, id).into_iter().filter(|x| x.id != eid) { option value=(x.id) { (x.label()) ": " (x.name) } } }
                button type="submit" class="btn small" { "Compare" }
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
                        @if !t.alternates.is_zero() { tr { td colspan="5" { "Accepted alternates" } td class="num" { (t.alternates) } } }
                        tr { td colspan="5" { "Subtotal" } td class="num" { (t.subtotal) } }
                        @for (m, amt) in e.markups.iter().zip(&t.markups) { tr { td colspan="5" { (m.name) " (" (m.percent) ")" } td class="num" { (amt) } } }
                        tr { td colspan="5" { "Total" } td class="num" { (t.total) } }
                    } }
                } }
            }
        }
        @if !e.alternates.is_empty() {
            div class="card" {
                h2 { "Alternates" }
                p class="muted small" { "Priced apart from the base estimate. Accepted ones are added to the subtotal, and to the budget under their own cost codes." }
                table class="data" {
                    thead { tr { th { "Alternate" } th { "Cost code" } th class="num" { "Amount" } th { "" } th {} } }
                    tbody { @for (i, a) in e.alternates.iter().enumerate() { tr {
                        td { strong { (a.name) } @if !a.description.is_empty() { br; span class="muted small" { (a.description) } } }
                        td { (code(a.cost_code)) }
                        td class="num" { (a.amount) }
                        td { @if a.accepted { span class="pill green" { "Accepted" } } @else { span class="pill" { "Not accepted" } } }
                        td class="right" { @if lvl >= Level::Standard && e.applied.is_none() {
                            (button_form(req, "estimate.accept_alternate", &here, &[("project", id.to_string()), ("id", eid.to_string()), ("index", i.to_string()), ("accepted", (!a.accepted).to_string())], if a.accepted { "Decline" } else { "Accept" }, "small"))
                        } }
                    } } }
                }
            }
        }
        @if lvl >= Level::Standard && e.status == EstimateStatus::Draft {
            @let catalog = buildkiosk_engine::cmd::catalog::can_read(state, req.actor());
            div class="card" {
                details class="disclosure" {
                    summary { "Add lines from the catalogue, an assembly, a bid or a takeoff" }
                    @if catalog {
                        h3 { "From the catalogue" }
                        @let items = buildkiosk_engine::cmd::catalog::items(state);
                        @if items.is_empty() { p class="muted" { "The catalogue is empty. " a href="/catalog" { "Add items" } "." } } @else {
                            (command_form(req, "estimate.add_from_catalog", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(eid);
                                div class="form-grid" {
                                    (field("Item", "cat_item", true, false, html! { select id="cat_item" name="item" required { @for i in items.iter().take(500) { option value=(i.id) { (i.name) " (" (i.unit_cost) @if !i.unit.is_empty() { " per " (i.unit) } ")" } } } }))
                                    (field("Quantity", "cat_qty", true, false, html! { input type="text" inputmode="decimal" id="cat_qty" name="quantity" required; }))
                                    (field("Cost code", "cat_code", false, false, html! { select id="cat_code" name="cost_code" { option value="" { "The item's own" } @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } }))
                                }
                                div class="form-actions" { button type="submit" class="btn" { "Add line" } }
                            }))
                        }
                        h3 { "An assembly" }
                        @let assemblies = buildkiosk_engine::cmd::catalog::assemblies(state);
                        @if assemblies.is_empty() { p class="muted" { "No assemblies yet." } } @else {
                            (command_form(req, "estimate.add_assembly", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(eid);
                                div class="form-grid" {
                                    (field("Assembly", "asm", true, false, html! { select id="asm" name="assembly" required { @for a in &assemblies { option value=(a.id) { (a.name) @if !a.unit.is_empty() { " (per " (a.unit) ")" } } } } }))
                                    (field("How many units", "asm_qty", true, false, html! { input type="text" inputmode="decimal" id="asm_qty" name="quantity" required; }))
                                    (field("Cost code for parts without one", "asm_code", false, false, html! { select id="asm_code" name="cost_code" { option value="" { "—" } @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } }))
                                }
                                div class="form-actions" { button type="submit" class="btn" { "Add its lines" } }
                            }))
                        }
                    }
                    @let packages: Vec<_> = state.bid_packages.iter().filter(|b| b.project == id && b.deleted.is_none() && b.cost_code.is_some() && b.bids.iter().any(|x| !x.withdrawn)).collect();
                    @if !packages.is_empty() {
                        h3 { "A bid" }
                        (command_form(req, "estimate.add_from_bid", &here, &here, html! {
                            input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(eid);
                            div class="form-grid" { (field("Bid package (its lowest bid)", "bid_pkg", true, false, html! { select id="bid_pkg" name="package" required { @for b in &packages { option value=(b.id) { (b.title) } } } })) }
                            div class="form-actions" { button type="submit" class="btn" { "Add as a lump sum" } }
                        }))
                    }
                    h3 { "A takeoff from a drawing" }
                    @let sheets: Vec<_> = state.drawings.iter().filter(|d| d.project == id && d.deleted.is_none() && d.current().is_some_and(|r| r.markups.iter().any(|m| m.kind == MarkupKind::Pin || m.kind.measures_length() || m.kind.measures_area()))).collect();
                    @if sheets.is_empty() { p class="muted" { "No sheet has measurements yet. Set a page's scale in the sheet viewer and draw lengths, areas or pins." } } @else {
                        form method="get" action=(here.clone()) class="filters" {
                            select name="takeoff" aria-label="Drawing" { @for d in &sheets { option value=(d.id) selected[takeoff == Some(d.id)] { (d.label()) } } }
                            button type="submit" class="btn small" { "Show measurements" }
                        }
                        @if let Some(d) = takeoff.and_then(|t| sheets.iter().copied().find(|d| d.id == t)) {
                            @let groups = takeoff_groups(d);
                            @for (kind, label) in &groups {
                                (command_form(req, "estimate.add_from_takeoff", &here, &here, html! {
                                    input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(eid);
                                    input type="hidden" name="drawing" value=(d.id); input type="hidden" name="kind" value=(kind.key()); input type="hidden" name="label" value=(label);
                                    div class="form-grid" {
                                        div class="field" { div class="label" { (kind.label()) ": " strong { (if label.is_empty() { "unlabelled" } else { label.as_str() }) } } }
                                        (field("Cost code", "tk_code", true, false, html! { select name="cost_code" required { @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } }))
                                        (field("Unit cost", "tk_cost", true, false, html! { input type="text" inputmode="decimal" name="unit_cost" required; }))
                                    }
                                    div class="form-actions" { button type="submit" class="btn small" { "Add the total as a line" } }
                                }))
                            }
                        }
                    }
                }
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
    let alternate_rows = cur.map(|e| e.alternates.len()).unwrap_or(0) + 2;
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit estimate" } else { "New estimate" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(e) = cur { input type="hidden" name="id" value=(e.id); }
                div class="form-grid" {
                    (field("Name", "name", true, true, html! { input type="text" id="name" name="name" required value=(cur.map(|e| e.name.as_str()).unwrap_or_default()); }))
                    (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (cur.map(|e| e.notes.as_str()).unwrap_or_default()) } }))
                    @if cur.is_none() && can_read_templates(state, req.actor()) && !templates(state).is_empty() {
                        (field("Start from a template", "template", false, false, html! { select id="template" name="template" {
                            option value="" { "Blank (use the lines below)" }
                            @for t in templates(state) { option value=(t.id) { (t.name) " (" (t.lines.len()) " lines)" } }
                        } }))
                    }
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
                h2 { "Alternates (options priced apart from the base)" }
                table class="data" {
                    thead { tr { th { "Name" } th { "Cost code" } th { "Amount (negative for a deduct)" } th { "Accepted" } } }
                    tbody { @for i in 0..alternate_rows { @let a = cur.and_then(|e| e.alternates.get(i)); tr {
                        td { input type="text" name=(format!("alternates.{i}.name")) value=(a.map(|x| x.name.as_str()).unwrap_or_default()) placeholder="Polished slab" aria-label=(format!("Alternate {} name", i + 1)); }
                        td { select name=(format!("alternates.{i}.cost_code")) aria-label=(format!("Alternate {} cost code", i + 1)) { option value="" { "—" } @for c in &codes { option value=(c.id) selected[a.is_some_and(|x| x.cost_code == c.id)] { (c.code) " " (c.description) } } } }
                        td { input type="text" inputmode="decimal" name=(format!("alternates.{i}.amount")) value=(a.map(|x| x.amount.plain()).unwrap_or_default()) aria-label=(format!("Alternate {} amount", i + 1)); }
                        td { select name=(format!("alternates.{i}.accepted")) aria-label=(format!("Alternate {} accepted", i + 1)) { option value="false" selected[a.is_none_or(|x| !x.accepted)] { "No" } option value="true" selected[a.is_some_and(|x| x.accepted)] { "Yes" } } }
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
    for a in e.alternates.iter().filter(|a| a.accepted) {
        let c = state.cost_codes.get(a.cost_code).map(|c| c.code.clone()).unwrap_or_default();
        let cells = [c, format!("Alternate: {}", a.name), String::new(), String::new(), String::new(), a.amount.plain()];
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

/// The kinds and labels of the measurements on a sheet's current revision, in the order they first appear.
fn takeoff_groups(d: &Drawing) -> Vec<(MarkupKind, String)> {
    let mut groups: Vec<(MarkupKind, String)> = Vec::new();
    for m in d.current().into_iter().flat_map(|r| &r.markups) {
        if (m.kind == MarkupKind::Pin || m.kind.measures_length() || m.kind.measures_area()) && (m.kind == MarkupKind::Pin || m.measure > 0) {
            let key = (takeoff_kind(m.kind), m.text.trim().to_string());
            if !groups.contains(&key) {
                groups.push(key);
            }
        }
    }
    groups
}

/// Two estimates side by side, cost code by cost code.
pub fn compare(req: &Req<'_>, id: Id<Project>, a: Id<Estimate>, b: Id<Estimate>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Estimating) < Level::Read {
        return not_found(req);
    }
    let find = |x: Id<Estimate>| state.estimates.get(x).filter(|e| e.project == id && e.deleted.is_none());
    let (Some(ea), Some(eb)) = (find(a), find(b)) else { return not_found(req) };
    let base = format!("/projects/{id}/estimating");
    let by_code = |e: &Estimate| -> Vec<(Id<buildkiosk_engine::model::project::CostCode>, Money)> {
        let mut out: Vec<(Id<buildkiosk_engine::model::project::CostCode>, Money)> = Vec::new();
        let lines = e.lines.iter().map(|l| (l.cost_code, l.total().unwrap_or(Money::ZERO)));
        let alts = e.alternates.iter().filter(|x| x.accepted).map(|x| (x.cost_code, x.amount));
        for (c, amt) in lines.chain(alts) {
            match out.iter_mut().find(|(x, _)| *x == c) {
                Some((_, m)) => *m = m.checked_add(amt).unwrap_or(*m),
                None => out.push((c, amt)),
            }
        }
        out
    };
    let (ma, mb) = (by_code(ea), by_code(eb));
    let mut codes: Vec<_> = ma.iter().chain(mb.iter()).map(|(c, _)| *c).collect();
    codes.sort_by_key(|c| state.cost_codes.get(*c).map(|x| x.code.clone()).unwrap_or_default());
    codes.dedup();
    let amount = |m: &[(Id<buildkiosk_engine::model::project::CostCode>, Money)], c| m.iter().find(|(x, _)| *x == c).map_or(Money::ZERO, |(_, v)| *v);
    let diff = |m: Money| html! { span class=(if m.is_negative() { "overdue" } else { "" }) { (m) } };
    let (ta, tb) = (ea.totals(), eb.totals());
    let body = html! {
        div class="page-head" {
            div { h1 { "Compare estimates" } div class="sub" { (ea.label()) ": " (ea.name) " with " (eb.label()) ": " (eb.name) } }
            div class="actions" { a class="btn" href=(format!("{base}/{a}")) { "Back to " (ea.label()) } }
        }
        div class="table-wrap" { table class="data" {
            thead { tr { th { "Cost code" } th class="num" { (ea.label()) } th class="num" { (eb.label()) } th class="num" { "Difference" } } }
            tbody { @for c in &codes { @let (x, y) = (amount(&ma, *c), amount(&mb, *c)); tr {
                td { (state.cost_codes.get(*c).map(|c| format!("{} {}", c.code, c.description)).unwrap_or_default()) }
                td class="num" { (x) } td class="num" { (y) } td class="num" { (diff(x.checked_sub(y).unwrap_or(Money::ZERO))) }
            } } }
            @if let (Some(ta), Some(tb)) = (&ta, &tb) { tfoot {
                tr { td { "Subtotal" } td class="num" { (ta.subtotal) } td class="num" { (tb.subtotal) } td class="num" { (diff(ta.subtotal.checked_sub(tb.subtotal).unwrap_or(Money::ZERO))) } }
                tr { td { "Total with markups" } td class="num" { (ta.total) } td class="num" { (tb.total) } td class="num" { (diff(ta.total.checked_sub(tb.total).unwrap_or(Money::ZERO))) } }
            } }
        } }
    };
    layout::app(
        req,
        "Compare estimates",
        Nav::Estimating(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Estimating", Some(base)), ("Compare", None)],
        body,
    )
}
