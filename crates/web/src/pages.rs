//! The pages: sign-in, setup, projects, project home, team and RFIs.

use buildkiosk_core::Id;
use buildkiosk_engine::model::directory::{Person, VendorKind};
use buildkiosk_engine::model::perm::ProjectMember;
use buildkiosk_engine::model::project::{Project, ProjectStage};
use buildkiosk_engine::model::rfi::{Rfi, RfiStatus};
use buildkiosk_engine::model::{Choice, Impact, Level, Tool, perm};
use buildkiosk_engine::{JournalEntry, cmd::rfi::can_edit, find_command, view};
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
        @if projects.len() > 1 {
            div class="card" {
                h2 { "Portfolio" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Project" } th { "Stage" } th class="num" { "Needs action" } th class="num" { "Past due" } th class="num" { "Revised budget" } th class="num" { "Projected over / under" } } }
                    tbody { @for r in buildkiosk_engine::report::portfolio(state, actor, req.today()) { tr {
                        td { a href=(format!("/projects/{}", r.project)) { (r.name) } @if !r.number.is_empty() { " " span class="muted small" { (r.number) } } }
                        td { (r.stage) }
                        td class="num" { (r.open) }
                        td class="num" { @if r.overdue > 0 { span class="overdue" { (r.overdue) } } @else { (r.overdue) } }
                        td class="num" { @if let Some(x) = r.revised { (x) } @else { span class="muted" { "—" } } }
                        td class="num" { @if let Some(x) = r.variance { span class=(if x.is_negative() { "overdue" } else { "" }) { (x) } } @else { span class="muted" { "—" } } }
                    } } }
                } }
            }
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
pub(crate) fn project_for<'a>(req: &'a Req<'_>, id: Id<Project>) -> Option<&'a Project> {
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
    let waiting = buildkiosk_engine::report::waiting_on(state, req.actor(), id, req.person);
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
        @if !waiting.is_empty() {
            div class="card" {
                h2 { "Waiting on me" }
                table class="data" {
                    thead { tr { th { "Item" } th { "What" } th { "Due" } } }
                    tbody { @for t in waiting.iter().take(10) { tr {
                        td class="nowrap" { a href=(t.link) { (t.label) } }
                        td { (t.title) }
                        td class="nowrap" { @if let Some(d) = t.due { @if d < today { span class="overdue" { (d.to_string()) } } @else { (d.to_string()) } } }
                    } } }
                }
                @if waiting.len() > 10 { p class="muted small" { (waiting.len() - 10) " more. " a href=(format!("{base}/reports")) { "See the project report" } } }
            }
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

pub(crate) fn user_options<'a>(req: &'a Req<'_>, project: Id<Project>, members_only: bool) -> Vec<&'a Person> {
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
                            @if can_edit {
                                td class="right" {
                                    details class="disclosure" {
                                        summary { "Edit" }
                                        (command_form(req, "member.update", &here, &here, html! {
                                            input type="hidden" name="project" value=(id);
                                            input type="hidden" name="person" value=(m.person);
                                            div class="stack" {
                                                (field("Permissions", &format!("t{}", m.id), false, false, html! { select id=(format!("t{}", m.id)) name="template" { @for t in &templates { option value=(t.id) selected[m.template == Some(t.id)] { (t.name) } } } }))
                                                (field("Role", &format!("r{}", m.id), false, false, html! { input type="text" id=(format!("r{}", m.id)) name="role" value=(m.role); }))
                                                input type="hidden" name="project_admin" value="false";
                                                label class="check" { input type="checkbox" name="project_admin" value="true" checked[m.project_admin]; "Project admin" }
                                                button type="submit" class="btn small primary" { "Save" }
                                            }
                                        }))
                                    }
                                    (button_form(req, "member.remove", &here, &[("project", id.to_string()), ("person", m.person.to_string())], "Remove", "small danger"))
                                }
                            }
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
            div class="actions" {
                a class="btn" href=(format!("{base}.csv")) { (Icon::Download.svg()) "Export CSV" }
                @if req.allowed("rfi.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "New RFI" } } }
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

pub fn rfi_detail(req: &Req<'_>, id: Id<Project>, rfi_id: Id<Rfi>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(r) = state.rfis.get(rfi_id).filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r)) else { return not_found(req) };
    let base = format!("/projects/{id}/rfis");
    let here = format!("{base}/{}", r.id);
    let ids = || [("project", id.to_string()), ("id", r.id.to_string())];
    let editable = can_edit(state, req.actor(), r);
    let level = view::level(state, req.actor(), id, Tool::Rfis);
    let manager_like = level == Level::Admin || r.manager == Some(req.person);
    let can_attach = r.status != RfiStatus::Closed && (editable || (r.status == RfiStatus::Open && r.assignees.contains(&req.person)));
    let body = html! {
        div class="page-head" {
            div {
                h1 { (r.label()) ": " (r.subject) " " (status_pill(r.status)) }
                div class="sub" { "Created " (req.when(r.created.at)) @if let Some(b) = r.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
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
                            @for a in &resp.attachments {
                                div class="row" { a href=(format!("{here}/files/{}", a.sha256)) { (a.name) } span class="muted small" { (human_size(a.size)) } }
                            }
                            @if manager_like && r.status != RfiStatus::Closed {
                                @let f = [("project", id.to_string()), ("id", r.id.to_string()), ("response", resp.id.to_string()), ("official", (!resp.official).to_string())];
                                (button_form(req, "rfi.mark_official", &here, &f, if resp.official { "Remove official mark" } else { "Mark as official" }, "small"))
                            }
                        }
                    }
                    @if r.status == RfiStatus::Open && level >= Level::Standard {
                        form method="post" action=(format!("{here}/respond")) enctype="multipart/form-data" class="stack" {
                            input type="hidden" name="_csrf" value=(req.csrf);
                            (field("Your response", "body", true, true, html! { textarea id="body" name="body" required {} }))
                            (field("Files (optional)", "file", false, false, html! { input type="file" id="file" name="file" multiple; }))
                            div class="form-actions" { button type="submit" class="btn primary" { "Post response" } }
                        }
                    }
                }
            }
            div class="stack" {
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
                    @if !r.distribution.is_empty() { dt { "Distribution" } dd { (view::people_names(state, &r.distribution)) } }
                }
            }
            div class="card" {
                h2 { "Files" }
                @if r.attachments.is_empty() { p class="muted" { "No files attached." } }
                @for a in &r.attachments {
                    div class="row" {
                        a href=(format!("{here}/files/{}", a.sha256)) { (a.name) }
                        span class="muted small" { (human_size(a.size)) }
                        @if can_attach {
                            (button_form(req, "rfi.detach", &here, &[("project", id.to_string()), ("id", r.id.to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger"))
                        }
                    }
                }
                @if can_attach {
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
                            div { (find_command(&e.cmd).map(|c| c.label).unwrap_or(e.cmd.as_str())) " by " @if let Some(a) = e.actor { (view::person_name(state, a)) } @else { "the system" } }
                            div class="when" { (req.when(e.at)) }
                        }
                    }
                }
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

pub(crate) fn human_size(n: u64) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{} KB", n / 1024),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

pub(crate) fn dash(s: &str) -> &str {
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
                    div class="field full" {
                        div class="label" { "Distribution list" }
                        input type="hidden" name="distribution[]" value="";
                        details class="multi" {
                            summary { "Choose people to keep informed" }
                            div class="options" { @for x in &people {
                                label { input type="checkbox" name="distribution[]" value=(x.id) checked[current.is_some_and(|r| r.distribution.contains(&x.id))]; (x.name()) }
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

/// The RFIs the person may see, as CSV (spreadsheet formulas in text are defused).
pub fn rfi_csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let cell = layout::csv_cell;
    let mut out = String::from(
        "Number,Subject,Status,Manager,Assignees,Ball in court,Due,Created,Opened,Closed,Spec section,Drawing,Location,Cost impact,Schedule impact\r\n",
    );
    for r in view::rfis_for(state, req.actor(), id) {
        let day = |t: buildkiosk_core::Timestamp| t.date_at(state.utc_offset()).to_string();
        let row = [
            r.number.to_string(),
            r.subject.clone(),
            r.status.label().to_string(),
            r.manager.map(|m| view::person_name(state, m)).unwrap_or_default(),
            view::people_names(state, &r.assignees),
            view::people_names(state, &r.ball_in_court),
            r.due.map(|d| d.to_string()).unwrap_or_default(),
            day(r.created.at),
            r.opened.map(|s| day(s.at)).unwrap_or_default(),
            r.closed.map(|s| day(s.at)).unwrap_or_default(),
            r.spec_section.clone(),
            r.drawing_number.clone(),
            r.location.clone(),
            r.cost_impact.label().to_string(),
            r.schedule_impact.label().to_string(),
        ];
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

// -- company directory

/// People, companies and permission templates of the whole company.
pub fn directory(req: &Req<'_>) -> Markup {
    if !req.allowed("person.list", None) {
        return not_found(req);
    }
    let state = req.engine.state();
    let is_admin = state.people.get(req.person).is_some_and(|p| p.company_admin);
    let can_add = req.allowed("person.create", None);
    let mut people: Vec<&Person> = state.people.iter().filter(|p| p.deleted.is_none()).collect();
    people.sort_by_key(|p| p.name().to_lowercase());
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let templates: Vec<_> = state.templates.iter().filter(|t| t.deleted.is_none()).collect();
    let here = "/directory";
    let body = html! {
        div class="page-head" { div { h1 { "Directory" } div class="sub" { "People and companies you work with" } } }
        div class="card-head" { h2 { "People" } }
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "Name" } th { "Company" } th { "Title" } th { "Access" } @if is_admin { th { "Password" } } } }
                tbody {
                    @for p in &people {
                        tr {
                            td { a class="row-link" href=(format!("/directory/people/{}", p.id)) { (p.name()) } @if p.is_enabled() {} @else { " " span class="tag" { "Inactive" } } @if let Some(e) = &p.email { div class="muted small" { (e) } } }
                            td { (view::employer_name(state, p)) }
                            td { (p.title) }
                            td {
                                @if p.company_admin { span class="tag" { "Company admin" } }
                                @else if p.is_user { span class="tag" { "Can sign in" } }
                                @else { span class="muted" { "Contact only" } }
                            }
                            @if is_admin {
                                td {
                                    @if p.is_user && p.is_enabled() {
                                        details class="disclosure" {
                                            summary { "Set password" }
                                            form method="post" action=(format!("/people/{}/password", p.id)) class="stack" {
                                                input type="hidden" name="_csrf" value=(req.csrf);
                                                input type="password" name="password" minlength="10" required placeholder="New password" aria-label=(format!("New password for {}", p.name())) autocomplete="new-password";
                                                button type="submit" class="btn small" { "Save password" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        @if can_add {
            div class="card" {
                h2 { "Add a person" }
                (command_form(req, "person.create", here, here, html! {
                    div class="form-grid" {
                        (field("First name", "first_name", true, false, html! { input type="text" id="first_name" name="first_name" required; }))
                        (field("Last name", "last_name", true, false, html! { input type="text" id="last_name" name="last_name" required; }))
                        (field("Email", "email", false, false, html! { input type="email" id="email" name="email"; }))
                        (field("Title", "title", false, false, html! { input type="text" id="title" name="title"; }))
                        (field("Company", "vendor", false, false, html! { select id="vendor" name="vendor" { option value="" { "Our company" } @for v in &vendors { option value=(v.id) { (v.name) } } } }))
                        div class="field" {
                            input type="hidden" name="is_user" value="false";
                            label class="check" { input type="checkbox" name="is_user" value="true"; "Can sign in (needs an email)" }
                        }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add person" } }
                }))
            }
        }
        div class="card-head" { h2 { "Companies" } }
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "Name" } th { "Kind" } th { "Trade" } th { "Phone" } @if req.allowed("vendor.update", None) { th {} } } }
                tbody { @for v in &vendors { tr { td { (v.name) } td { (v.kind.label()) } td { (v.trade) } td { (v.phone) } @if req.allowed("vendor.update", None) { td class="right" { a class="btn small" href=(format!("/directory/companies/{}", v.id)) { "Edit" } } } } } }
            }
        }
        @if req.allowed("vendor.create", None) {
            div class="card" {
                h2 { "Add a company" }
                (command_form(req, "vendor.create", here, here, html! {
                    div class="form-grid" {
                        (field("Name", "vname", true, false, html! { input type="text" id="vname" name="name" required; }))
                        (field("Kind", "vkind", false, false, html! { select id="vkind" name="kind" { @for k in [VendorKind::Subcontractor, VendorKind::Supplier, VendorKind::Owner, VendorKind::Design, VendorKind::Consultant, VendorKind::Inspector, VendorKind::Other] { option value=(k.key()) { (k.label()) } } } }))
                        (field("Trade", "trade", false, false, html! { input type="text" id="trade" name="trade"; }))
                        (field("Phone", "vphone", false, false, html! { input type="text" id="vphone" name="phone"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add company" } }
                }))
            }
        }
        div class="card-head" { h2 { "Permission templates" } }
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "Template" } th { "What it is for" } @if is_admin { th {} } } }
                tbody { @for t in &templates { tr { td { strong { (t.name) } } td { (t.description) } @if is_admin { td class="right" { a class="btn small" href=(format!("/directory/templates/{}", t.id)) { "Edit levels" } } } } } }
            }
        }
        @if is_admin {
            div class="card" {
                h2 { "Add a permission template" }
                (command_form(req, "template.create", here, here, html! {
                    div class="form-grid" {
                        (field("Name", "tname", true, false, html! { input type="text" id="tname" name="name" required; }))
                        (field("What it is for", "tdesc", false, false, html! { input type="text" id="tdesc" name="description"; }))
                    }
                    p class="muted small" { "Set what the template allows on each tool after adding it." }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add template" } }
                }))
            }
        }
    };
    layout::app(req, "Directory", Nav::Directory, &[("Directory", None)], body)
}

/// Edit one person of the directory.
pub fn person_edit(req: &Req<'_>, id: Id<Person>) -> Markup {
    let state = req.engine.state();
    let Some(p) = state.people.get(id).filter(|p| p.deleted.is_none()) else { return not_found(req) };
    if !req.allowed("person.update", None) {
        return not_found(req);
    }
    let is_admin = state.people.get(req.person).is_some_and(|x| x.company_admin);
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let here = format!("/directory/people/{id}");
    let flag = |name: &str, label: &str, on: bool| {
        html! { div class="field" { input type="hidden" name=(name) value="false"; label class="check" { input type="checkbox" name=(name) value="true" checked[on]; (label) } } }
    };
    let body = html! {
        div class="page-head" { div { h1 { (p.name()) } div class="sub" { (view::employer_name(state, p)) } } }
        div class="card" {
            (command_form(req, "person.update", &here, &here, html! {
                input type="hidden" name="id" value=(id);
                div class="form-grid" {
                    (field("First name", "first_name", true, false, html! { input type="text" id="first_name" name="first_name" required value=(p.first_name); }))
                    (field("Last name", "last_name", true, false, html! { input type="text" id="last_name" name="last_name" required value=(p.last_name); }))
                    (field("Email", "email", false, false, html! { input type="email" id="email" name="email" value=(p.email.clone().unwrap_or_default()); }))
                    (field("Phone", "phone", false, false, html! { input type="text" id="phone" name="phone" value=(p.phone); }))
                    (field("Title", "title", false, false, html! { input type="text" id="title" name="title" value=(p.title); }))
                    (field("Company", "vendor", false, false, html! { select id="vendor" name="vendor" { option value="" { "Our company" } @for v in &vendors { option value=(v.id) selected[p.vendor == Some(v.id)] { (v.name) } } } }))
                    (flag("is_user", "Can sign in (needs an email)", p.is_user))
                    (flag("active", "Active", p.active))
                    @if is_admin { (flag("company_admin", "Company admin", p.company_admin)) }
                }
                div class="form-actions" {
                    a class="btn" href="/directory" { "Back" }
                    button type="submit" class="btn primary" { "Save changes" }
                }
            }))
        }
        @if req.allowed("person.delete", None) && id != req.person {
            div class="card" {
                h2 { "Remove from directory" }
                p class="muted" { "Removing a person keeps what they did in the history; they can no longer sign in." }
                (button_form(req, "person.delete", "/directory", &[("id", id.to_string())], "Remove person", "danger"))
            }
        }
    };
    layout::app(req, &p.name(), Nav::Directory, &[("Directory", Some("/directory".into())), (&p.name(), None)], body)
}

/// Edit a permission template: its name and the level it gives on each tool.
pub fn template_edit(req: &Req<'_>, id: buildkiosk_core::Id<perm::PermissionTemplate>) -> Markup {
    use buildkiosk_engine::model::perm::PermissionTemplate;
    use buildkiosk_engine::model::tool::ToolGroup;
    let state = req.engine.state();
    let Some(t) = state.templates.get(id).filter(|t| t.deleted.is_none()) else { return not_found(req) };
    if !state.people.get(req.person).is_some_and(|x| x.company_admin) {
        return not_found(req);
    }
    let _: Option<&PermissionTemplate> = Some(t);
    let here = format!("/directory/templates/{id}");
    let body = html! {
        div class="page-head" { div { h1 { "Permission template: " (t.name) } div class="sub" { "The level this template gives on each tool. People with it get these levels on the projects they join, unless a person has an override." } } }
        div class="card" {
            (command_form(req, "template.update", &here, &here, html! {
                input type="hidden" name="id" value=(id);
                div class="form-grid" {
                    (field("Name", "tname", true, false, html! { input type="text" id="tname" name="name" value=(t.name) required; }))
                    (field("What it is for", "tdesc", false, false, html! { input type="text" id="tdesc" name="description" value=(t.description); }))
                }
                @for group in ToolGroup::ALL {
                    @let tools: Vec<Tool> = Tool::ALL.iter().copied().filter(|x| x.group() == *group).collect();
                    @if !tools.is_empty() {
                        h2 { (group.label()) }
                        table class="data matrix" {
                            thead { tr { th { "Tool" } th { "Level" } } }
                            tbody { @for tool in tools { tr {
                                td { (tool.label()) }
                                td { select name=(format!("levels.{}", tool.key())) aria-label=(format!("Level on {}", tool.label())) {
                                    @for l in Level::ALL { option value=(l.key()) selected[t.level(tool) == *l] { (l.label()) } }
                                } }
                            } } }
                        }
                    }
                }
                div class="form-actions" { a class="btn" href="/directory" { "Back" } button type="submit" class="btn primary" { "Save levels" } }
            }))
            (button_form(req, "template.delete", "/directory", &[("id", id.to_string())], "Delete this template", "small danger"))
        }
    };
    layout::app(req, "Permission template", Nav::Directory, &[("Directory", Some("/directory".into())), (&t.name, None)], body)
}

/// Edit one company of the directory.
pub fn vendor_edit(req: &Req<'_>, id: buildkiosk_core::Id<buildkiosk_engine::model::directory::Vendor>) -> Markup {
    let state = req.engine.state();
    let Some(v) = state.vendors.get(id).filter(|v| v.deleted.is_none()) else { return not_found(req) };
    if !req.allowed("vendor.update", None) {
        return not_found(req);
    }
    let here = format!("/directory/companies/{id}");
    let body = html! {
        div class="page-head" { div { h1 { (v.name) } } }
        div class="card" {
            (command_form(req, "vendor.update", "/directory", &here, html! {
                input type="hidden" name="id" value=(id);
                div class="form-grid" {
                    (field("Name", "vname", true, false, html! { input type="text" id="vname" name="name" value=(v.name) required; }))
                    (field("Kind", "vkind", false, false, html! { select id="vkind" name="kind" { @for k in [VendorKind::Subcontractor, VendorKind::Supplier, VendorKind::Owner, VendorKind::Design, VendorKind::Consultant, VendorKind::Inspector, VendorKind::Other] { option value=(k.key()) selected[v.kind == k] { (k.label()) } } } }))
                    (field("Trade", "trade", false, false, html! { input type="text" id="trade" name="trade" value=(v.trade); }))
                    (field("Licence number", "license_number", false, false, html! { input type="text" id="license_number" name="license_number" value=(v.license_number); }))
                    (field("Email", "vemail", false, false, html! { input type="email" id="vemail" name="email" value=(v.email); }))
                    (field("Phone", "vphone", false, false, html! { input type="text" id="vphone" name="phone" value=(v.phone); }))
                    (field("Website", "website", false, false, html! { input type="text" id="website" name="website" value=(v.website); }))
                    (field("Street", "street", false, false, html! { input type="text" id="street" name="street" value=(v.address.street); }))
                    (field("City", "city", false, false, html! { input type="text" id="city" name="city" value=(v.address.city); }))
                    (field("State or region", "region", false, false, html! { input type="text" id="region" name="region" value=(v.address.region); }))
                    (field("Postal code", "postal_code", false, false, html! { input type="text" id="postal_code" name="postal_code" value=(v.address.postal_code); }))
                    (field("Country", "country", false, false, html! { input type="text" id="country" name="country" value=(v.address.country); }))
                    (field("Notes", "vnotes", false, true, html! { textarea id="vnotes" name="notes" class="short" { (v.notes) } }))
                }
                div class="form-actions" { a class="btn" href="/directory" { "Cancel" } button type="submit" class="btn primary" { "Save" } }
            }))
        }
    };
    layout::app(req, &v.name, Nav::Directory, &[("Directory", Some("/directory".into())), (&v.name, None)], body)
}
