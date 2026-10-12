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
    Notifications,
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
        Nav::Projects | Nav::Directory | Nav::Notifications => None,
        Nav::Home(p)
        | Nav::Team(p)
        | Nav::Rfis(p)
        | Nav::Submittals(p)
        | Nav::Specs(p)
        | Nav::Inspections(p)
        | Nav::Budget(p)
        | Nav::Commitments(p)
        | Nav::DirectCosts(p)
        | Nav::ChangeEvents(p)
        | Nav::ChangeOrders(p)
        | Nav::BudgetChanges(p)
        | Nav::DailyLogs(p)
        | Nav::Punch(p)
        | Nav::Observations(p) => state.projects.get(p),
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
                        }
                        @if let Some(p) = project {
                            div class="context" { (p.name) small { (p.number) } }
                            div class="nav" {
                                a href=(format!("/projects/{}", p.id)) class=(if matches!(nav, Nav::Home(_)) { "active" } else { "" }) { (Icon::Home.svg()) "Project home" }
                                a href=(format!("/projects/{}/daily-logs", p.id)) class=(if matches!(nav, Nav::DailyLogs(_)) { "active" } else { "" }) { (tool_icon(Tool::DailyLog).svg()) "Daily log" }
                                a href=(format!("/projects/{}/punch-list", p.id)) class=(if matches!(nav, Nav::Punch(_)) { "active" } else { "" }) { (tool_icon(Tool::PunchList).svg()) "Punch list" }
                                a href=(format!("/projects/{}/observations", p.id)) class=(if matches!(nav, Nav::Observations(_)) { "active" } else { "" }) { (tool_icon(Tool::Observations).svg()) "Observations" }
                                a href=(format!("/projects/{}/inspections", p.id)) class=(if matches!(nav, Nav::Inspections(_)) { "active" } else { "" }) { (tool_icon(Tool::Inspections).svg()) "Inspections" }
                                @for (tool, path, label, current) in [
                                    (Tool::Budget, "budget", "Budget", matches!(nav, Nav::Budget(_))),
                                    (Tool::Commitments, "commitments", "Commitments", matches!(nav, Nav::Commitments(_))),
                                    (Tool::DirectCosts, "direct-costs", "Direct costs", matches!(nav, Nav::DirectCosts(_))),
                                    (Tool::ChangeEvents, "change-events", "Change events", matches!(nav, Nav::ChangeEvents(_))),
                                    (Tool::ChangeOrders, "change-orders", "Change orders", matches!(nav, Nav::ChangeOrders(_))),
                                    (Tool::ChangeOrders, "budget-changes", "Budget changes", matches!(nav, Nav::BudgetChanges(_))),
                                ] {
                                    @if view::level(state, req.actor(), p.id, tool) >= buildkiosk_engine::model::Level::Read {
                                        a href=(format!("/projects/{}/{path}", p.id)) class=(if current { "active" } else { "" }) { (tool_icon(tool).svg()) (label) }
                                    }
                                }
                                a href=(format!("/projects/{}/rfis", p.id)) class=(if matches!(nav, Nav::Rfis(_)) { "active" } else { "" }) { (tool_icon(Tool::Rfis).svg()) "RFIs" }
                                a href=(format!("/projects/{}/submittals", p.id)) class=(if matches!(nav, Nav::Submittals(_)) { "active" } else { "" }) { (tool_icon(Tool::Submittals).svg()) "Submittals" }
                                a href=(format!("/projects/{}/specifications", p.id)) class=(if matches!(nav, Nav::Specs(_)) { "active" } else { "" }) { (tool_icon(Tool::Specifications).svg()) "Specifications" }
                                a href=(format!("/projects/{}/team", p.id)) class=(if matches!(nav, Nav::Team(_)) { "active" } else { "" }) { (tool_icon(Tool::ProjectDirectory).svg()) "Team" }
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
