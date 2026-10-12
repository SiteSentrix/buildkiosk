//! The pages: sign-in, setup, projects, project home, team and RFIs.

use buildkiosk_core::Id;
use buildkiosk_engine::model::directory::Person;
use buildkiosk_engine::model::perm::ProjectMember;
use buildkiosk_engine::model::project::{Project, ProjectStage};
use buildkiosk_engine::model::rfi::{Rfi, RfiStatus};
use buildkiosk_engine::model::{Choice, Impact, Level, Tool, perm};
use buildkiosk_engine::{cmd::rfi::can_edit, view};
use maud::{Markup, html};

use crate::icons::Icon;
use crate::layout::{self, Nav, Req, auth_page, button_form, command_form, empty, field};

fn stage_class(s: ProjectStage) -> &'static str {
    match s {
        ProjectStage::Construction => "blue",
        ProjectStage::Complete | ProjectStage::Closeout | ProjectStage::Warranty => "green",
        ProjectStage::OnHold | ProjectStage::Cancelled => "amber",
        _ => "violet",
    }
}

fn status_pill(s: RfiStatus) -> Markup {
    let class = match s {
        RfiStatus::Draft => "violet",
        RfiStatus::Open => "blue",
        RfiStatus::Closed => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

// -- sign-in and setup

pub fn login(csrf: &str, company: &str, email: &str, error: Option<&str>) -> Markup {
    auth_page(
        "Sign in",
        error,
        html! {
            p class="muted" { (company) }
            form method="post" action="/login" class="stack" {
                input type="hidden" name="_csrf" value=(csrf);
                div class="field" { label for="email" { "Email" } input type="email" id="email" name="email" value=(email) required autofocus autocomplete="username"; }
                div class="field" { label for="password" { "Password" } input type="password" id="password" name="password" required autocomplete="current-password"; }
                button type="submit" class="btn primary" { "Sign in" }
            }
        },
    )
}

pub fn setup(csrf: &str, values: &[(String, String)], error: Option<&str>) -> Markup {
    let v = |k: &str| values.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone()).unwrap_or_default();
    auth_page(
        "Set up your company",
        error,
        html! {
            p class="muted" { "Create the company and its first administrator." }
            form method="post" action="/setup" class="stack" {
                input type="hidden" name="_csrf" value=(csrf);
                div class="field" { label for="name" { "Company name" } input type="text" id="name" name="name" value=(v("name")) required autofocus; }
                div class="field" { label for="first" { "Your first name" } input type="text" id="first" name="first_name" value=(v("first_name")) required; }
                div class="field" { label for="last" { "Your last name" } input type="text" id="last" name="last_name" value=(v("last_name")) required; }
                div class="field" { label for="email" { "Email" } input type="email" id="email" name="email" value=(v("email")) required autocomplete="username"; }
                div class="field" { label for="password" { "Password" } input type="password" id="password" name="password" required minlength="10" autocomplete="new-password"; div class="hint" { "At least 10 characters." } }
                button type="submit" class="btn primary" { "Create company" }
            }
        },
    )
}

// -- projects

pub fn home(req: &Req<'_>) -> Markup {
    let state = req.engine.state();
    let actor = req.actor();
    let projects = view::projects_for(state, actor);
    let body = html! {
        div class="page-head" {
            div { h1 { "Projects" } div class="sub" { (projects.len()) " project" @if projects.len() != 1 { "s" } } }
        }
        @if projects.is_empty() {
            div class="card" { (empty(Icon::Building, "No projects yet", "Projects you are part of appear here.")) }
        } @else {
            div class="project-grid" {
                @for p in &projects {
                    @let rfis = view::rfis_for(state, actor, p.id);
                    @let open = rfis.iter().filter(|r| r.status == RfiStatus::Open).count();
                    a class="project-card" href=(format!("/projects/{}", p.id)) {
                        @if !p.number.is_empty() { div class="num" { (p.number) } }
                        h3 { (p.name) }
                        div class="meta" {
                            span class=(format!("pill {}", stage_class(p.stage))) { (p.stage.label()) }
                            @if !p.address.city.is_empty() { span { (p.address.city) } }
                        }
                        div class="counts" { span { strong { (open) } "open RFIs" } }
                    }
                }
            }
        }
    };
    layout::app(req, "Projects", Nav::Projects, &[], body)
}

/// The project, if the person may open it.
fn project_for<'a>(req: &'a Req<'_>, id: Id<Project>) -> Option<&'a Project> {
    let p = req.engine.state().projects.get(id)?;
    (p.deleted.is_none() && view::level(req.engine.state(), req.actor(), id, Tool::ProjectHome) >= Level::Read).then_some(p)
}

pub fn not_found(req: &Req<'_>) -> Markup {
    layout::app(
        req,
        "Not found",
        Nav::Projects,
        &[],
        html! { div class="card" { (empty(Icon::Search, "Not found", "That page doesn't exist, or you don't have access to it.")) } },
    )
}

pub fn project_home(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let rfis = view::rfis_for(state, req.actor(), id);
    let today = req.today();
    let open = rfis.iter().filter(|r| r.status == RfiStatus::Open).count();
    let overdue = rfis.iter().filter(|r| r.is_overdue(today)).count();
    let mine = rfis.iter().filter(|r| r.ball_in_court.contains(&req.person)).count();
    let team = state.members.iter().filter(|m| m.project == id && m.removed.is_none()).count();
    let base = format!("/projects/{id}");
    let body = html! {
        div class="page-head" {
            div {
                h1 { (p.name) " " span class=(format!("pill {}", stage_class(p.stage))) { (p.stage.label()) } }
                div class="sub" { @if !p.number.is_empty() { (p.number) " · " } (p.address.one_line()) }
            }
        }
        div class="stat-row" {
            a class="stat" href=(format!("{base}/rfis?status=open")) { div class="label" { "Open RFIs" } div class="value" { (open) } }
            a class=(if overdue > 0 { "stat alert" } else { "stat" }) href=(format!("{base}/rfis?status=open")) { div class="label" { "Overdue RFIs" } div class="value" { (overdue) } }
            a class="stat" href=(format!("{base}/rfis?mine=1")) { div class="label" { "Waiting on me" } div class="value" { (mine) } }
            a class="stat" href=(format!("{base}/team")) { div class="label" { "Team members" } div class="value" { (team) } }
        }
        @if !p.description.is_empty() { div class="card" { h2 { "About" } p class="prose" { (p.description) } } }
        div class="card" {
            div class="card-head" { h2 { "Recent RFIs" } a class="btn small" href=(format!("{base}/rfis")) { "All RFIs" } }
            @if rfis.is_empty() { (empty(Icon::Question, "No RFIs yet", "Questions that need a documented answer are tracked here.")) }
            @else { (rfi_table(req, id, &rfis.iter().rev().take(5).copied().collect::<Vec<_>>())) }
        }
    };
    layout::app(req, &p.name, Nav::Home(id), &[("Projects", Some("/".into())), (&p.name, None)], body)
}

// -- team

fn user_options<'a>(req: &'a Req<'_>, project: Id<Project>, members_only: bool) -> Vec<&'a Person> {
    let state = req.engine.state();
    let mut out: Vec<&Person> = state
        .people
        .iter()
        .filter(|p| p.is_enabled() && p.is_user && (!members_only || perm::membership(state, project, p.id).is_some() || p.company_admin))
        .collect();
    out.sort_by_key(|p| p.name().to_lowercase());
    out
}

pub fn team(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    if view::level(req.engine.state(), req.actor(), id, Tool::ProjectDirectory) < Level::Read {
        return not_found(req);
    }
    let state = req.engine.state();
    let mut members: Vec<&ProjectMember> = state.members.iter().filter(|m| m.project == id && m.removed.is_none()).collect();
    members.sort_by_key(|m| view::person_name(state, m.person).to_lowercase());
    let here = format!("/projects/{id}/team");
    let can_edit = req.allowed("member.add", Some(id));
    let candidates: Vec<&Person> = user_options(req, id, false).into_iter().filter(|p| perm::membership(state, id, p.id).is_none()).collect();
    let templates: Vec<_> = state.templates.iter().filter(|t| t.deleted.is_none()).collect();
    let body = html! {
        div class="page-head" { div { h1 { "Team" } div class="sub" { (members.len()) " people on this project" } } }
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "Name" } th { "Company" } th { "Role" } th { "Permissions" } @if can_edit { th {} } } }
                tbody {
                    @for m in &members {
                        @let person = state.people.get(m.person);
                        tr {
                            td { span class="row-link" { (view::person_name(state, m.person)) } @if let Some(e) = person.and_then(|x| x.email.as_ref()) { div class="muted small" { (e) } } }
                            td { (person.map(|x| view::employer_name(state, x)).unwrap_or_default()) }
                            td { (m.role) }
                            td {
                                @if m.project_admin { span class="tag" { "Project admin" } }
                                @else if let Some(t) = m.template.and_then(|t| state.templates.get(t)) { span class="tag" { (t.name) } }
                            }
                            @if can_edit { td class="right" { (button_form(req, "member.remove", &here, &[("project", id.to_string()), ("person", m.person.to_string())], "Remove", "small danger")) } }
                        }
                    }
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Add a person" }
                @if candidates.is_empty() { p class="muted" { "Everyone in the company directory who can sign in is already on this project." } }
                @else {
                    (command_form(req, "member.add", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        div class="form-grid" {
                            (field("Person", "person", true, false, html! { select id="person" name="person" required { @for c in &candidates { option value=(c.id) { (c.name()) } } } }))
                            (field("Permissions", "template", false, false, html! { select id="template" name="template" { @for t in &templates { option value=(t.id) { (t.name) } } } }))
                            (field("Role", "role", false, false, html! { input type="text" id="role" name="role"; }))
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Add to project" } }
                    }))
                }
            }
        }
    };
    layout::app(req, "Team", Nav::Team(id), &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Team", None)], body)
}

// -- RFIs

fn rfi_table(req: &Req<'_>, project: Id<Project>, rfis: &[&Rfi]) -> Markup {
    let state = req.engine.state();
    let today = req.today();
    html! {
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "#" } th { "Subject" } th { "Status" } th { "Ball in court" } th { "Due" } th { "Manager" } } }
                tbody {
                    @for r in rfis {
                        tr {
                            td class="nowrap" { (r.number) }
                            td { a class="row-link" href=(format!("/projects/{project}/rfis/{}", r.id)) { (r.subject) } }
                            td { (status_pill(r.status)) }
                            td { (view::people_names(state, &r.ball_in_court)) }
                            td class="nowrap" { @if let Some(d) = r.due { span class=(if r.is_overdue(today) { "overdue" } else { "" }) { (d.pretty()) } } }
                            td { @if let Some(m) = r.manager { (view::person_name(state, m)) } }
                        }
                    }
                }
            }
        }
    }
}

/// Filters of the RFI list, from the query string.
#[derive(Default)]
pub struct RfiFilter {
    pub status: Option<RfiStatus>,
    pub q: String,
    pub mine: bool,
}

pub fn rfi_list(req: &Req<'_>, id: Id<Project>, filter: &RfiFilter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    if view::level(req.engine.state(), req.actor(), id, Tool::Rfis) < Level::Read {
        return not_found(req);
    }
    let all = view::rfis_for(req.engine.state(), req.actor(), id);
    let count = |s: RfiStatus| all.iter().filter(|r| r.status == s).count();
    let q = filter.q.to_lowercase();
    let shown: Vec<&Rfi> = all
        .iter()
        .copied()
        .filter(|r| filter.status.is_none_or(|s| r.status == s))
        .filter(|r| !filter.mine || r.ball_in_court.contains(&req.person))
        .filter(|r| q.is_empty() || r.subject.to_lowercase().contains(&q) || r.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/rfis");
    let tab = |label: &str, status: Option<RfiStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "RFIs" } div class="sub" { "Questions that need a documented answer" } }
            div class="actions" { @if req.allowed("rfi.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "New RFI" } } }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Draft", Some(RfiStatus::Draft), count(RfiStatus::Draft)))
            (tab("Open", Some(RfiStatus::Open), count(RfiStatus::Open)))
            (tab("Closed", Some(RfiStatus::Closed), count(RfiStatus::Closed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search subject or number" aria-label="Search RFIs";
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Waiting on me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(Icon::Question, "No RFIs here", if all.is_empty() { "Create the first RFI to get started." } else { "No RFI matches these filters." })) }
        } @else { (rfi_table(req, id, &shown)) }
    };
    layout::app(req, "RFIs", Nav::Rfis(id), &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("RFIs", None)], body)
}

pub fn rfi_detail(req: &Req<'_>, id: Id<Project>, rfi_id: Id<Rfi>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(r) = state.rfis.get(rfi_id).filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r)) else { return not_found(req) };
    let base = format!("/projects/{id}/rfis");
    let here = format!("{base}/{}", r.id);
    let ids = || [("project", id.to_string()), ("id", r.id.to_string())];
    let editable = can_edit(state, req.actor(), r);
    let level = view::level(state, req.actor(), id, Tool::Rfis);
    let manager_like = level == Level::Admin || r.manager == Some(req.person);
    let body = html! {
        div class="page-head" {
            div {
                h1 { (r.label()) ": " (r.subject) " " (status_pill(r.status)) }
                div class="sub" { "Created " (req.when(r.created.at)) @if let Some(b) = r.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if r.status == RfiStatus::Draft && (manager_like || r.created.by == Some(req.person)) && level >= Level::Standard {
                    (button_form(req, "rfi.open", &here, &ids(), "Open RFI", "primary"))
                }
                @if r.status == RfiStatus::Open && manager_like { (button_form(req, "rfi.close", &here, &ids(), "Close RFI", "")) }
                @if r.status == RfiStatus::Closed && manager_like { (button_form(req, "rfi.reopen", &here, &ids(), "Reopen", "")) }
                @if level >= Level::Standard && (level == Level::Admin || (r.status == RfiStatus::Draft && r.created.by == Some(req.person))) {
                    (button_form(req, "rfi.delete", &base, &ids(), "Delete", "danger"))
                }
            }
        }
        div class="detail" {
            div {
                div class="card" { h2 { "Question" } p class="prose" { @if r.question.is_empty() { span class="muted" { "No question text." } } @else { (r.question) } } }
                div class="card" {
                    h2 { "Responses" }
                    @if r.responses.is_empty() { p class="muted" { "No responses yet." } }
                    @for resp in &r.responses {
                        div class=(if resp.official { "response official" } else { "response" }) {
                            div class="meta" {
                                span { strong { @if let Some(b) = resp.by { (view::person_name(state, b)) } @else { "System" } } " · " (req.when(resp.at)) }
                                @if resp.official { span class="pill green" { "Official response" } }
                            }
                            p class="prose" { (resp.body) }
                            @if manager_like && r.status != RfiStatus::Closed {
                                @let f = [("project", id.to_string()), ("id", r.id.to_string()), ("response", resp.id.to_string()), ("official", (!resp.official).to_string())];
                                (button_form(req, "rfi.mark_official", &here, &f, if resp.official { "Remove official mark" } else { "Mark as official" }, "small"))
                            }
                        }
                    }
                    @if r.status == RfiStatus::Open && level >= Level::Standard {
                        (command_form(req, "rfi.respond", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(r.id);
                            (field("Your response", "body", true, true, html! { textarea id="body" name="body" required {} }))
                            div class="form-actions" { button type="submit" class="btn primary" { "Post response" } }
                        }))
                    }
                }
            }
            div class="card" {
                h2 { "Details" }
                dl class="facts" {
                    dt { "Manager" } dd { @if let Some(m) = r.manager { (view::person_name(state, m)) } @else { "—" } }
                    dt { "Assignees" } dd { @if r.assignees.is_empty() { "—" } @else { (view::people_names(state, &r.assignees)) } }
                    dt { "Ball in court" } dd { @if r.ball_in_court.is_empty() { "—" } @else { (view::people_names(state, &r.ball_in_court)) } }
                    dt { "Due" } dd { @if let Some(d) = r.due { span class=(if r.is_overdue(req.today()) { "overdue" } else { "" }) { (d.pretty()) } } @else { "—" } }
                    @if let Some(o) = r.opened { dt { "Opened" } dd { (req.when(o.at)) } }
                    @if let Some(c) = r.closed { dt { "Closed" } dd { (req.when(c.at)) } }
                    dt { "Spec section" } dd { (dash(&r.spec_section)) }
                    dt { "Drawing" } dd { (dash(&r.drawing_number)) }
                    dt { "Location" } dd { (dash(&r.location)) }
                    dt { "Cost impact" } dd { (r.cost_impact.label()) @if let Some(a) = r.cost_amount { " (" (a) ")" } }
                    dt { "Schedule impact" } dd { (r.schedule_impact.label()) @if let Some(d) = r.schedule_days { " (" (d) " days)" } }
                    @if r.private { dt { "Visibility" } dd { "Private" } }
                }
            }
        }
    };
    layout::app(
        req,
        &r.label(),
        Nav::Rfis(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("RFIs", Some(base.clone())), (&r.label(), None)],
        body,
    )
}

fn dash(s: &str) -> &str {
    if s.is_empty() { "—" } else { s }
}

/// The new/edit form. `existing` is `Some` when editing.
pub fn rfi_form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<Rfi>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(rid) => {
            match state.rfis.get(rid).filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r) && can_edit(state, req.actor(), r)) {
                Some(r) => Some(r),
                None => return not_found(req),
            }
        }
        None => {
            if !req.allowed("rfi.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let people = user_options(req, id, true);
    let base = format!("/projects/{id}/rfis");
    let (command, next, back) = match current {
        Some(r) => ("rfi.update", format!("{base}/{}", r.id), format!("{base}/{}/edit", r.id)),
        None => ("rfi.create", format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&Rfi) -> String| current.map(f).unwrap_or_default();
    let manager = current.and_then(|r| r.manager).unwrap_or(req.person);
    let impacts = |name: &str, label: &str, sel: Impact| {
        field(
            label,
            name,
            false,
            false,
            html! { select id=(name) name=(name) { @for i in [Impact::Tbd, Impact::No, Impact::Yes] { option value=(i.key()) selected[i == sel] { (i.label()) } } } },
        )
    };
    let title = if current.is_some() { "Edit RFI" } else { "New RFI" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(r) = current { input type="hidden" name="id" value=(r.id); }
                div class="form-grid" {
                    (field("Subject", "subject", true, true, html! { input type="text" id="subject" name="subject" required maxlength="200" value=(text(|r| r.subject.clone())); }))
                    (field("Question", "question", false, true, html! { textarea id="question" name="question" { (text(|r| r.question.clone())) } }))
                    (field("Manager", "manager", false, false, html! { select id="manager" name="manager" { @for x in &people { option value=(x.id) selected[x.id == manager] { (x.name()) } } } }))
                    (field("Due date", "due", false, false, html! { input type="date" id="due" name="due" value=(current.and_then(|r| r.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    div class="field full" {
                        div class="label" { "Assignees" }
                        input type="hidden" name="assignees[]" value="";
                        details class="multi" {
                            summary { "Choose people who should answer" }
                            div class="options" { @for x in &people {
                                label { input type="checkbox" name="assignees[]" value=(x.id) checked[current.is_some_and(|r| r.assignees.contains(&x.id))]; (x.name()) }
                            } }
                        }
                    }
                    (field("Spec section", "spec_section", false, false, html! { input type="text" id="spec_section" name="spec_section" value=(text(|r| r.spec_section.clone())); }))
                    (field("Drawing number", "drawing_number", false, false, html! { input type="text" id="drawing_number" name="drawing_number" value=(text(|r| r.drawing_number.clone())); }))
                    (field("Location", "location", false, true, html! { input type="text" id="location" name="location" value=(text(|r| r.location.clone())); }))
                    (impacts("cost_impact", "Cost impact", current.map(|r| r.cost_impact).unwrap_or_default()))
                    (impacts("schedule_impact", "Schedule impact", current.map(|r| r.schedule_impact).unwrap_or_default()))
                    div class="field full" {
                        input type="hidden" name="private" value="false";
                        label class="check" { input type="checkbox" name="private" value="true" checked[current.is_some_and(|r| r.private)]; "Private (only people on this RFI can see it)" }
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(current.map(|r| format!("{base}/{}", r.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    @if current.is_none() {
                        button type="submit" name="open" value="true" class="btn" { "Save and open" }
                        button type="submit" class="btn primary" { "Save as draft" }
                    } @else {
                        button type="submit" class="btn primary" { "Save changes" }
                    }
                }
            }))
        }
    };
    layout::app(
        req,
        title,
        Nav::Rfis(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("RFIs", Some(base.clone())), (title, None)],
        body,
    )
}
