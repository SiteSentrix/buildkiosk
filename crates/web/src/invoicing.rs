//! Owner invoice pages: the list and the payment application (detail).

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::invoice::{can_edit, can_see, invoices_for};
use buildkiosk_engine::model::finance::{ChangeStatus, InvoiceStatus, OwnerInvoice};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{not_found, project_for};

fn status_pill(s: InvoiceStatus) -> Markup {
    let class = match s {
        InvoiceStatus::Draft => "violet",
        InvoiceStatus::Submitted => "blue",
        InvoiceStatus::Approved => "amber",
        InvoiceStatus::Paid => "green",
        InvoiceStatus::Void => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn crumbs<'a>(p: &'a Project, last: Option<&'a str>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some(l) => {
            c.push(("Owner invoices", Some(format!("/projects/{}/owner-invoices", p.id))));
            c.push((l, None));
        }
        None => c.push(("Owner invoices", None)),
    }
    c
}

pub fn list(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Invoicing) < Level::Read {
        return not_found(req);
    }
    let all = invoices_for(state, req.actor(), id);
    let base = format!("/projects/{id}/owner-invoices");
    let contracts: Vec<_> =
        state.prime_contracts.iter().filter(|c| c.project == id && c.deleted.is_none() && c.status == ChangeStatus::Approved).collect();
    let body = html! {
        div class="page-head" { div { h1 { "Owner invoices" } div class="sub" { "Payment applications for the work completed each period" } } }
        @if all.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Invoicing), "No invoices yet", "Approve the prime contract, then start the first invoice below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Period ending" } th { "Status" } th class="num" { "This period" } th class="num" { "Retainage" } th class="num" { "Amount due" } } }
                    tbody {
                        @for i in &all {
                            @let s = view::invoice_summary(state, i).ok();
                            tr {
                                td class="nowrap" { (i.number) }
                                td { a class="row-link" href=(format!("{base}/{}", i.id)) { (i.period_end.pretty()) } }
                                td { (status_pill(i.status)) }
                                td class="num" { @if let Some(s) = &s { (s.this_period) } }
                                td class="num" { @if let Some(s) = &s { (s.retainage_this) } }
                                td class="num" { @if let Some(s) = &s { (s.net_due) } }
                            }
                        }
                    }
                }
            }
        }
        @if req.allowed("owner_invoice.create", Some(id)) {
            div class="card" {
                h2 { "Start an invoice" }
                @if contracts.is_empty() { p class="muted" { "There is no approved prime contract to invoice yet." } }
                @else {
                    (command_form(req, "owner_invoice.create", &format!("{base}/{{id}}"), &base, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field("Prime contract", "contract", true, false, html! { select id="contract" name="contract" required { @for c in &contracts { option value=(c.id) { (c.title) } } } }))
                            (field("Period ending", "period_end", true, false, html! { input type="date" id="period_end" name="period_end" required value=(req.today().to_string()); }))
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Start invoice" } }
                    }))
                }
            }
        }
    };
    layout::app(req, "Owner invoices", Nav::OwnerInvoices(id), &crumbs(p, None), body)
}

pub fn detail(req: &Req<'_>, id: Id<Project>, iid: Id<OwnerInvoice>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = state.owner_invoices.get(iid).filter(|i| i.project == id && can_see(state, req.actor(), i)) else { return not_found(req) };
    let Ok(s) = view::invoice_summary(state, i) else { return not_found(req) };
    let base = format!("/projects/{id}/owner-invoices");
    let here = format!("{base}/{}", i.id);
    let ids = [("project", id.to_string()), ("id", i.id.to_string())];
    let level = view::level(state, req.actor(), id, Tool::Invoicing);
    let admin = level == Level::Admin;
    let editing = can_edit(state, req.actor(), i) && i.status == InvoiceStatus::Draft;
    let name = format!("Invoice #{}", i.number);
    let contract = state.prime_contracts.get(i.contract);
    let table = html! {
        div class="table-wrap" { table class="data" {
            thead { tr { th { "Item" } th class="num" { "Scheduled value" } th class="num" { "Billed before" } th class="num" { "This period" } th class="num" { "To date" } th class="num" { "%" } th class="num" { "Balance to finish" } } }
            tbody { @for r in &s.rows {
                @let pct = r.to_date().ratio(r.row.scheduled);
                tr {
                    td { (r.row.description) " " span class="muted small" { (state.cost_codes.get(r.row.cost_code).map(|c| c.code.clone()).unwrap_or_default()) } }
                    td class="num" { (r.row.scheduled) }
                    td class="num" { (r.previous) }
                    td class="num" { @if editing { input type="text" name=(format!("amount.{}", r.row.key())) value=(if r.this_period.is_zero() { String::new() } else { r.this_period.plain() }) inputmode="decimal" aria-label=(format!("This period, {}", r.row.description)); } @else { (r.this_period) } }
                    td class="num" { (r.to_date()) }
                    td class="num" { @if let Some(x) = pct { (x) } }
                    td class="num" { (r.balance()) }
                }
            } }
            tfoot { tr { td { "Total" } td class="num" { (s.scheduled) } td class="num" { (s.previous) } td class="num" { (s.this_period) } td class="num" { (s.to_date) } td {} td class="num" { (s.scheduled.checked_sub(s.to_date).unwrap_or_default()) } } }
        } }
    };
    let body = html! {
        div class="page-head" {
            div {
                h1 { (name) " " (status_pill(i.status)) }
                div class="sub" { "Period ending " (i.period_end.pretty()) @if let Some(c) = contract { " · " a href=(format!("/projects/{id}/prime-contracts/{}", c.id)) { (c.title) } } @if let Some(d) = i.paid_on { " · paid " (d.pretty()) } }
            }
            div class="actions" {
                @if i.status == InvoiceStatus::Draft && can_edit(state, req.actor(), i) { (button_form(req, "owner_invoice.submit", &here, &ids, "Submit to owner", "primary")) }
                @if i.status == InvoiceStatus::Submitted && admin { (button_form(req, "owner_invoice.approve", &here, &ids, "Approve", "primary")) (button_form(req, "owner_invoice.reopen", &here, &ids, "Back to draft", "")) }
                @if i.status == InvoiceStatus::Approved && admin { (button_form(req, "owner_invoice.pay", &here, &ids, "Record payment", "primary")) }
                @if admin && !matches!(i.status, InvoiceStatus::Paid | InvoiceStatus::Void) { (button_form(req, "owner_invoice.void", &here, &ids, "Void", "danger")) }
                @if level >= Level::Standard && matches!(i.status, InvoiceStatus::Draft | InvoiceStatus::Void) && (admin || i.created.by == Some(req.person)) { (button_form(req, "owner_invoice.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="card" {
            h2 { "Payment application" }
            @if editing {
                form method="post" action=(format!("{here}/save")) {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    p class="muted small" { "Enter the work completed this period for each item. You can't bill an item beyond its scheduled value." }
                    (table)
                    div class="form-grid" {
                        (field("Period ending", "period_end", true, false, html! { input type="date" id="period_end" name="period_end" required value=(i.period_end.to_string()); }))
                        (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (i.notes) } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save amounts" } }
                }
            } @else {
                (table)
                @if !i.notes.is_empty() { p class="prose" { (i.notes) } }
            }
        }
        div class="card" {
            h2 { "Amount due" }
            dl class="facts" {
                dt { "Work completed this period" } dd { (s.this_period) }
                dt { "Retainage withheld (" (s.retainage_rate) ")" } dd { "−" (s.retainage_this) }
                dt { strong { "Amount due this invoice" } } dd { strong { (s.net_due) } }
                dt { "Retainage held to date" } dd { (s.retainage_to_date) }
            }
        }
        div class="card" {
            h2 { "History" }
            ul class="timeline" {
                @for e in history.iter().rev() {
                    li {
                        div { (find_command(&e.cmd).map(|c| c.label).unwrap_or(e.cmd.as_str())) " by " @if let Some(a) = e.actor { (view::person_name(state, a)) } @else { "the system" } }
                        div class="when" { (req.when(e.at)) }
                    }
                }
            }
        }
    };
    layout::app(req, &name, Nav::OwnerInvoices(id), &crumbs(p, Some(&name)), body)
}
