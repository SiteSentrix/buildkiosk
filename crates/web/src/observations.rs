//! Observations pages: list, detail and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::observation::can_edit;
use buildkiosk_engine::model::observation::{Observation, ObservationStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Priority, Tool};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

fn status_pill(s: ObservationStatus) -> Markup {
    let class = match s {
        ObservationStatus::Open => "blue",
        ObservationStatus::ReadyForReview => "amber",
        ObservationStatus::Closed => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn priority_tag(p: Priority) -> Markup {
    let class = match p {
        Priority::Urgent => "pill red",
        Priority::High => "pill amber",
        _ => "tag",
    };
    html! { span class=(class) { (p.label()) } }
}

/// Filters of the list, from the query string.
#[derive(Default)]
pub struct Filter {
    pub status: Option<ObservationStatus>,
    pub q: String,
    pub mine: bool,
}

fn crumbs<'a>(p: &'a Project, last: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some((l, list)) => {
            c.push(("Observations", Some(list)));
            c.push((l, None));
        }
        None => c.push(("Observations", None)),
    }
    c
}

pub fn list(req: &Req<'_>, id: Id<Project>, filter: &Filter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Observations) < Level::Read {
        return not_found(req);
    }
    let all = view::observations_for(state, req.actor(), id);
    let count = |s: ObservationStatus| all.iter().filter(|x| x.status == s).count();
    let q = filter.q.to_lowercase();
    let today = req.today();
    let shown: Vec<&Observation> = all
        .iter()
        .copied()
        .filter(|i| filter.status.is_none_or(|s| i.status == s))
        .filter(|i| !filter.mine || i.ball_in_court.contains(&req.person))
        .filter(|i| q.is_empty() || i.title.to_lowercase().contains(&q) || i.location.to_lowercase().contains(&q) || i.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/observations");
    let tab = |label: &str, status: Option<ObservationStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "Observations" } div class="sub" { "Small items to fix or finish before closeout" } }
            div class="actions" {
                a class="btn" href=(format!("{base}.csv")) { (Icon::Download.svg()) "Export CSV" }
                @if req.allowed("observation.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "Add item" } }
            }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Open", Some(ObservationStatus::Open), count(ObservationStatus::Open)))
            (tab("Ready for review", Some(ObservationStatus::ReadyForReview), count(ObservationStatus::ReadyForReview)))
            (tab("Closed", Some(ObservationStatus::Closed), count(ObservationStatus::Closed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title, location or number" aria-label="Search observations";
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Waiting on me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Observations), "No items here", if all.is_empty() { "Add the first item to get started." } else { "No item matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Item" } th { "Location" } th { "Priority" } th { "Status" } th { "Ball in court" } th { "Due" } } }
                    tbody {
                        @for i in &shown {
                            tr {
                                td class="nowrap" { (i.number) }
                                td { a class="row-link" href=(format!("{base}/{}", i.id)) { (i.title) } }
                                td { (i.location) }
                                td { (priority_tag(i.priority)) }
                                td { (status_pill(i.status)) }
                                td { (view::people_names(state, &i.ball_in_court)) }
                                td class="nowrap" { @if let Some(d) = i.due { span class=(if i.is_overdue(today) { "overdue" } else { "" }) { (d.pretty()) } } }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, "Observations", Nav::Observations(id), &crumbs(p, None), body)
}

pub fn detail(req: &Req<'_>, id: Id<Project>, iid: Id<Observation>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = state.observations.get(iid).filter(|i| i.project == id && view::can_see_observation(state, req.actor(), i)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/observations");
    let here = format!("{base}/{}", i.id);
    let ids = [("project", id.to_string()), ("id", i.id.to_string())];
    let level = view::level(state, req.actor(), id, Tool::Observations);
    let editable = can_edit(state, req.actor(), i);
    let manager_like = level == Level::Admin || i.manager == Some(req.person);
    let assignee = i.assignees.contains(&req.person);
    let standard = level >= Level::Standard;
    let can_attach = i.status != ObservationStatus::Closed && (editable || (standard && assignee));
    let body = html! {
        div class="page-head" {
            div {
                h1 { (i.label()) ": " (i.title) " " (status_pill(i.status)) }
                div class="sub" { "Added " (req.when(i.created.at)) @if let Some(b) = i.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if standard && i.status == ObservationStatus::Open && (assignee || manager_like) { (button_form(req, "observation.complete", &here, &ids, "Mark work done", "primary")) }
                @if standard && manager_like && i.status != ObservationStatus::Closed { (button_form(req, "observation.close", &here, &ids, "Close item", if i.status == ObservationStatus::ReadyForReview { "primary" } else { "" })) }
                @if standard && manager_like && i.status == ObservationStatus::Closed { (button_form(req, "observation.reopen", &here, &ids, "Reopen", "")) }
                @if standard && (level == Level::Admin || (i.status == ObservationStatus::Open && i.created.by == Some(req.person))) { (button_form(req, "observation.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="detail" {
            div {
                @if !i.description.is_empty() { div class="card" { h2 { "Description" } p class="prose" { (i.description) } } }
                @if !i.corrective_action.is_empty() { div class="card" { h2 { "Corrective action" } p class="prose" { (i.corrective_action) } } }
                @if i.status == ObservationStatus::ReadyForReview && standard && manager_like {
                    div class="card" {
                        h2 { "Not right yet?" }
                        (command_form(req, "observation.send_back", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id);
                            (field("What still needs doing", "sb_comment", true, true, html! { textarea id="sb_comment" name="comment" class="short" required {} }))
                            div class="form-actions" { button type="submit" class="btn" { "Send back" } }
                        }))
                    }
                }
                div class="card" {
                    h2 { "Comments" }
                    @if i.comments.is_empty() { p class="muted" { "No comments yet." } }
                    @for c in &i.comments {
                        div class="response" {
                            div class="meta" { span { strong { @if let Some(b) = c.by { (view::person_name(state, b)) } @else { "System" } } " · " (req.when(c.at)) } }
                            p class="prose" { (c.body) }
                        }
                    }
                    @if standard && i.status != ObservationStatus::Closed {
                        (command_form(req, "observation.comment", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id);
                            (field("Add a comment", "body", true, true, html! { textarea id="body" name="body" class="short" required {} }))
                            div class="form-actions" { button type="submit" class="btn primary" { "Post comment" } }
                        }))
                    }
                }
            }
            div class="stack" {
                div class="card" {
                    h2 { "Details" }
                    dl class="facts" {
                        dt { "Priority" } dd { (priority_tag(i.priority)) }
                        dt { "Location" } dd { (dash(&i.location)) }
                        dt { "Category" } dd { (dash(&i.kind)) }
                        dt { "Observed" } dd { @if let Some(d) = i.observed { (d.pretty()) } @else { "—" } }
                        dt { "Manager" } dd { @if let Some(m) = i.manager { (view::person_name(state, m)) } @else { "—" } }
                        dt { "Assigned to" } dd { @if i.assignees.is_empty() { "—" } @else { (view::people_names(state, &i.assignees)) } }
                        dt { "Ball in court" } dd { @if i.ball_in_court.is_empty() { "—" } @else { (view::people_names(state, &i.ball_in_court)) } }
                        dt { "Due" } dd { @if let Some(d) = i.due { span class=(if i.is_overdue(req.today()) { "overdue" } else { "" }) { (d.pretty()) } } @else { "—" } }
                        @if let Some(w) = i.work_done { dt { "Work done" } dd { (req.when(w.at)) } }
                        @if let Some(c) = i.closed { dt { "Closed" } dd { (req.when(c.at)) } }
                    }
                }
                div class="card" {
                    h2 { "Files" }
                    @if i.attachments.is_empty() { p class="muted" { "No files attached." } }
                    @for a in &i.attachments {
                        div class="row" {
                            a href=(format!("{here}/files/{}", a.sha256)) { (a.name) }
                            span class="muted small" { (human_size(a.size)) }
                            @if can_attach { (button_form(req, "observation.detach", &here, &[("project", id.to_string()), ("id", i.id.to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
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
    layout::app(req, &i.label(), Nav::Observations(id), &crumbs(p, Some((&i.label(), base.clone()))), body)
}

pub fn form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<Observation>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(iid) => match state
            .observations
            .get(iid)
            .filter(|i| i.project == id && view::can_see_observation(state, req.actor(), i) && can_edit(state, req.actor(), i))
        {
            Some(i) => Some(i),
            None => return not_found(req),
        },
        None => {
            if !req.allowed("observation.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let people = user_options(req, id, true);
    let base = format!("/projects/{id}/observations");
    let (command, next, back) = match current {
        Some(i) => ("observation.update", format!("{base}/{}", i.id), format!("{base}/{}/edit", i.id)),
        None => ("observation.create", format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&Observation) -> String| current.map(f).unwrap_or_default();
    let manager = current.and_then(|i| i.manager).unwrap_or(req.person);
    let priority = current.map(|i| i.priority).unwrap_or_default();
    let title = if current.is_some() { "Edit observation" } else { "Add observation" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(i) = current { input type="hidden" name="id" value=(i.id); }
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(text(|i| i.title.clone())); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (text(|i| i.description.clone())) } }))
                    (field("Corrective action", "corrective_action", false, true, html! { textarea id="corrective_action" name="corrective_action" class="short" { (text(|i| i.corrective_action.clone())) } }))
                    (field("Location", "location", false, false, html! { input type="text" id="location" name="location" value=(text(|i| i.location.clone())); }))
                    (field("Category", "kind", false, false, html! { input type="text" id="kind" name="kind" placeholder="Safety, quality, housekeeping…" value=(text(|i| i.kind.clone())); }))
                    (field("Date observed", "observed", false, false, html! { input type="date" id="observed" name="observed" value=(current.map(|i| i.observed).unwrap_or_else(|| Some(req.today())).map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Priority", "priority", false, false, html! { select id="priority" name="priority" { @for pr in [Priority::Low, Priority::Medium, Priority::High, Priority::Urgent] { option value=(pr.key()) selected[pr == priority] { (pr.label()) } } } }))
                    (field("Due date", "due", false, false, html! { input type="date" id="due" name="due" value=(current.and_then(|i| i.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Manager (accepts the work)", "manager", false, false, html! { select id="manager" name="manager" { @for x in &people { option value=(x.id) selected[x.id == manager] { (x.name()) } } } }))
                    div class="field full" {
                        div class="label" { "Assigned to" }
                        input type="hidden" name="assignees[]" value="";
                        details class="multi" {
                            summary { "Choose who does the work" }
                            div class="options" { @for x in &people {
                                label { input type="checkbox" name="assignees[]" value=(x.id) checked[current.is_some_and(|i| i.assignees.contains(&x.id))]; (x.name()) }
                            } }
                        }
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(current.map(|i| format!("{base}/{}", i.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { "Add item" }) }
                }
            }))
        }
    };
    layout::app(req, title, Nav::Observations(id), &crumbs(p, Some((title, base.clone()))), body)
}

/// The observations as CSV.
pub fn csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let day = |t: buildkiosk_core::Timestamp| t.date_at(state.utc_offset()).to_string();
    let mut out = String::from("Number,Title,Location,Category,Priority,Status,Manager,Assigned to,Due,Created,Closed\r\n");
    for i in view::observations_for(state, req.actor(), id) {
        let row = [
            i.number.to_string(),
            i.title.clone(),
            i.location.clone(),
            i.kind.clone(),
            i.priority.label().to_string(),
            i.status.label().to_string(),
            i.manager.map(|m| view::person_name(state, m)).unwrap_or_default(),
            view::people_names(state, &i.assignees),
            i.due.map(|d| d.to_string()).unwrap_or_default(),
            day(i.created.at),
            i.closed.map(|c| day(c.at)).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
