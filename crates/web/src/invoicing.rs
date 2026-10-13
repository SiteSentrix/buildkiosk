//! Owner invoice pages: the list and the payment application (detail).

use buildkiosk_core::Id;
use buildkiosk_engine::invoicedoc::{InvoiceDoc, can_edit, can_see, invoices_for};
use buildkiosk_engine::model::finance::{ChangeStatus, Commitment, CommitmentInvoice, InvoiceStatus, OwnerInvoice};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{not_found, project_for};
use buildkiosk_core::Date;

/// How one kind of invoice presents itself.
pub trait InvoicePage: InvoiceDoc {
    /// The command prefix: `owner_invoice`.
    const PREFIX: &'static str;
    /// The path under the project.
    const PATH: &'static str;
    /// Its table in the journal (for the change history).
    const TABLE: &'static str;
    const TITLE: &'static str;
    const SUBTITLE: &'static str;
    /// What is invoiced: "Prime contract" or "Commitment".
    const PARENT_LABEL: &'static str;

    fn nav(project: Id<Project>) -> Nav;
    fn period_end(&self) -> Date;
    fn paid_on(&self) -> Option<Date>;
    fn notes(&self) -> &str;
    /// The things that can be invoiced now: (id, label).
    fn parents(req: &Req<'_>, project: Id<Project>) -> Vec<(u64, String)>;
    /// A link to what the invoice bills.
    fn parent_link(&self, req: &Req<'_>) -> Markup;
}

impl InvoicePage for OwnerInvoice {
    const PREFIX: &'static str = "owner_invoice";
    const PATH: &'static str = "owner-invoices";
    const TABLE: &'static str = "owner_invoices";
    const TITLE: &'static str = "Owner invoices";
    const SUBTITLE: &'static str = "Payment applications for the work completed each period";
    const PARENT_LABEL: &'static str = "Prime contract";

    fn nav(project: Id<Project>) -> Nav {
        Nav::OwnerInvoices(project)
    }
    fn period_end(&self) -> Date {
        self.period_end
    }
    fn paid_on(&self) -> Option<Date> {
        self.paid_on
    }
    fn notes(&self) -> &str {
        &self.notes
    }
    fn parents(req: &Req<'_>, project: Id<Project>) -> Vec<(u64, String)> {
        req.engine
            .state()
            .prime_contracts
            .iter()
            .filter(|c| c.project == project && c.deleted.is_none() && c.status == ChangeStatus::Approved)
            .map(|c| (c.id.raw(), c.title.clone()))
            .collect()
    }
    fn parent_link(&self, req: &Req<'_>) -> Markup {
        html! { @if let Some(c) = req.engine.state().prime_contracts.get(self.contract) { a href=(format!("/projects/{}/prime-contracts/{}", self.project, c.id)) { (c.title) } } }
    }
}

impl InvoicePage for CommitmentInvoice {
    const PREFIX: &'static str = "commitment_invoice";
    const PATH: &'static str = "commitment-invoices";
    const TABLE: &'static str = "commitment_invoices";
    const TITLE: &'static str = "Commitment invoices";
    const SUBTITLE: &'static str = "What subcontractors and suppliers bill against their commitments";
    const PARENT_LABEL: &'static str = "Commitment";

    fn nav(project: Id<Project>) -> Nav {
        Nav::CommitmentInvoices(project)
    }
    fn period_end(&self) -> Date {
        self.period_end
    }
    fn paid_on(&self) -> Option<Date> {
        self.paid_on
    }
    fn notes(&self) -> &str {
        &self.notes
    }
    fn parents(req: &Req<'_>, project: Id<Project>) -> Vec<(u64, String)> {
        let state = req.engine.state();
        view::commitments_for(state, req.actor(), project)
            .into_iter()
            .filter(|c| c.status.counts())
            .map(|c| (c.id.raw(), format!("{}: {}", c.label(), c.title)))
            .collect()
    }
    fn parent_link(&self, req: &Req<'_>) -> Markup {
        let c: Option<&Commitment> = req.engine.state().commitments.get(self.commitment);
        html! { @if let Some(c) = c { a href=(format!("/projects/{}/commitments/{}", self.project, c.id)) { (c.label()) ": " (c.title) } } }
    }
}

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

fn crumbs<'a, T: InvoicePage>(p: &'a Project, last: Option<&'a str>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some(l) => {
            c.push((T::TITLE, Some(format!("/projects/{}/{}", p.id, T::PATH))));
            c.push((l, None));
        }
        None => c.push((T::TITLE, None)),
    }
    c
}

fn cmd<T: InvoicePage>(verb: &str) -> String {
    format!("{}.{verb}", T::PREFIX)
}

pub fn list<T: InvoicePage>(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, T::TOOL) < Level::Read {
        return not_found(req);
    }
    let all = invoices_for::<T>(state, req.actor(), id);
    let base = format!("/projects/{id}/{}", T::PATH);
    let parents = T::parents(req, id);
    let body = html! {
        div class="page-head" { div { h1 { (T::TITLE) } div class="sub" { (T::SUBTITLE) } } }
        @if all.is_empty() {
            div class="card" { (empty(tool_icon(T::TOOL), "No invoices yet", "Once there is something approved to invoice, start the first invoice below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { (T::PARENT_LABEL) } th { "Period ending" } th { "Status" } th class="num" { "This period" } th class="num" { "Retainage" } th class="num" { "Amount due" } } }
                    tbody {
                        @for i in &all {
                            @let s = i.summary(state).ok();
                            tr {
                                td class="nowrap" { (i.number()) }
                                td { (i.parent_link(req)) }
                                td { a class="row-link" href=(format!("{base}/{}", i.id())) { (i.period_end().pretty()) } }
                                td { (status_pill(i.status())) }
                                td class="num" { @if let Some(s) = &s { (s.this_period) } }
                                td class="num" { @if let Some(s) = &s { (s.retainage_this) } }
                                td class="num" { @if let Some(s) = &s { (s.net_due) } }
                            }
                        }
                    }
                }
            }
        }
        @if req.allowed(&cmd::<T>("create"), Some(id)) {
            div class="card" {
                h2 { "Start an invoice" }
                @if parents.is_empty() { p class="muted" { "Nothing is approved to invoice yet." } }
                @else {
                    (command_form(req, &cmd::<T>("create"), &format!("{base}/{{id}}"), &base, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field(T::PARENT_LABEL, "parent", true, false, html! { select id="parent" name=(T::PARENT) required { @for (pid, name) in &parents { option value=(pid) { (name) } } } }))
                            (field("Period ending", "period_end", true, false, html! { input type="date" id="period_end" name="period_end" required value=(req.today().to_string()); }))
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Start invoice" } }
                    }))
                }
            }
        }
    };
    layout::app(req, T::TITLE, T::nav(id), &crumbs::<T>(p, None), body)
}

pub fn detail<T: InvoicePage>(req: &Req<'_>, id: Id<Project>, iid: Id<T>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i)) else { return not_found(req) };
    let Ok(s) = i.summary(state) else { return not_found(req) };
    let base = format!("/projects/{id}/{}", T::PATH);
    let here = format!("{base}/{}", i.id());
    let ids = [("project", id.to_string()), ("id", i.id().to_string())];
    let level = view::level(state, req.actor(), id, T::TOOL);
    let admin = level == Level::Admin;
    let status = i.status();
    let editing = can_edit(state, req.actor(), i) && status == InvoiceStatus::Draft;
    let name = format!("Invoice #{}", i.number());
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
                h1 { (name) " " (status_pill(status)) }
                div class="sub" { "Period ending " (i.period_end().pretty()) " · " (i.parent_link(req)) @if let Some(d) = i.paid_on() { " · paid " (d.pretty()) } }
            }
            div class="actions" {
                a class="btn" href=(format!("{here}/print")) { "Print view" }
                a class="btn" href=(format!("{here}/csv")) { "Export CSV" }
                @if status == InvoiceStatus::Draft && can_edit(state, req.actor(), i) { (button_form(req, &cmd::<T>("submit"), &here, &ids, "Submit", "primary")) }
                @if status == InvoiceStatus::Submitted && admin { (button_form(req, &cmd::<T>("approve"), &here, &ids, "Approve", "primary")) (button_form(req, &cmd::<T>("reopen"), &here, &ids, "Back to draft", "")) }
                @if status == InvoiceStatus::Approved && admin { (button_form(req, &cmd::<T>("pay"), &here, &ids, "Record payment", "primary")) }
                @if admin && !matches!(status, InvoiceStatus::Paid | InvoiceStatus::Void) { (button_form(req, &cmd::<T>("void"), &here, &ids, "Void", "danger")) }
                @if level >= Level::Standard && matches!(status, InvoiceStatus::Draft | InvoiceStatus::Void) && (admin || i.created().by == Some(req.person)) { (button_form(req, &cmd::<T>("delete"), &base, &ids, "Delete", "danger")) }
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
                        (field("Period ending", "period_end", true, false, html! { input type="date" id="period_end" name="period_end" required value=(i.period_end().to_string()); }))
                        (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (i.notes()) } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save amounts" } }
                }
            } @else {
                (table)
                @if !i.notes().is_empty() { p class="prose" { (i.notes()) } }
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
    layout::app(req, &name, T::nav(id), &crumbs::<T>(p, Some(&name)), body)
}

/// A printable payment application: the schedule of values with this period's work, the amount due
/// and blank lines for signatures.
pub fn print<T: InvoicePage>(req: &Req<'_>, id: Id<Project>, iid: Id<T>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i)) else { return not_found(req) };
    let Ok(s) = i.summary(state) else { return not_found(req) };
    let company = state.company().map(|c| c.name.clone()).unwrap_or_default();
    let back = format!("/projects/{id}/{}/{}", T::PATH, i.id());
    let title = format!("Invoice #{}", i.number());
    let body = html! {
        h1 { (title) }
        p { strong { (company) } br; "Project: " (p.name) @if !p.number.is_empty() { " (" (p.number) ")" } br; (T::PARENT_LABEL) ": " (i.parent_link(req)) br; "Period ending " (i.period_end().pretty()) br; "Status: " (i.status().label()) @if let Some(d) = i.paid_on() { ", paid " (d.pretty()) } }
        table class="data print-table" {
            thead { tr { th { "Item" } th class="num" { "Scheduled value" } th class="num" { "Billed before" } th class="num" { "This period" } th class="num" { "To date" } th class="num" { "%" } th class="num" { "Balance to finish" } } }
            tbody { @for r in &s.rows {
                tr {
                    td { (r.row.description) " " span class="muted" { (state.cost_codes.get(r.row.cost_code).map(|c| c.code.clone()).unwrap_or_default()) } }
                    td class="num" { (r.row.scheduled) } td class="num" { (r.previous) } td class="num" { (r.this_period) } td class="num" { (r.to_date()) }
                    td class="num" { @if let Some(x) = r.to_date().ratio(r.row.scheduled) { (x) } } td class="num" { (r.balance()) }
                }
            } }
            tfoot { tr { td { "Total" } td class="num" { (s.scheduled) } td class="num" { (s.previous) } td class="num" { (s.this_period) } td class="num" { (s.to_date) } td {} td class="num" { (s.scheduled.checked_sub(s.to_date).unwrap_or_default()) } } }
        }
        dl class="facts" {
            dt { "Work completed this period" } dd { (s.this_period) }
            dt { "Retainage withheld (" (s.retainage_rate) ")" } dd { "−" (s.retainage_this) }
            dt { strong { "Amount due this invoice" } } dd { strong { (s.net_due) } }
            dt { "Retainage held to date" } dd { (s.retainage_to_date) }
        }
        @if !i.notes().is_empty() { p { (i.notes()) } }
        div class="signatures" {
            div { div class="line" {} "Submitted by" }
            div { div class="line" {} "Approved by" }
            div { div class="line" {} "Date" }
        }
    };
    layout::print_page(&title, &back, body)
}

/// The invoice's lines and totals as CSV.
pub fn csv<T: InvoicePage>(req: &Req<'_>, id: Id<Project>, iid: Id<T>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let i = T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i))?;
    let s = i.summary(state).ok()?;
    let mut out = String::from("Item,Cost code,Scheduled value,Billed before,This period,To date,Balance to finish\r\n");
    for r in &s.rows {
        let cells = [
            r.row.description.clone(),
            state.cost_codes.get(r.row.cost_code).map(|c| c.code.clone()).unwrap_or_default(),
            r.row.scheduled.plain(),
            r.previous.plain(),
            r.this_period.plain(),
            r.to_date().plain(),
            r.balance().plain(),
        ];
        out.push_str(&cells.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    let total = [
        "Total".to_string(),
        String::new(),
        s.scheduled.plain(),
        s.previous.plain(),
        s.this_period.plain(),
        s.to_date.plain(),
        s.scheduled.checked_sub(s.to_date).unwrap_or_default().plain(),
    ];
    out.push_str(&total.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    for (label, value) in [
        ("Retainage withheld this invoice", s.retainage_this.plain()),
        ("Amount due this invoice", s.net_due.plain()),
        ("Retainage held to date", s.retainage_to_date.plain()),
    ] {
        out.push_str(&format!("{},,,,,,{}\r\n", layout::csv_cell(label), value));
    }
    Some(out)
}
