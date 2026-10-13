//! Pages for the cost documents (change events, commitment change orders, budget changes): list,
//! detail and form, written once over [`LineDoc`].

use buildkiosk_core::{Id, Money};
use buildkiosk_engine::costdoc::{CostDoc, can_edit, can_see, docs_for, label};
use buildkiosk_engine::model::finance::{BudgetChange, ChangeEvent, ChangeEventStatus, ChangeStatus, CommitmentChange, PrimeChange, PrimeContract};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, empty, field};
use crate::pages::{not_found, project_for};

/// How one cost document presents itself.
pub trait LineDoc: CostDoc {
    /// The command prefix: `change_event` for `change_event.create`.
    const PREFIX: &'static str;
    /// The page title: "Change events".
    const TITLE: &'static str;
    const SUBTITLE: &'static str;
    /// The path under the project.
    const PATH: &'static str;
    /// Its table in the journal (for the change history).
    const TABLE: &'static str;
    /// The plain form fields passed on to the create and update commands.
    const FIELDS: &'static [&'static str];

    fn nav(project: Id<Project>) -> Nav;
    fn title(&self) -> &str;
    fn description(&self) -> &str;
    fn status_pill(&self) -> Markup;
    /// Extra list columns: headings and cells.
    fn extra_heads() -> Markup {
        html! {}
    }
    fn extra_cells(&self, _req: &Req<'_>) -> Markup {
        html! {}
    }
    /// Facts shown above the lines.
    fn extra_facts(&self, _req: &Req<'_>) -> Markup {
        html! {}
    }
    /// The step buttons for the person (approve, close, void…).
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup;
    /// Cards shown between the details and the lines.
    fn extra_cards(&self, _req: &Req<'_>, _here: &str) -> Markup {
        html! {}
    }
    /// Form fields only this record has.
    fn extra_fields(_item: Option<&Self>, _req: &Req<'_>, _project: Id<Project>) -> Markup {
        html! {}
    }
}

fn change_event_options(req: &Req<'_>, project: Id<Project>, selected: Option<Id<ChangeEvent>>) -> Markup {
    let state = req.engine.state();
    html! {
        option value="" { "—" }
        @for e in docs_for::<ChangeEvent>(state, req.actor(), project) { option value=(e.id) selected[Some(e.id) == selected] { (label(e)) ": " (e.title) } }
    }
}

fn event_field(req: &Req<'_>, project: Id<Project>, selected: Option<Id<ChangeEvent>>) -> Markup {
    field(
        "From change event",
        "change_event",
        false,
        true,
        html! { select id="change_event" name="change_event" { (change_event_options(req, project, selected)) } },
    )
}

fn event_link(req: &Req<'_>, project: Id<Project>, event: Option<Id<ChangeEvent>>) -> Markup {
    let Some(e) = event.and_then(|e| req.engine.state().change_events.get(e)) else { return html! {} };
    html! { dt { "Change event" } dd { a href=(format!("/projects/{project}/change-events/{}", e.id)) { (label(e)) ": " (e.title) } } }
}

impl LineDoc for ChangeEvent {
    const PREFIX: &'static str = "change_event";
    const TITLE: &'static str = "Change events";
    const SUBTITLE: &'static str = "Possible changes to cost, tracked while they are priced";
    const PATH: &'static str = "change-events";
    const TABLE: &'static str = "change_events";
    const FIELDS: &'static [&'static str] = &["title", "description", "reason"];

    fn nav(project: Id<Project>) -> Nav {
        Nav::ChangeEvents(project)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn status_pill(&self) -> Markup {
        let class = match self.status {
            ChangeEventStatus::Open => "blue",
            ChangeEventStatus::Closed => "green",
            ChangeEventStatus::Void => "red",
        };
        html! { span class=(format!("pill {class}")) { (self.status.label()) } }
    }
    fn extra_facts(&self, _req: &Req<'_>) -> Markup {
        html! { @if !self.reason.is_empty() { dt { "Reason" } dd { (self.reason) } } }
    }
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup {
        let state = req.engine.state();
        let admin = view::level(state, req.actor(), self.project, buildkiosk_engine::model::Tool::ChangeEvents) == Level::Admin;
        html! {
            @if self.status == ChangeEventStatus::Open && can_edit(state, req.actor(), self) { (button_form(req, "change_event.close", here, ids, "Close", "primary")) }
            @if self.status == ChangeEventStatus::Closed && admin { (button_form(req, "change_event.reopen", here, ids, "Reopen", "")) }
            @if self.status != ChangeEventStatus::Void && admin { (button_form(req, "change_event.void", here, ids, "Void", "danger")) }
        }
    }
    fn extra_fields(item: Option<&Self>, _req: &Req<'_>, _project: Id<Project>) -> Markup {
        field(
            "Reason",
            "reason",
            false,
            true,
            html! { input type="text" id="reason" name="reason" placeholder="Owner request, field condition, design change…" value=(item.map(|i| i.reason.clone()).unwrap_or_default()); },
        )
    }
}

fn change_pill(s: ChangeStatus) -> Markup {
    let class = match s {
        ChangeStatus::Draft => "violet",
        ChangeStatus::Approved => "green",
        ChangeStatus::Void => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn change_actions<T: CostDoc>(req: &Req<'_>, d: &T, prefix: &str, status: ChangeStatus, here: &str, ids: &[(&str, String)]) -> Markup {
    let admin = view::level(req.engine.state(), req.actor(), d.project(), T::TOOL) == Level::Admin;
    html! {
        @if status == ChangeStatus::Draft && admin { (button_form(req, &format!("{prefix}.approve"), here, ids, "Approve", "primary")) }
        @if status != ChangeStatus::Void && admin { (button_form(req, &format!("{prefix}.void"), here, ids, "Void", "danger")) }
    }
}

impl LineDoc for CommitmentChange {
    const PREFIX: &'static str = "commitment_change";
    const TITLE: &'static str = "Commitment change orders";
    const SUBTITLE: &'static str = "Approved changes to what a subcontract or purchase order costs";
    const PATH: &'static str = "change-orders";
    const TABLE: &'static str = "commitment_changes";
    const FIELDS: &'static [&'static str] = &["title", "description", "commitment", "change_event"];

    fn nav(project: Id<Project>) -> Nav {
        Nav::ChangeOrders(project)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn status_pill(&self) -> Markup {
        change_pill(self.status)
    }
    fn extra_heads() -> Markup {
        html! { th { "Commitment" } }
    }
    fn extra_cells(&self, req: &Req<'_>) -> Markup {
        html! { td { @if let Some(c) = req.engine.state().commitments.get(self.commitment) { (c.label()) ": " (c.title) } } }
    }
    fn extra_facts(&self, req: &Req<'_>) -> Markup {
        html! {
            @if let Some(c) = req.engine.state().commitments.get(self.commitment) { dt { "Commitment" } dd { a href=(format!("/projects/{}/commitments/{}", self.project, c.id)) { (c.label()) ": " (c.title) } } }
            (event_link(req, self.project, self.change_event))
        }
    }
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup {
        change_actions(req, self, "commitment_change", self.status, here, ids)
    }
    fn extra_fields(item: Option<&Self>, req: &Req<'_>, project: Id<Project>) -> Markup {
        let state = req.engine.state();
        let commitments: Vec<_> = view::commitments_for(state, req.actor(), project).into_iter().filter(|c| c.status.counts()).collect();
        html! {
            @if let Some(i) = item {
                input type="hidden" name="commitment" value=(i.commitment);
            } @else {
                (field("Commitment", "commitment", true, true, html! { select id="commitment" name="commitment" required { @for c in &commitments { option value=(c.id) { (c.label()) ": " (c.title) } } } }))
            }
            (event_field(req, project, item.and_then(|i| i.change_event)))
        }
    }
}

impl LineDoc for BudgetChange {
    const PREFIX: &'static str = "budget_change";
    const TITLE: &'static str = "Budget changes";
    const SUBTITLE: &'static str = "Approved changes to the budget: add, remove or move money between cost codes";
    const PATH: &'static str = "budget-changes";
    const TABLE: &'static str = "budget_changes";
    const FIELDS: &'static [&'static str] = &["title", "description", "change_event"];

    fn nav(project: Id<Project>) -> Nav {
        Nav::BudgetChanges(project)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn status_pill(&self) -> Markup {
        change_pill(self.status)
    }
    fn extra_facts(&self, req: &Req<'_>) -> Markup {
        event_link(req, self.project, self.change_event)
    }
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup {
        change_actions(req, self, "budget_change", self.status, here, ids)
    }
    fn extra_fields(item: Option<&Self>, req: &Req<'_>, project: Id<Project>) -> Markup {
        event_field(req, project, item.and_then(|i| i.change_event))
    }
}

impl LineDoc for PrimeContract {
    const PREFIX: &'static str = "prime_contract";
    const TITLE: &'static str = "Prime contract";
    const SUBTITLE: &'static str = "What the owner pays: the schedule of values that gets invoiced";
    const PATH: &'static str = "prime-contracts";
    const TABLE: &'static str = "prime_contracts";
    const FIELDS: &'static [&'static str] = &["title", "description", "owner", "retainage"];

    fn nav(project: Id<Project>) -> Nav {
        Nav::PrimeContracts(project)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn status_pill(&self) -> Markup {
        change_pill(self.status)
    }
    fn extra_facts(&self, req: &Req<'_>) -> Markup {
        let state = req.engine.state();
        let summary = view::contract_summary(state, self).ok();
        html! {
            @if let Some(o) = self.owner { dt { "Owner" } dd { (view::vendor_name(state, o)) } }
            dt { "Retainage" } dd { @if let Some(r) = self.retainage { (r) } @else { "None" } }
            @if let Some(c) = summary {
                dt { "Original contract" } dd { (c.original) }
                dt { "Approved changes" } dd { (c.changes) }
                dt { "Revised contract" } dd { strong { (c.revised) } }
                dt { "Billed to date" } dd { (c.billed) }
                dt { "Retainage held" } dd { (c.retainage_held) }
                dt { "Paid (net)" } dd { (c.paid) }
                dt { "Balance to finish" } dd { (c.balance_to_finish) }
            }
            dt { "Related" } dd {
                a href=(format!("/projects/{}/prime-change-orders", self.project)) { "Prime change orders" } " · "
                a href=(format!("/projects/{}/owner-invoices", self.project)) { "Owner invoices" }
            }
        }
    }
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup {
        change_actions(req, self, "prime_contract", self.status, here, ids)
    }
    fn extra_cards(&self, req: &Req<'_>, here: &str) -> Markup {
        crate::finance::signature_card(
            req,
            "prime_contract",
            self.project,
            self.id.raw(),
            &self.signature,
            (self.status != ChangeStatus::Void, self.status == ChangeStatus::Approved),
            here,
        )
    }
    fn extra_fields(item: Option<&Self>, req: &Req<'_>, _project: Id<Project>) -> Markup {
        let state = req.engine.state();
        let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
        vendors.sort_by_key(|v| v.name.to_lowercase());
        let owner = item.and_then(|i| i.owner);
        html! {
            (field("Owner", "owner", false, false, html! { select id="owner" name="owner" { option value="" { "—" } @for v in &vendors { option value=(v.id) selected[Some(v.id) == owner] { (v.name) } } } }))
            (field("Retainage (%)", "retainage", false, false, html! { input type="text" id="retainage" name="retainage" inputmode="decimal" placeholder="10" value=(item.and_then(|i| i.retainage).map(|r| r.to_string().trim_end_matches('%').to_string()).unwrap_or_default()); }))
        }
    }
}

impl LineDoc for PrimeChange {
    const PREFIX: &'static str = "prime_change";
    const TITLE: &'static str = "Prime change orders";
    const SUBTITLE: &'static str = "Approved changes to the owner's contract; approved lines become billable";
    const PATH: &'static str = "prime-change-orders";
    const TABLE: &'static str = "prime_changes";
    const FIELDS: &'static [&'static str] = &["title", "description", "contract", "change_event"];

    fn nav(project: Id<Project>) -> Nav {
        Nav::PrimeContracts(project)
    }
    fn title(&self) -> &str {
        &self.title
    }
    fn description(&self) -> &str {
        &self.description
    }
    fn status_pill(&self) -> Markup {
        change_pill(self.status)
    }
    fn extra_heads() -> Markup {
        html! { th { "Contract" } }
    }
    fn extra_cells(&self, req: &Req<'_>) -> Markup {
        html! { td { @if let Some(c) = req.engine.state().prime_contracts.get(self.contract) { (c.title) } } }
    }
    fn extra_facts(&self, req: &Req<'_>) -> Markup {
        html! {
            @if let Some(c) = req.engine.state().prime_contracts.get(self.contract) { dt { "Contract" } dd { a href=(format!("/projects/{}/prime-contracts/{}", self.project, c.id)) { (c.title) } } }
            (event_link(req, self.project, self.change_event))
        }
    }
    fn actions(&self, req: &Req<'_>, here: &str, ids: &[(&str, String)]) -> Markup {
        change_actions(req, self, "prime_change", self.status, here, ids)
    }
    fn extra_cards(&self, req: &Req<'_>, here: &str) -> Markup {
        crate::finance::signature_card(
            req,
            "prime_change",
            self.project,
            self.id.raw(),
            &self.signature,
            (self.status != ChangeStatus::Void, self.status == ChangeStatus::Approved),
            here,
        )
    }
    fn extra_fields(item: Option<&Self>, req: &Req<'_>, project: Id<Project>) -> Markup {
        let state = req.engine.state();
        let contracts: Vec<_> = buildkiosk_engine::costdoc::docs_for::<PrimeContract>(state, req.actor(), project)
            .into_iter()
            .filter(|c| c.status == ChangeStatus::Approved)
            .collect();
        html! {
            @if let Some(i) = item {
                input type="hidden" name="contract" value=(i.contract);
            } @else {
                (field("Prime contract", "contract", true, true, html! { select id="contract" name="contract" required { @for c in &contracts { option value=(c.id) { (c.title) } } } }))
            }
            (event_field(req, project, item.and_then(|i| i.change_event)))
        }
    }
}

fn crumbs<'a, T: LineDoc>(p: &'a Project, last: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some((l, list)) => {
            c.push((T::TITLE, Some(list)));
            c.push((l, None));
        }
        None => c.push((T::TITLE, None)),
    }
    c
}

fn total<T: CostDoc>(d: &T) -> Option<Money> {
    Money::sum(d.lines().iter().map(|l| l.amount))
}

fn cmd<T: LineDoc>(verb: &str) -> String {
    format!("{}.{verb}", T::PREFIX)
}

pub fn list<T: LineDoc>(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, T::TOOL) < Level::Read {
        return not_found(req);
    }
    let all = docs_for::<T>(state, req.actor(), id);
    let base = format!("/projects/{id}/{}", T::PATH);
    let body = html! {
        div class="page-head" {
            div { h1 { (T::TITLE) } div class="sub" { (T::SUBTITLE) } }
            div class="actions" { @if req.allowed(&cmd::<T>("create"), Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "New" } } }
        }
        @if all.is_empty() {
            div class="card" { (empty(tool_icon(T::TOOL), "Nothing here yet", "Create the first one to get started.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Title" } (T::extra_heads()) th { "Status" } th class="num" { "Amount" } } }
                    tbody {
                        @for d in &all {
                            tr {
                                td class="nowrap" { (d.number()) }
                                td { a class="row-link" href=(format!("{base}/{}", d.id())) { (d.title()) } }
                                (d.extra_cells(req))
                                td { (d.status_pill()) }
                                td class="num" { @if let Some(t) = total(*d) { (t) } }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, T::TITLE, T::nav(id), &crumbs::<T>(p, None), body)
}

pub fn detail<T: LineDoc>(req: &Req<'_>, id: Id<Project>, did: Id<T>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(d) = T::table(state).get(did).filter(|d| d.project() == id && can_see(state, req.actor(), *d)) else { return not_found(req) };
    let base = format!("/projects/{id}/{}", T::PATH);
    let here = format!("{base}/{}", d.id());
    let ids = [("project", id.to_string()), ("id", d.id().to_string())];
    let level = view::level(state, req.actor(), id, T::TOOL);
    let name = label(d);
    let body = html! {
        div class="page-head" {
            div {
                h1 { (name) ": " (d.title()) " " (d.status_pill()) }
                div class="sub" { "Created " (req.when(d.created().at)) @if let Some(b) = d.created().by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if can_edit(state, req.actor(), d) { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                (d.actions(req, &here, &ids))
                @if level >= Level::Standard && (level == Level::Admin || d.is_editable()) && !d.is_void() { (button_form(req, &cmd::<T>("delete"), &base, &ids, "Delete", "danger")) }
            }
        }
        @if !d.description().is_empty() { div class="card" { h2 { "Description" } p class="prose" { (d.description()) } } }
        div class="card" {
            h2 { "Details" }
            dl class="facts" { (d.extra_facts(req)) }
        }
        (d.extra_cards(req, &here))
        div class="card" {
            h2 { "Lines" }
            @if d.lines().is_empty() { p class="muted" { "No lines yet." } }
            @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "#" } th { "Cost code" } th { "Description" } th class="num" { "Amount" } } }
                    tbody { @for (n, l) in d.lines().iter().enumerate() {
                        tr { td { (n + 1) } td { (state.cost_codes.get(l.cost_code).map(|c| c.display()).unwrap_or_default()) } td { (l.description) } td class="num" { (l.amount) } }
                    } }
                    tfoot { tr { td colspan="3" { "Total" } td class="num" { @if let Some(t) = total(d) { (t) } } } }
                } }
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
    layout::app(req, &name, T::nav(id), &crumbs::<T>(p, Some((&name, base.clone()))), body)
}

pub fn form<T: LineDoc>(req: &Req<'_>, id: Id<Project>, existing: Option<Id<T>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(did) => {
            match T::table(state).get(did).filter(|d| d.project() == id && can_see(state, req.actor(), *d) && can_edit(state, req.actor(), *d)) {
                Some(d) => Some(d),
                None => return not_found(req),
            }
        }
        None => {
            if !req.allowed(&cmd::<T>("create"), Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let base = format!("/projects/{id}/{}", T::PATH);
    let action = match current {
        Some(d) => format!("{base}/{}/save", d.id()),
        None => format!("{base}/save"),
    };
    let lines = current.map(|d| d.lines()).unwrap_or(&[]);
    let codes = view::project_cost_codes(state, id);
    let title = if current.is_some() { format!("Edit {}", T::NOUN) } else { format!("New {}", T::NOUN) };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        form method="post" action=(action) {
            input type="hidden" name="_csrf" value=(req.csrf);
            div class="card" {
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(current.map(|d| d.title().to_string()).unwrap_or_default()); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (current.map(|d| d.description().to_string()).unwrap_or_default()) } }))
                    (T::extra_fields(current, req, id))
                }
            }
            div class="card" {
                h2 { "Lines" }
                p class="muted small" { "One line per cost code. Amounts may be negative to take money away. Leave spare rows blank." }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Cost code" } th { "Description" } th { "Amount" } } }
                    tbody { @for i in 0..(lines.len() + 3).max(5) {
                        @let l = lines.get(i);
                        tr {
                            td { select name=(format!("lines.{i}.cost_code")) aria-label=(format!("Cost code, line {}", i + 1)) {
                                option value="" { "—" }
                                @for c in &codes { option value=(c.id) selected[l.is_some_and(|l| l.cost_code == c.id)] { (c.code) " " (c.description) } }
                            } }
                            td { input type="text" name=(format!("lines.{i}.description")) value=(l.map(|l| l.description.clone()).unwrap_or_default()) aria-label=(format!("Description, line {}", i + 1)); }
                            td { input type="text" name=(format!("lines.{i}.amount")) value=(l.map(|l| l.amount.plain()).unwrap_or_default()) inputmode="decimal" aria-label=(format!("Amount, line {}", i + 1)); }
                        }
                    } }
                } }
            }
            div class="form-actions" {
                a class="btn" href=(current.map(|d| format!("{base}/{}", d.id())).unwrap_or_else(|| base.clone())) { "Cancel" }
                button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { "Save" }) }
            }
        }
    };
    layout::app(req, &title, T::nav(id), &crumbs::<T>(p, Some((&title, base.clone()))), body)
}
