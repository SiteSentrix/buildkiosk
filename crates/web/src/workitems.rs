//! Pages for the tools that share the work item workflow (punch list and observations): list,
//! detail, form and CSV export, written once over [`WorkPage`].

use buildkiosk_core::Id;
use buildkiosk_engine::model::incident::{Incident, Severity};
use buildkiosk_engine::model::observation::Observation;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::punch::PunchItem;
use buildkiosk_engine::model::task::Task;
use buildkiosk_engine::model::work::WorkStatus;
use buildkiosk_engine::model::{Choice, Level, Priority};
use buildkiosk_engine::workitem::{WorkItem, can_edit, can_see, items_for, label};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

/// How one work item tool presents itself.
pub trait WorkPage: WorkItem {
    /// The command prefix: `punch` for `punch.create`.
    const PREFIX: &'static str;
    /// The page title: "Punch list".
    const TITLE: &'static str;
    const SUBTITLE: &'static str;
    const ADD: &'static str;
    /// What the free-text kind field is called, with an example.
    const KIND_LABEL: &'static str;
    const KIND_HINT: &'static str;
    const EMPTY_HINT: &'static str;

    fn nav(project: Id<Project>) -> Nav;
    /// Cards shown above the comments (kind-specific).
    fn extra_cards(&self) -> Markup {
        html! {}
    }
    /// Rows of the details panel (kind-specific).
    fn extra_facts(&self) -> Markup {
        html! {}
    }
    /// Form fields (kind-specific); `item` is `None` on the new form.
    fn extra_fields(_item: Option<&Self>, _req: &Req<'_>) -> Markup {
        html! {}
    }
}

impl WorkPage for PunchItem {
    const PREFIX: &'static str = "punch";
    const TITLE: &'static str = "Punch list";
    const SUBTITLE: &'static str = "Small items to fix or finish before closeout";
    const ADD: &'static str = "Add punch item";
    const KIND_LABEL: &'static str = "Type";
    const KIND_HINT: &'static str = "Paint, electrical, glazing…";
    const EMPTY_HINT: &'static str = "Add the first item to get started.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Punch(project)
    }
}

impl WorkPage for Task {
    const PREFIX: &'static str = "task";
    const TITLE: &'static str = "Tasks";
    const SUBTITLE: &'static str = "Things to get done, with an owner and a due date";
    const ADD: &'static str = "Add task";
    const KIND_LABEL: &'static str = "Category";
    const KIND_HINT: &'static str = "Closeout, procurement, permits…";
    const EMPTY_HINT: &'static str = "Add the first task to get started.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Tasks(project)
    }
}

impl WorkPage for Incident {
    const PREFIX: &'static str = "incident";
    const TITLE: &'static str = "Incidents";
    const SUBTITLE: &'static str = "Injuries, near misses and damage, with the investigation and what was done about it";
    const ADD: &'static str = "Report incident";
    const KIND_LABEL: &'static str = "Type";
    const KIND_HINT: &'static str = "Fall, struck by, equipment, spill…";
    const EMPTY_HINT: &'static str = "Report an incident to record what happened and what was done.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Incidents(project)
    }

    fn extra_cards(&self) -> Markup {
        html! {
            @if !self.people_involved.is_empty() { div class="card" { h2 { "People involved" } p class="prose" { (self.people_involved) } } }
            @if !self.root_cause.is_empty() { div class="card" { h2 { "Root cause" } p class="prose" { (self.root_cause) } } }
            @if !self.corrective_action.is_empty() { div class="card" { h2 { "Corrective action" } p class="prose" { (self.corrective_action) } } }
        }
    }

    fn extra_facts(&self) -> Markup {
        html! {
            dt { "Occurred" } dd { @if let Some(d) = self.occurred { (d.pretty()) } @else { "—" } }
            dt { "Severity" } dd { (self.severity.label()) }
            dt { "Recordable" } dd { (if self.recordable { "Yes" } else { "No" }) }
        }
    }

    fn extra_fields(item: Option<&Self>, req: &Req<'_>) -> Markup {
        let occurred = item.map(|i| i.occurred).unwrap_or_else(|| Some(req.today())).map(|d| d.to_string()).unwrap_or_default();
        let severity = item.map(|i| i.severity).unwrap_or_default();
        let recordable = item.is_some_and(|i| i.recordable);
        let text = |f: fn(&Incident) -> &str| item.map(|i| f(i).to_string()).unwrap_or_default();
        html! {
            (field("Date it happened", "occurred", false, false, html! { input type="date" id="occurred" name="occurred" value=(occurred); }))
            (field("Severity", "severity", false, false, html! { select id="severity" name="severity" { @for s in Severity::ALL { option value=(s.key()) selected[*s == severity] { (s.label()) } } } }))
            (field("Recordable", "recordable", false, false, html! { select id="recordable" name="recordable" {
                option value="false" selected[!recordable] { "No" }
                option value="true" selected[recordable] { "Yes, enter it on the injury log" }
            } }))
            (field("People involved and witnesses", "people_involved", false, true, html! { textarea id="people_involved" name="people_involved" class="short" { (text(|i| &i.people_involved)) } }))
            (field("Root cause", "root_cause", false, true, html! { textarea id="root_cause" name="root_cause" class="short" { (text(|i| &i.root_cause)) } }))
            (field("Corrective action", "corrective_action", false, true, html! { textarea id="corrective_action" name="corrective_action" class="short" { (text(|i| &i.corrective_action)) } }))
        }
    }
}

impl WorkPage for Observation {
    const PREFIX: &'static str = "observation";
    const TITLE: &'static str = "Observations";
    const SUBTITLE: &'static str = "Safety, quality and housekeeping findings";
    const ADD: &'static str = "Add observation";
    const KIND_LABEL: &'static str = "Category";
    const KIND_HINT: &'static str = "Safety, quality, housekeeping…";
    const EMPTY_HINT: &'static str = "Add the first observation to get started.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Observations(project)
    }

    fn extra_cards(&self) -> Markup {
        html! { @if !self.corrective_action.is_empty() { div class="card" { h2 { "Corrective action" } p class="prose" { (self.corrective_action) } } } }
    }

    fn extra_facts(&self) -> Markup {
        html! { dt { "Observed" } dd { @if let Some(d) = self.observed { (d.pretty()) } @else { "—" } } }
    }

    fn extra_fields(item: Option<&Self>, req: &Req<'_>) -> Markup {
        let observed = item.map(|i| i.observed).unwrap_or_else(|| Some(req.today())).map(|d| d.to_string()).unwrap_or_default();
        let action = item.map(|i| i.corrective_action.clone()).unwrap_or_default();
        html! {
            (field("Date observed", "observed", false, false, html! { input type="date" id="observed" name="observed" value=(observed); }))
            (field("Corrective action", "corrective_action", false, true, html! { textarea id="corrective_action" name="corrective_action" class="short" { (action) } }))
        }
    }
}

fn status_pill(s: WorkStatus) -> Markup {
    let class = match s {
        WorkStatus::Open => "blue",
        WorkStatus::ReadyForReview => "amber",
        WorkStatus::Closed => "green",
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
    pub status: Option<WorkStatus>,
    pub q: String,
    pub mine: bool,
}

fn crumbs<'a, T: WorkPage>(p: &'a Project, last: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
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

fn cmd<T: WorkPage>(verb: &str) -> String {
    format!("{}.{verb}", T::PREFIX)
}

pub fn list<T: WorkPage>(req: &Req<'_>, id: Id<Project>, filter: &Filter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, T::TOOL) < Level::Read {
        return not_found(req);
    }
    let all = items_for::<T>(state, req.actor(), id);
    let count = |s: WorkStatus| all.iter().filter(|x| x.status == s).count();
    let q = filter.q.to_lowercase();
    let today = req.today();
    let shown: Vec<&T> = all
        .iter()
        .copied()
        .filter(|i| filter.status.is_none_or(|s| i.status == s))
        .filter(|i| !filter.mine || i.ball_in_court.contains(&req.person))
        .filter(|i| q.is_empty() || i.title.to_lowercase().contains(&q) || i.location.to_lowercase().contains(&q) || i.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/{}", T::PATH);
    let tab = |label: &str, status: Option<WorkStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { (T::TITLE) } div class="sub" { (T::SUBTITLE) } }
            div class="actions" {
                a class="btn" href=(format!("{base}.csv")) { (Icon::Download.svg()) "Export CSV" }
                @if req.allowed(&cmd::<T>("create"), Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) (T::ADD) } }
            }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Open", Some(WorkStatus::Open), count(WorkStatus::Open)))
            (tab("Ready for review", Some(WorkStatus::ReadyForReview), count(WorkStatus::ReadyForReview)))
            (tab("Closed", Some(WorkStatus::Closed), count(WorkStatus::Closed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title, location or number" aria-label=(format!("Search {}", T::TITLE.to_lowercase()));
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Waiting on me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(T::TOOL), "No items here", if all.is_empty() { T::EMPTY_HINT } else { "No item matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Title" } th { "Location" } th { "Priority" } th { "Status" } th { "Ball in court" } th { "Due" } } }
                    tbody {
                        @for i in &shown {
                            tr {
                                td class="nowrap" { (i.number) }
                                td { a class="row-link" href=(format!("{base}/{}", i.id())) { (i.title) } }
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
    layout::app(req, T::TITLE, T::nav(id), &crumbs::<T>(p, None), body)
}

pub fn detail<T: WorkPage>(req: &Req<'_>, id: Id<Project>, iid: Id<T>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i)) else { return not_found(req) };
    let base = format!("/projects/{id}/{}", T::PATH);
    let here = format!("{base}/{}", i.id());
    let ids = [("project", id.to_string()), ("id", i.id().to_string())];
    let level = view::level(state, req.actor(), id, T::TOOL);
    let editable = can_edit(state, req.actor(), i);
    let manager_like = level == Level::Admin || i.manager == Some(req.person);
    let assignee = i.assignees.contains(&req.person);
    let standard = level >= Level::Standard;
    let can_attach = i.status != WorkStatus::Closed && (editable || (standard && assignee));
    let name = label(i);
    let body = html! {
        div class="page-head" {
            div {
                h1 { (name) ": " (i.title) " " (status_pill(i.status)) }
                div class="sub" { "Added " (req.when(i.created.at)) @if let Some(b) = i.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if standard && i.status == WorkStatus::Open && (assignee || manager_like) { (button_form(req, &cmd::<T>("complete"), &here, &ids, "Mark work done", "primary")) }
                @if standard && manager_like && i.status != WorkStatus::Closed { (button_form(req, &cmd::<T>("close"), &here, &ids, "Close item", if i.status == WorkStatus::ReadyForReview { "primary" } else { "" })) }
                @if standard && manager_like && i.status == WorkStatus::Closed { (button_form(req, &cmd::<T>("reopen"), &here, &ids, "Reopen", "")) }
                @if standard && (level == Level::Admin || (i.status == WorkStatus::Open && i.created.by == Some(req.person))) { (button_form(req, &cmd::<T>("delete"), &base, &ids, "Delete", "danger")) }
            }
        }
        div class="detail" {
            div {
                @if !i.description.is_empty() { div class="card" { h2 { "Description" } p class="prose" { (i.description) } } }
                (i.extra_cards())
                @if i.status == WorkStatus::ReadyForReview && standard && manager_like {
                    div class="card" {
                        h2 { "Not right yet?" }
                        (command_form(req, &cmd::<T>("send_back"), &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id());
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
                    @if standard && i.status != WorkStatus::Closed {
                        (command_form(req, &cmd::<T>("comment"), &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id());
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
                        dt { (T::KIND_LABEL) } dd { (dash(&i.kind)) }
                        (i.extra_facts())
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
                            @if can_attach { (button_form(req, &cmd::<T>("detach"), &here, &[("project", id.to_string()), ("id", i.id().to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
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
    layout::app(req, &name, T::nav(id), &crumbs::<T>(p, Some((&name, base.clone()))), body)
}

pub fn form<T: WorkPage>(req: &Req<'_>, id: Id<Project>, existing: Option<Id<T>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(iid) => {
            match T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i) && can_edit(state, req.actor(), *i)) {
                Some(i) => Some(i),
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
    let people = user_options(req, id, true);
    let base = format!("/projects/{id}/{}", T::PATH);
    let (command, next, back) = match current {
        Some(i) => (cmd::<T>("update"), format!("{base}/{}", i.id()), format!("{base}/{}/edit", i.id())),
        None => (cmd::<T>("create"), format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&T) -> String| current.map(f).unwrap_or_default();
    let manager = current.and_then(|i| i.manager).unwrap_or(req.person);
    let priority = current.map(|i| i.priority).unwrap_or_default();
    let title = if current.is_some() { format!("Edit {}", T::NOUN) } else { T::ADD.to_string() };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, &command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(i) = current { input type="hidden" name="id" value=(i.id()); }
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(text(|i| i.title.clone())); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (text(|i| i.description.clone())) } }))
                    (field("Location", "location", false, false, html! { input type="text" id="location" name="location" value=(text(|i| i.location.clone())); }))
                    (field(T::KIND_LABEL, "kind", false, false, html! { input type="text" id="kind" name="kind" placeholder=(T::KIND_HINT) value=(text(|i| i.kind.clone())); }))
                    (T::extra_fields(current, req))
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
                    a class="btn" href=(current.map(|i| format!("{base}/{}", i.id())).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { T::ADD }) }
                }
            }))
        }
    };
    layout::app(req, &title, T::nav(id), &crumbs::<T>(p, Some((&title, base.clone()))), body)
}

/// The items as CSV.
pub fn csv<T: WorkPage>(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let day = |t: buildkiosk_core::Timestamp| t.date_at(state.utc_offset()).to_string();
    let mut out = format!("Number,Title,Location,{},Priority,Status,Manager,Assigned to,Due,Created,Closed\r\n", T::KIND_LABEL);
    for i in items_for::<T>(state, req.actor(), id) {
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
