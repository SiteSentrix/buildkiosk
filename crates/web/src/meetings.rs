//! Meeting pages: list with open action items, detail and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::meeting::visible;
use buildkiosk_engine::model::meeting::{Meeting, MeetingStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

fn status_pill(s: MeetingStatus) -> Markup {
    html! { span class=(format!("pill {}", if s == MeetingStatus::Issued { "green" } else { "violet" })) { (s.label()) } }
}

pub fn list(req: &Req<'_>, id: Id<Project>, status: Option<MeetingStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Meetings);
    if lvl < Level::Read {
        return not_found(req);
    }
    let needle = q.trim().to_lowercase();
    let all = visible(state, id);
    let shown: Vec<&Meeting> = all
        .iter()
        .copied()
        .filter(|m| status.is_none_or(|s| m.status == s))
        .filter(|m| {
            needle.is_empty()
                || m.title.to_lowercase().contains(&needle)
                || m.category.to_lowercase().contains(&needle)
                || m.agenda.iter().any(|a| a.title.to_lowercase().contains(&needle))
        })
        .collect();
    let today = req.today();
    // Every open action across the project's meetings, soonest due first.
    let mut open: Vec<(&Meeting, usize)> =
        all.iter().flat_map(|m| m.actions.iter().enumerate().filter(|(_, a)| a.is_open()).map(move |(i, _)| (*m, i))).collect();
    open.sort_by_key(|(m, i)| (m.actions.get(*i).and_then(|a| a.due).is_none(), m.actions.get(*i).and_then(|a| a.due), m.number));
    let base = format!("/projects/{id}/meetings");
    let body = html! {
        div class="page-head" {
            div { h1 { "Meetings" } div class="sub" { "Agendas, attendance, minutes and who agreed to do what" } }
            div class="actions" { @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New meeting" } } }
        }
        @if !open.is_empty() {
            div class="card" {
                h2 { "Open action items" }
                table class="data" {
                    thead { tr { th { "Action" } th { "Who" } th { "Due" } th { "Meeting" } } }
                    tbody { @for (m, i) in open.iter().take(15) { @if let Some(a) = m.actions.get(*i) { tr {
                        td { (a.description) }
                        td { (a.assignee.map(|x| view::person_name(state, x)).unwrap_or_default()) }
                        td class="nowrap" { @if let Some(d) = a.due { @if d < today { span class="overdue" { (d.to_string()) } } @else { (d.to_string()) } } }
                        td { a href=(format!("{base}/{}", m.id)) { (m.label()) } }
                    } } } }
                }
            }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search title, type or agenda" aria-label="Search meetings";
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in MeetingStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Meetings), "No meetings", "Plan a meeting to keep its agenda, minutes and action items together.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Date" } th { "Meeting" } th { "Type" } th { "Open actions" } th { "Status" } } }
                tbody { @for m in &shown { tr {
                    td { (m.number) }
                    td class="nowrap" { (m.date.to_string()) }
                    td { a href=(format!("{base}/{}", m.id)) { (m.title) } }
                    td { (dash(&m.category)) }
                    td { (m.actions.iter().filter(|a| a.is_open()).count()) }
                    td { (status_pill(m.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Meetings",
        Nav::Meetings(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Meetings", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, mid: Id<Meeting>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Meetings);
    let Some(m) = state.meetings.get(mid).filter(|m| m.project == id && m.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/meetings");
    let here = format!("{base}/{mid}");
    let ids = [("project", id.to_string()), ("id", mid.to_string())];
    let admin = lvl == Level::Admin;
    let author = m.created.by.is_some() && m.created.by == req.actor().person();
    let can_edit = req.allowed("meeting.update", Some(id)) && (admin || (author && m.status == MeetingStatus::Scheduled));
    let today = req.today();
    let can_task = req.allowed("meeting.action_to_task", Some(id)) && view::level(state, req.actor(), id, Tool::Tasks) >= Level::Standard;
    let earlier_open = can_edit
        && state.meetings.iter().any(|x| {
            x.project == id && x.deleted.is_none() && x.id != m.id && (x.date, x.number) < (m.date, m.number) && x.actions.iter().any(|a| a.is_open())
        });
    let body = html! {
        div class="page-head" {
            div { h1 { (m.label()) ": " (m.title) " " (status_pill(m.status)) } div class="sub" { (m.date.pretty()) @if !m.time.is_empty() { " · " (m.time) } @if !m.location.is_empty() { " · " (m.location) } @if !m.category.is_empty() { " · " (m.category) } } }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                a class="btn" href=(format!("{here}/signin.pdf")) { "Sign-in sheet" }
                @if can_edit { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if can_edit && m.status == MeetingStatus::Scheduled { (button_form(req, "meeting.issue", &here, &ids, "Issue minutes", "primary")) }
                @if admin && m.status == MeetingStatus::Issued { (button_form(req, "meeting.reopen", &here, &ids, "Reopen minutes", "")) }
                @if can_edit { (button_form(req, "meeting.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if req.allowed("meeting_template.save", Some(id)) {
            div class="card" {
                details class="disclosure" {
                    summary { "Save as a template" }
                    (command_form(req, "meeting_template.save", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="from_meeting" value=(mid);
                        div class="form-grid" { (field("Template name", "tpl_name", true, false, html! { input type="text" id="tpl_name" name="name" required value=(if m.category.is_empty() { m.title.clone() } else { m.category.clone() }); })) }
                        p class="muted small" { "Keeps the type, time, place, attendees and agenda topics (not the notes or actions) to start the next meeting from." }
                        div class="form-actions" { button type="submit" class="btn small" { "Save template" } }
                    }))
                }
            }
        }
        div class="card" {
            h2 { "Attendees" }
            @if m.attendees.is_empty() { p class="muted" { "Nobody invited." } } @else {
                ul class="plain" { @for a in &m.attendees { li { (view::person_name(state, a.person)) " " @if a.present { span class="pill green" { "Present" } } @else { span class="pill" { "Not recorded" } } } } }
            }
        }
        div class="card" {
            h2 { "Agenda and notes" }
            @if m.agenda.is_empty() { p class="muted" { "No agenda items." } } @else {
                ol { @for a in &m.agenda { li { strong { (a.title) } @if !a.notes.is_empty() { p class="prose" { (a.notes) } } } } }
            }
            @if !m.minutes.is_empty() { h3 { "Other minutes" } p class="prose" { (m.minutes) } }
        }
        div class="card" {
            h2 { "Action items" }
            @if m.actions.is_empty() { p class="muted" { "No action items." } } @else {
                table class="data" {
                    thead { tr { th { "Action" } th { "Who" } th { "Due" } th { "Done" } @if can_task { th {} } } }
                    tbody { @for (i, a) in m.actions.iter().enumerate() { tr {
                        td {
                            (a.description)
                            @if let Some(f) = a.from_meeting.and_then(|f| state.meetings.get(f)) { " " span class="tag" { "From " a href=(format!("{base}/{}", f.id)) { (f.label()) } } }
                            @if a.carried { " " span class="tag" { "Carried forward" } }
                            @if let Some(t) = a.task.and_then(|t| state.tasks.get(t)) { " " span class="tag" { "Task " a href=(format!("/projects/{id}/tasks/{}", t.id)) { "#" (t.number) } } }
                        }
                        td { (a.assignee.map(|x| view::person_name(state, x)).unwrap_or_default()) }
                        td class="nowrap" { @if let Some(d) = a.due { @if d < today && !a.done { span class="overdue" { (d.to_string()) } } @else { (d.to_string()) } } }
                        td {
                            @if req.allowed("meeting.set_action", Some(id)) && (admin || author || (a.assignee.is_some() && a.assignee == req.actor().person())) {
                                (button_form(req, "meeting.set_action", &here, &[("project", id.to_string()), ("id", mid.to_string()), ("index", i.to_string()), ("done", (!a.done).to_string())], if a.done { "Done · reopen" } else { "Mark done" }, "small"))
                            } @else if a.done { span class="pill green" { "Done" } } @else if a.carried { span class="pill" { "Carried" } } @else { span class="pill amber" { "Open" } }
                        }
                        @if can_task { td { @if a.task.is_none() && !a.done && (admin || author || (a.assignee.is_some() && a.assignee == req.actor().person())) {
                            (button_form(req, "meeting.action_to_task", &here, &[("project", id.to_string()), ("id", mid.to_string()), ("index", i.to_string())], "Make a task", "small"))
                        } } }
                    } } }
                }
            }
        }
        @if earlier_open {
            div class="card" {
                h2 { "Open actions from earlier meetings" }
                p class="muted small" { "Bring the actions still open from the previous meeting of this type into this one. They are marked as carried forward there, so each is counted once." }
                (button_form(req, "meeting.carry_actions", &here, &ids, "Carry open actions forward", "small"))
            }
        }
        @if req.allowed("meeting.repeat", Some(id)) {
            div class="card" {
                details class="disclosure" {
                    summary { "Schedule more of these meetings" }
                    (command_form(req, "meeting.repeat", &here, &base, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(mid);
                        div class="form-grid" {
                            (field("How often", "rep_every", true, false, html! { select id="rep_every" name="every" {
                                option value="weekly" { "Every week" } option value="biweekly" { "Every two weeks" } option value="monthly" { "Every month" }
                            } }))
                            (field("How many more", "rep_count", true, false, html! { input type="number" id="rep_count" name="count" min="1" max="26" value="4" required; }))
                        }
                        p class="muted small" { "Each has the same title, type, time, place, invitees and agenda topics, on the dates after this one." }
                        div class="form-actions" { button type="submit" class="btn primary" { "Schedule" } }
                    }))
                }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if m.attachments.is_empty() { p class="muted" { "No files attached." } } @else {
                ul class="plain" { @for f in &m.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if can_edit { " " (button_form(req, "meeting.detach", &here, &[("project", id.to_string()), ("id", mid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if can_edit {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
    };
    layout::app(
        req,
        &m.label(),
        Nav::Meetings(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Meetings", Some(base)), (&m.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<Meeting>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.meetings.get(c).filter(|m| m.project == id && m.deleted.is_none()) {
            Some(m) => Some(m),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "meeting.update" } else { "meeting.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/meetings");
    let back = cur.map(|m| format!("{base}/{}/edit", m.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|m| format!("{base}/{}", m.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let people = user_options(req, id, true);
    let invited = |pid| cur.is_some_and(|m| m.attendees.iter().any(|a| a.person == pid));
    let present = |pid| cur.is_some_and(|m| m.attendees.iter().any(|a| a.person == pid && a.present));
    let tpls = buildkiosk_engine::cmd::meeting::templates(state, id);
    let agenda_rows = cur.map(|m| m.agenda.len()).unwrap_or(0) + 3;
    let action_rows = cur.map(|m| m.actions.len()).unwrap_or(0) + 3;
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit meeting" } else { "New meeting" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(m) = cur { input type="hidden" name="id" value=(m.id); }
                div class="form-grid" {
                    @if cur.is_none() && !tpls.is_empty() {
                        (field("Start from a template", "template", false, true, html! { select id="template" name="template" {
                            option value="" { "None" }
                            @for t in &tpls { option value=(t.id) { (t.name) } }
                        } }))
                        p class="muted small" { "Whatever you leave blank below (type, time, place, attendees, agenda) comes from the template." }
                    }
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required value=(cur.map(|m| m.title.as_str()).unwrap_or_default()); }))
                    (field("Date", "date", true, false, html! { input type="date" id="date" name="date" required value=(cur.map(|m| m.date.to_string()).unwrap_or_else(|| req.today().to_string())); }))
                    (field("Time", "time", false, false, html! { input type="text" id="time" name="time" placeholder="9:00 AM" value=(cur.map(|m| m.time.as_str()).unwrap_or_default()); }))
                    (field("Type", "category", false, false, html! { input type="text" id="category" name="category" placeholder="Coordination" value=(cur.map(|m| m.category.as_str()).unwrap_or_default()); }))
                    (field("Location", "location", false, false, html! { input type="text" id="location" name="location" value=(cur.map(|m| m.location.as_str()).unwrap_or_default()); }))
                }
                h2 { "Attendees" }
                input type="hidden" name="invitees[]" value="";
                @if cur.is_some() { input type="hidden" name="present[]" value=""; }
                table class="data" {
                    thead { tr { th { "Person" } th { "Invited" } @if cur.is_some() { th { "Present" } } } }
                    tbody { @for x in &people { tr {
                        td { (x.name()) }
                        td { input type="checkbox" name="invitees[]" value=(x.id) checked[invited(x.id)] aria-label=(format!("Invite {}", x.name())); }
                        @if cur.is_some() { td { input type="checkbox" name="present[]" value=(x.id) checked[present(x.id)] aria-label=(format!("{} was present", x.name())); } }
                    } } }
                }
                h2 { "Agenda and notes" }
                table class="data" {
                    thead { tr { th { "Topic" } th { "Notes" } } }
                    tbody { @for i in 0..agenda_rows { @let a = cur.and_then(|m| m.agenda.get(i)); tr {
                        td { input type="text" name=(format!("agenda.{i}.title")) value=(a.map(|x| x.title.as_str()).unwrap_or_default()) aria-label=(format!("Topic {}", i + 1)); }
                        td { input type="text" name=(format!("agenda.{i}.notes")) value=(a.map(|x| x.notes.as_str()).unwrap_or_default()) aria-label=(format!("Notes on topic {}", i + 1)); }
                    } } }
                }
                (field("Other minutes", "minutes", false, true, html! { textarea id="minutes" name="minutes" { (cur.map(|m| m.minutes.as_str()).unwrap_or_default()) } }))
                h2 { "Action items" }
                table class="data" {
                    thead { tr { th { "Action" } th { "Who" } th { "Due" } th { "Done" } } }
                    tbody { @for i in 0..action_rows { @let a = cur.and_then(|m| m.actions.get(i)); tr {
                        td { input type="text" name=(format!("actions.{i}.description")) value=(a.map(|x| x.description.as_str()).unwrap_or_default()) aria-label=(format!("Action {}", i + 1)); }
                        td { select name=(format!("actions.{i}.assignee")) aria-label=(format!("Who does action {}", i + 1)) {
                            option value="" { "—" }
                            @for x in &people { option value=(x.id) selected[a.is_some_and(|y| y.assignee == Some(x.id))] { (x.name()) } }
                        } }
                        td { input type="date" name=(format!("actions.{i}.due")) value=(a.and_then(|x| x.due).map(|d| d.to_string()).unwrap_or_default()) aria-label=(format!("Due date of action {}", i + 1)); }
                        td { input type="checkbox" name=(format!("actions.{i}.done")) value="true" checked[a.is_some_and(|y| y.done)] aria-label=(format!("Action {} is done", i + 1)); }
                    } } }
                }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|m| format!("{base}/{}", m.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Meeting",
        Nav::Meetings(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Meetings", Some(base)), ("Meeting", None)],
        body,
    )
}
