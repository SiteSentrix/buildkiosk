//! The page frame: top bar, navigation, forms and small shared widgets.

use buildkiosk_core::{Date, Id, Timestamp};
use buildkiosk_engine::Engine;
use buildkiosk_engine::model::directory::Person;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::{Actor, model::Tool, view};
use maud::{DOCTYPE, Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::style;

/// A one-off message shown above the page (the result of the last action).
#[derive(Clone, Debug)]
pub struct Flash {
    pub ok: bool,
    pub msg: String,
}

/// What a page needs to know about the request.
pub struct Req<'a> {
    pub engine: &'a Engine,
    pub person: Id<Person>,
    /// The CSRF token to put in every form.
    pub csrf: &'a str,
    pub now: Timestamp,
    pub flash: Option<Flash>,
}

impl Req<'_> {
    pub fn actor(&self) -> Actor {
        Actor::Person(self.person)
    }

    /// Today's date at the company's UTC offset.
    pub fn today(&self) -> Date {
        self.now.date_at(self.engine.state().utc_offset())
    }

    pub fn allowed(&self, command: &str, project: Option<Id<Project>>) -> bool {
        self.engine.allowed(self.actor(), command, project)
    }

    pub fn when(&self, t: Timestamp) -> String {
        t.pretty_at(self.engine.state().utc_offset())
    }
}

/// The project tools in the sidebar, by group: tool, path, label and the level needed to see it.
type NavEntry = (Tool, &'static str, &'static str, buildkiosk_engine::model::Level);

const NAV_GROUPS: &[(&str, &[NavEntry])] = {
    use buildkiosk_engine::model::Level::{Read, Standard};
    &[
        (
            "Project management",
            &[
                (Tool::Rfis, "rfis", "RFIs", Read),
                (Tool::Submittals, "submittals", "Submittals", Read),
                (Tool::Transmittals, "transmittals", "Transmittals", Read),
                (Tool::Correspondence, "correspondence", "Correspondence", Read),
                (Tool::Meetings, "meetings", "Meetings", Read),
                (Tool::Forms, "forms", "Forms", Read),
                (Tool::DailyLog, "daily-logs", "Daily log", Read),
                (Tool::Tasks, "tasks", "Tasks", Read),
                (Tool::ActionPlans, "action-plans", "Action plans", Read),
            ],
        ),
        (
            "Quality and safety",
            &[
                (Tool::PunchList, "punch-list", "Punch list", Read),
                (Tool::Observations, "observations", "Observations", Read),
                (Tool::Inspections, "inspections", "Inspections", Read),
                (Tool::Incidents, "incidents", "Incidents", Read),
            ],
        ),
        (
            "Plans and files",
            &[
                (Tool::Schedule, "schedule", "Schedule", Read),
                (Tool::Drawings, "drawings", "Drawings", Read),
                (Tool::Specifications, "specifications", "Specifications", Read),
                (Tool::Documents, "documents", "Documents", Read),
                (Tool::Photos, "photos", "Photos", Read),
            ],
        ),
        (
            "Financials",
            &[
                (Tool::Budget, "budget", "Budget", Read),
                (Tool::Commitments, "commitments", "Commitments", Read),
                (Tool::DirectCosts, "direct-costs", "Direct costs", Read),
                (Tool::PrimeContracts, "prime-contracts", "Prime contract", Read),
                (Tool::Invoicing, "owner-invoices", "Owner invoices", Read),
                (Tool::Invoicing, "commitment-invoices", "Commitment invoices", Read),
                (Tool::ChangeEvents, "change-events", "Change events", Read),
                (Tool::ChangeOrders, "change-orders", "Change orders", Read),
                (Tool::ChangeOrders, "budget-changes", "Budget changes", Read),
            ],
        ),
        ("Preconstruction", &[(Tool::Estimating, "estimating", "Estimating", Read), (Tool::Bidding, "bidding", "Bidding", Read)]),
        ("Resources", &[(Tool::Timesheets, "timesheets", "Timesheets", Standard)]),
        ("Project", &[(Tool::Reports, "reports", "Report", Read), (Tool::ProjectDirectory, "team", "Team", Read)]),
    ]
};

/// The path segment of the current page's sidebar entry ("" when it has none).
fn nav_key(nav: Nav) -> &'static str {
    match nav {
        Nav::Projects | Nav::Directory | Nav::Notifications | Nav::Prequalification | Nav::Home(_) => "",
        Nav::Team(_) => "team",
        Nav::Rfis(_) => "rfis",
        Nav::Submittals(_) => "submittals",
        Nav::Specs(_) => "specifications",
        Nav::Documents(_) => "documents",
        Nav::Transmittals(_) => "transmittals",
        Nav::Meetings(_) => "meetings",
        Nav::Tasks(_) => "tasks",
        Nav::Incidents(_) => "incidents",
        Nav::Photos(_) => "photos",
        Nav::Correspondence(_) => "correspondence",
        Nav::Forms(_) => "forms",
        Nav::Timesheets(_) => "timesheets",
        Nav::Reports(_) => "reports",
        Nav::ActionPlans(_) => "action-plans",
        Nav::Bidding(_) => "bidding",
        Nav::Estimating(_) => "estimating",
        Nav::Drawings(_) => "drawings",
        Nav::Schedule(_) => "schedule",
        Nav::DailyLogs(_) => "daily-logs",
        Nav::Punch(_) => "punch-list",
        Nav::Observations(_) => "observations",
        Nav::Inspections(_) => "inspections",
        Nav::Budget(_) => "budget",
        Nav::Commitments(_) => "commitments",
        Nav::DirectCosts(_) => "direct-costs",
        Nav::ChangeEvents(_) => "change-events",
        Nav::ChangeOrders(_) => "change-orders",
        Nav::BudgetChanges(_) => "budget-changes",
        Nav::PrimeContracts(_) => "prime-contracts",
        Nav::OwnerInvoices(_) => "owner-invoices",
        Nav::CommitmentInvoices(_) => "commitment-invoices",
    }
}

/// Which navigation entry is current.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    Projects,
    Directory,
    Home(Id<Project>),
    Team(Id<Project>),
    Rfis(Id<Project>),
    Submittals(Id<Project>),
    Specs(Id<Project>),
    Documents(Id<Project>),
    Transmittals(Id<Project>),
    Meetings(Id<Project>),
    Tasks(Id<Project>),
    Incidents(Id<Project>),
    Photos(Id<Project>),
    Correspondence(Id<Project>),
    Forms(Id<Project>),
    Timesheets(Id<Project>),
    Reports(Id<Project>),
    ActionPlans(Id<Project>),
    Bidding(Id<Project>),
    Estimating(Id<Project>),
    Drawings(Id<Project>),
    Schedule(Id<Project>),
    DailyLogs(Id<Project>),
    Punch(Id<Project>),
    Observations(Id<Project>),
    Inspections(Id<Project>),
    Budget(Id<Project>),
    Commitments(Id<Project>),
    DirectCosts(Id<Project>),
    ChangeEvents(Id<Project>),
    ChangeOrders(Id<Project>),
    BudgetChanges(Id<Project>),
    PrimeContracts(Id<Project>),
    OwnerInvoices(Id<Project>),
    CommitmentInvoices(Id<Project>),
    Notifications,
    Prequalification,
}

fn head(title: &str) -> Markup {
    html! {
        meta charset="utf-8";
        meta name="viewport" content="width=device-width, initial-scale=1";
        title { (title) " · BuildKiosk" }
        link rel="stylesheet" href=(format!("/static/app.css?v={}", style::version()));
        link rel="icon" href="/static/favicon.svg" type="image/svg+xml";
    }
}

/// A bare page (sign-in and first-run setup).
pub fn auth_page(title: &str, error: Option<&str>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head { (head(title)) }
            body {
                main class="auth-wrap" {
                    div class="auth-card" {
                        div class="brand" { (Icon::Logo.svg()) "BuildKiosk" }
                        h1 { (title) }
                        @if let Some(e) = error { div class="flash error" role="alert" { (e) } }
                        (body)
                    }
                }
            }
        }
    }
}

/// A page for printing: no navigation, just the content and a link back.
pub fn print_page(title: &str, back: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head { (head(title)) }
            body class="print-body" {
                main class="sheet" {
                    p class="no-print" { a href=(back) { "← Back" } " · use the browser's Print command (Ctrl+P) to print or save as PDF" }
                    (body)
                }
            }
        }
    }
}

fn initials(name: &str) -> String {
    name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()
}

/// The full page: top bar, sidebar, flash message and content.
pub fn app(req: &Req<'_>, title: &str, nav: Nav, crumbs: &[(&str, Option<String>)], body: Markup) -> Markup {
    let state = req.engine.state();
    let me = view::person_name(state, req.person);
    let company = state.company().map(|c| c.name.clone()).unwrap_or_default();
    let unread = state.notifications.iter().filter(|n| n.person == req.person && !n.read).count();
    let project = match nav {
        Nav::Projects | Nav::Directory | Nav::Notifications | Nav::Prequalification => None,
        Nav::Home(p)
        | Nav::Team(p)
        | Nav::Rfis(p)
        | Nav::Submittals(p)
        | Nav::Specs(p)
        | Nav::Documents(p)
        | Nav::Transmittals(p)
        | Nav::Meetings(p)
        | Nav::Drawings(p)
        | Nav::Schedule(p)
        | Nav::Inspections(p)
        | Nav::Budget(p)
        | Nav::Commitments(p)
        | Nav::DirectCosts(p)
        | Nav::ChangeEvents(p)
        | Nav::ChangeOrders(p)
        | Nav::BudgetChanges(p)
        | Nav::PrimeContracts(p)
        | Nav::OwnerInvoices(p)
        | Nav::CommitmentInvoices(p)
        | Nav::DailyLogs(p)
        | Nav::Punch(p)
        | Nav::Observations(p)
        | Nav::Tasks(p)
        | Nav::Incidents(p)
        | Nav::Photos(p)
        | Nav::Correspondence(p)
        | Nav::Forms(p)
        | Nav::Timesheets(p)
        | Nav::Reports(p)
        | Nav::ActionPlans(p)
        | Nav::Bidding(p)
        | Nav::Estimating(p) => state.projects.get(p),
    };
    html! {
        (DOCTYPE)
        html lang="en" {
            head { (head(title)) }
            body {
                a class="skip" href="#main" { "Skip to content" }
                input type="checkbox" id="nav-toggle" class="nav-toggle";
                header class="topbar" {
                    label for="nav-toggle" class="menu-button" aria-label="Menu" { (Icon::Menu.svg()) }
                    a class="brand" href="/" { (Icon::Logo.svg()) "BuildKiosk" }
                    span class="company" { (company) }
                    span class="spacer" {}
                    div class="who" {
                        a href="/notifications" aria-label=(format!("Notifications, {unread} unread")) { (Icon::Mail.svg()) @if unread > 0 { span class="tag" { (unread) } } }
                        span class="avatar" { (initials(&me)) }
                        span class="name" { (me) }
                        form method="post" action="/logout" {
                            input type="hidden" name="_csrf" value=(req.csrf);
                            button type="submit" { (Icon::SignOut.svg()) "Sign out" }
                        }
                    }
                }
                div class="shell" {
                    nav class="sidebar" aria-label="Main" {
                        div class="nav" {
                            a href="/" class=(if nav == Nav::Projects { "active" } else { "" }) { (Icon::Building.svg()) "Projects" }
                            @if req.allowed("person.list", None) {
                                a href="/directory" class=(if nav == Nav::Directory { "active" } else { "" }) { (Icon::People.svg()) "Directory" }
                            }
                            @if buildkiosk_engine::model::perm::company_level(state, req.person, Tool::Prequalification) >= buildkiosk_engine::model::Level::Read {
                                a href="/prequalification" class=(if nav == Nav::Prequalification { "active" } else { "" }) { (tool_icon(Tool::Prequalification).svg()) "Prequalification" }
                            }
                        }
                        @if let Some(p) = project {
                            div class="context" { (p.name) small { (p.number) } }
                            div class="nav" {
                                a href=(format!("/projects/{}", p.id)) class=(if matches!(nav, Nav::Home(_)) { "active" } else { "" }) { (Icon::Home.svg()) "Project home" }
                                @for (gi, (group, entries)) in NAV_GROUPS.iter().enumerate() {
                                    @let visible: Vec<&NavEntry> = entries.iter().filter(|(tool, _, _, min)| view::level(state, req.actor(), p.id, *tool) >= *min).collect();
                                    @if !visible.is_empty() {
                                        details class="nav-section" open[visible.iter().any(|(_, path, _, _)| *path == nav_key(nav)) || (gi == 0 && matches!(nav, Nav::Home(_)))] {
                                            summary { (group) }
                                            @for (tool, path, label, _) in visible {
                                                a href=(format!("/projects/{}/{path}", p.id)) class=(if *path == nav_key(nav) { "active" } else { "" }) { (tool_icon(*tool).svg()) (label) }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    main class="main" id="main" {
                        div class="content" {
                            @if !crumbs.is_empty() {
                                div class="crumbs" {
                                    @for (i, (label, href)) in crumbs.iter().enumerate() {
                                        @if i > 0 { span { "/" } }
                                        @if let Some(h) = href { a href=(h) { (label) } } @else { span { (label) } }
                                    }
                                }
                            }
                            @if let Some(f) = &req.flash {
                                div class=(if f.ok { "flash ok" } else { "flash error" }) role="status" { (f.msg) }
                            }
                            (body)
                        }
                    }
                }
            }
        }
    }
}

/// A form that runs `command` and then goes to `next` (`{id}` in it becomes the new record's id).
/// On failure the person returns to `back` with the error shown.
pub fn command_form(req: &Req<'_>, command: &str, next: &str, back: &str, inner: Markup) -> Markup {
    html! {
        form method="post" action=(format!("/do/{command}")) {
            input type="hidden" name="_csrf" value=(req.csrf);
            input type="hidden" name="_next" value=(next);
            input type="hidden" name="_back" value=(back);
            (inner)
        }
    }
}

/// A one-button form (open, close, delete…).
pub fn button_form(req: &Req<'_>, command: &str, next: &str, fields: &[(&str, String)], label: &str, class: &str) -> Markup {
    command_form(
        req,
        command,
        next,
        next,
        html! {
            @for (k, v) in fields { input type="hidden" name=(k) value=(v); }
            button type="submit" class=(format!("btn {class}")) { (label) }
        },
    )
}

/// One CSV cell: spreadsheet formulas in text are defused and special characters quoted.
pub(crate) fn csv_cell(s: &str) -> String {
    let s = if s.starts_with(['=', '+', '-', '@', '\t', '\r']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s }
}

/// An empty-state panel.
pub fn empty(icon: Icon, title: &str, text: &str) -> Markup {
    html! { div class="empty" { (icon.svg()) h3 { (title) } p { (text) } } }
}

/// A labelled form field.
pub fn field(label: &str, id: &str, required: bool, full: bool, input: Markup) -> Markup {
    html! {
        div class=(if full { "field full" } else { "field" }) {
            label for=(id) { (label) @if required { " " span class="req" { "*" } } }
            (input)
        }
    }
}
