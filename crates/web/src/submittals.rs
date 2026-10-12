//! Submittal pages: list, detail and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::submittal::{Decision, Submittal, SubmittalStatus};
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::{JournalEntry, cmd::submittal::can_edit, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

fn status_pill(s: SubmittalStatus) -> Markup {
    let class = match s {
        SubmittalStatus::Draft => "violet",
        SubmittalStatus::InReview => "blue",
        SubmittalStatus::Closed => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn decision_pill(d: Decision) -> Markup {
    let class = match d {
        Decision::Approved => "green",
        Decision::ApprovedAsNoted => "blue",
        Decision::ReviseResubmit => "amber",
        Decision::Rejected => "red",
    };
    html! { span class=(format!("pill {class}")) { (d.label()) } }
}

fn decision_select(name: &str) -> Markup {
    html! {
        select id=(name) name="decision" required aria-label="Decision" {
            @for d in [Decision::Approved, Decision::ApprovedAsNoted, Decision::ReviseResubmit, Decision::Rejected] { option value=(d.key()) { (d.label()) } }
        }
    }
}

fn table(req: &Req<'_>, project: Id<Project>, list: &[&Submittal]) -> Markup {
    let state = req.engine.state();
    let today = req.today();
    html! {
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "#" } th { "Title" } th { "Spec section" } th { "Status" } th { "Ball in court" } th { "Due" } } }
                tbody {
                    @for s in list {
                        tr {
                            td class="nowrap" { (s.number) @if s.revision > 0 { "." (s.revision) } }
                            td { a class="row-link" href=(format!("/projects/{project}/submittals/{}", s.id)) { (s.title) } }
                            td { (s.spec_section) }
                            td { (status_pill(s.status)) @if let Some(d) = s.final_decision { " " (decision_pill(d)) } }
                            td { (view::people_names(state, &s.ball_in_court)) }
                            td class="nowrap" { @if let Some(d) = s.due { span class=(if s.is_overdue(today) { "overdue" } else { "" }) { (d.pretty()) } } }
                        }
                    }
                }
            }
        }
    }
}

/// Filters of the submittal list, from the query string.
#[derive(Default)]
pub struct Filter {
    pub status: Option<SubmittalStatus>,
    pub q: String,
    pub mine: bool,
}

fn crumbs<'a>(p: &'a Project, last: &'a str, list_link: Option<String>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    if list_link.is_some() {
        c.push(("Submittals", list_link));
    }
    c.push((last, None));
    c
}

pub fn list(req: &Req<'_>, id: Id<Project>, filter: &Filter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    if view::level(req.engine.state(), req.actor(), id, Tool::Submittals) < Level::Read {
        return not_found(req);
    }
    let all = view::submittals_for(req.engine.state(), req.actor(), id);
    let count = |s: SubmittalStatus| all.iter().filter(|x| x.status == s).count();
    let q = filter.q.to_lowercase();
    let shown: Vec<&Submittal> = all
        .iter()
        .copied()
        .filter(|s| filter.status.is_none_or(|x| s.status == x))
        .filter(|s| !filter.mine || s.ball_in_court.contains(&req.person))
        .filter(|s| q.is_empty() || s.title.to_lowercase().contains(&q) || s.spec_section.to_lowercase().contains(&q) || s.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/submittals");
    let tab = |label: &str, status: Option<SubmittalStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "Submittals" } div class="sub" { "Documents sent for review before the work goes ahead" } }
            div class="actions" { @if req.allowed("submittal.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "New submittal" } } }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Draft", Some(SubmittalStatus::Draft), count(SubmittalStatus::Draft)))
            (tab("In review", Some(SubmittalStatus::InReview), count(SubmittalStatus::InReview)))
            (tab("Closed", Some(SubmittalStatus::Closed), count(SubmittalStatus::Closed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title, spec section or number" aria-label="Search submittals";
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Waiting on me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Submittals), "No submittals here", if all.is_empty() { "Create the first submittal to get started." } else { "No submittal matches these filters." })) }
        } @else { (table(req, id, &shown)) }
    };
    layout::app(
        req,
        "Submittals",
        Nav::Submittals(id),
        &crumbs(p, "", None).into_iter().take(2).chain([("Submittals", None)]).collect::<Vec<_>>(),
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, sid: Id<Submittal>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(s) = state.submittals.get(sid).filter(|s| s.project == id && view::can_see_submittal(state, req.actor(), s)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/submittals");
    let here = format!("{base}/{}", s.id);
    let ids = [("project", id.to_string()), ("id", s.id.to_string())];
    let level = view::level(state, req.actor(), id, Tool::Submittals);
    let editable = can_edit(state, req.actor(), s);
    let manager_like = level == Level::Admin || s.manager == Some(req.person);
    let in_review = s.status == SubmittalStatus::InReview;
    let can_review =
        in_review && level >= Level::Standard && (level == Level::Admin || s.reviewers.contains(&req.person)) && !s.has_reviewed(req.person);
    let can_attach = s.status != SubmittalStatus::Closed && (editable || (in_review && s.reviewers.contains(&req.person)));
    let body = html! {
        div class="page-head" {
            div {
                h1 { (s.label()) ": " (s.title) " " (status_pill(s.status)) @if let Some(d) = s.final_decision { " " (decision_pill(d)) } }
                div class="sub" { "Created " (req.when(s.created.at)) @if let Some(b) = s.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if s.status == SubmittalStatus::Draft && level >= Level::Standard && (manager_like || s.created.by == Some(req.person)) {
                    (button_form(req, "submittal.open", &here, &ids, "Send for review", "primary"))
                }
                @if s.status != SubmittalStatus::Draft && manager_like { (button_form(req, "submittal.resubmit", &here, &ids, "Resubmit (new revision)", "")) }
                @if level >= Level::Standard && (level == Level::Admin || (s.status == SubmittalStatus::Draft && s.created.by == Some(req.person))) {
                    (button_form(req, "submittal.delete", &base, &ids, "Delete", "danger"))
                }
            }
        }
        div class="detail" {
            div {
                @if !s.description.is_empty() { div class="card" { h2 { "Description" } p class="prose" { (s.description) } } }
                div class="card" {
                    h2 { "Reviews" }
                    @if s.reviews.is_empty() { p class="muted" { "No reviews yet." } }
                    @for r in s.reviews.iter().rev() {
                        div class="response" {
                            div class="meta" {
                                span { strong { @if let Some(b) = r.by { (view::person_name(state, b)) } @else { "System" } } " · " (req.when(r.at)) " · revision " (r.revision) }
                                (decision_pill(r.decision))
                            }
                            @if !r.comment.is_empty() { p class="prose" { (r.comment) } }
                        }
                    }
                    @if can_review {
                        (command_form(req, "submittal.review", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(s.id);
                            div class="form-grid" {
                                (field("Decision", "decision", true, false, decision_select("decision")))
                                (field("Comment", "comment", false, true, html! { textarea id="comment" name="comment" class="short" {} }))
                            }
                            div class="form-actions" { button type="submit" class="btn primary" { "Submit review" } }
                        }))
                    }
                }
                @if in_review && manager_like {
                    div class="card" {
                        h2 { "Close with a final decision" }
                        (command_form(req, "submittal.close", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(s.id);
                            div class="row" { (decision_select("final")) button type="submit" class="btn primary" { "Close submittal" } }
                        }))
                    }
                }
            }
            div class="stack" {
                div class="card" {
                    h2 { "Details" }
                    dl class="facts" {
                        dt { "Manager" } dd { @if let Some(m) = s.manager { (view::person_name(state, m)) } @else { "—" } }
                        dt { "Submitter" } dd { @if let Some(m) = s.submitter { (view::person_name(state, m)) } @else { "—" } }
                        dt { "Reviewers" } dd { @if s.reviewers.is_empty() { "—" } @else { (view::people_names(state, &s.reviewers)) } }
                        dt { "Ball in court" } dd { @if s.ball_in_court.is_empty() { "—" } @else { (view::people_names(state, &s.ball_in_court)) } }
                        dt { "Due" } dd { @if let Some(d) = s.due { span class=(if s.is_overdue(req.today()) { "overdue" } else { "" }) { (d.pretty()) } } @else { "—" } }
                        dt { "Spec section" } dd { (dash(&s.spec_section)) }
                        dt { "Type" } dd { (dash(&s.kind)) }
                        @if !s.distribution.is_empty() { dt { "Distribution" } dd { (view::people_names(state, &s.distribution)) } }
                        @if s.private { dt { "Visibility" } dd { "Private" } }
                    }
                }
                div class="card" {
                    h2 { "Files" }
                    @if s.attachments.is_empty() { p class="muted" { "No files attached." } }
                    @for a in &s.attachments {
                        div class="row" {
                            a href=(format!("{here}/files/{}", a.sha256)) { (a.name) }
                            span class="muted small" { (human_size(a.size)) }
                            @if can_attach { (button_form(req, "submittal.detach", &here, &[("project", id.to_string()), ("id", s.id.to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
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
    layout::app(req, &s.label(), Nav::Submittals(id), &crumbs(p, &s.label(), Some(base.clone())), body)
}

fn people_picker(
    name: &str,
    summary: &str,
    people: &[&buildkiosk_engine::model::directory::Person],
    chosen: &[Id<buildkiosk_engine::model::directory::Person>],
) -> Markup {
    html! {
        div class="field full" {
            div class="label" { (summary) }
            input type="hidden" name=(format!("{name}[]")) value="";
            details class="multi" {
                summary { "Choose people" }
                div class="options" { @for x in people {
                    label { input type="checkbox" name=(format!("{name}[]")) value=(x.id) checked[chosen.contains(&x.id)]; (x.name()) }
                } }
            }
        }
    }
}

/// The new/edit form. `existing` is `Some` when editing.
pub fn form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<Submittal>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(sid) => match state
            .submittals
            .get(sid)
            .filter(|s| s.project == id && view::can_see_submittal(state, req.actor(), s) && can_edit(state, req.actor(), s))
        {
            Some(s) => Some(s),
            None => return not_found(req),
        },
        None => {
            if !req.allowed("submittal.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let people = user_options(req, id, true);
    let base = format!("/projects/{id}/submittals");
    let (command, next, back) = match current {
        Some(s) => ("submittal.update", format!("{base}/{}", s.id), format!("{base}/{}/edit", s.id)),
        None => ("submittal.create", format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&Submittal) -> String| current.map(f).unwrap_or_default();
    let manager = current.and_then(|s| s.manager).unwrap_or(req.person);
    let person_select = |name: &str, label: &str, sel: Option<Id<buildkiosk_engine::model::directory::Person>>, blank: bool| {
        field(
            label,
            name,
            false,
            false,
            html! { select id=(name) name=(name) { @if blank { option value="" { "—" } } @for x in &people { option value=(x.id) selected[Some(x.id) == sel] { (x.name()) } } } },
        )
    };
    let title = if current.is_some() { "Edit submittal" } else { "New submittal" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(s) = current { input type="hidden" name="id" value=(s.id); }
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(text(|s| s.title.clone())); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" { (text(|s| s.description.clone())) } }))
                    (field("Spec section", "spec_section", false, false, html! { input type="text" id="spec_section" name="spec_section" value=(text(|s| s.spec_section.clone())); }))
                    (field("Type", "kind", false, false, html! { input type="text" id="kind" name="kind" placeholder="Shop drawing, product data, sample…" value=(text(|s| s.kind.clone())); }))
                    (person_select("manager", "Manager", Some(manager), false))
                    (person_select("submitter", "Submitter", current.and_then(|s| s.submitter), true))
                    (field("Due date", "due", false, false, html! { input type="date" id="due" name="due" value=(current.and_then(|s| s.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    (people_picker("reviewers", "Reviewers", &people, current.map(|s| s.reviewers.as_slice()).unwrap_or(&[])))
                    (people_picker("distribution", "Distribution list", &people, current.map(|s| s.distribution.as_slice()).unwrap_or(&[])))
                    div class="field full" {
                        input type="hidden" name="private" value="false";
                        label class="check" { input type="checkbox" name="private" value="true" checked[current.is_some_and(|s| s.private)]; "Private (only people on this submittal can see it)" }
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(current.map(|s| format!("{base}/{}", s.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    @if current.is_none() {
                        button type="submit" name="open" value="true" class="btn" { "Save and send for review" }
                        button type="submit" class="btn primary" { "Save as draft" }
                    } @else {
                        button type="submit" class="btn primary" { "Save changes" }
                    }
                }
            }))
        }
    };
    layout::app(req, title, Nav::Submittals(id), &crumbs(p, title, Some(base.clone())), body)
}
