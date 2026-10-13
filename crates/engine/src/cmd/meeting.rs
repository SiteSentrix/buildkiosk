//! Meetings: plan with an agenda and invitees, record who came, what was said and who agreed to
//! do what, then issue the minutes. Issuing tells the attendees and the people with actions.
//!
//! Who may do what: the author (or a Meetings admin) edits a meeting until its minutes are issued;
//! an admin changes issued minutes. Anyone given an action may tick it off, at any time.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::meeting::{AgendaItem, Attendee, Meeting, MeetingAction, MeetingStatus};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "meeting.create",
        "Create meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project: project id; title: text; date: YYYY-MM-DD; category?: text; time?: text; location?: text; invitees?: [person ids]; agenda?: [{title, notes?}]; actions?: [{description, assignee?: person id, due?: YYYY-MM-DD, done?: bool}]; minutes?: text",
        create,
    ),
    CommandSpec::mutate(
        "meeting.update",
        "Edit meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids; then any meeting.create field; present?: [person ids who attended] (lists replace the old ones)",
        update,
    ),
    CommandSpec::mutate(
        "meeting.issue",
        "Issue minutes",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids (notifies the attendees and the people with actions)",
        issue,
    ),
    CommandSpec::mutate("meeting.reopen", "Reopen minutes", Tool::Meetings, Access::Project(Level::Admin), "project, id: ids", reopen),
    CommandSpec::mutate(
        "meeting.set_action",
        "Mark action done",
        Tool::Meetings,
        Access::Project(Level::Read),
        "project, id: ids; index: action number (from 0); done: bool (whoever the action is assigned to, the author or an admin)",
        set_action,
    ),
    CommandSpec::mutate(
        "meeting.attach",
        "Attach file to meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        attach,
    ),
    CommandSpec::mutate(
        "meeting.detach",
        "Remove file from meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "meeting.delete",
        "Delete meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids (the author before the minutes are issued, or an admin)",
        delete,
    ),
    CommandSpec::mutate("meeting.restore", "Restore meeting", Tool::Meetings, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "meeting.list",
        "List meetings",
        Tool::Meetings,
        Access::Project(Level::Read),
        "project: project id; status?: scheduled|issued; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("meeting.get", "Show meeting", Tool::Meetings, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_ROWS: usize = 200;
const MAX_ATTACHMENTS: usize = 50;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Meeting> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Meeting> = p.id("id")?;
    let m = in_project(&ctx.state.meetings, project, id, "meeting")?;
    if m.deleted.is_some() {
        return Err(EngineError::not_found(format!("meeting {id} not found in this project")));
    }
    Ok(m.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Meetings) == Level::Admin
}

fn is_author(ctx: &Ctx<'_>, m: &Meeting) -> bool {
    ctx.person().is_some() && m.created.by == ctx.person()
}

/// Author until the minutes are issued; admins always.
pub fn can_edit(ctx: &Ctx<'_>, m: &Meeting) -> bool {
    is_admin(ctx, m.project) || (m.status == MeetingStatus::Scheduled && is_author(ctx, m))
}

fn rows<'a>(p: &Params<'a>, key: &str) -> Result<Vec<Params<'a>>> {
    let items = p.items(key)?;
    if items.len() > MAX_ROWS {
        return Err(p.bad(key, format!("holds more than {MAX_ROWS} rows")));
    }
    Ok(items)
}

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, m: &mut Meeting) -> Result<()> {
    if p.present("title") {
        m.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("date") {
        m.date = p.req_date("date")?;
    }
    if p.present("category") {
        m.category = p.line_or_empty("category", 100)?;
    }
    if p.present("time") {
        m.time = p.line_or_empty("time", 50)?;
    }
    if p.present("location") {
        m.location = p.line_or_empty("location", text::SHORT)?;
    }
    if p.present("minutes") {
        m.minutes = p.text_or_empty("minutes", text::LONG)?;
    }
    if p.present("invitees") {
        let mut ids = p.ids::<Person>("invitees")?;
        ids.sort();
        ids.dedup();
        if ids.len() > MAX_ROWS {
            return Err(p.bad("invitees", format!("lists more than {MAX_ROWS} people")));
        }
        for id in &ids {
            ctx.check_person(*id)?;
        }
        // Keep attendance already recorded for people who stay invited.
        m.attendees =
            ids.into_iter().map(|person| Attendee { person, present: m.attendees.iter().any(|a| a.person == person && a.present) }).collect();
    }
    if p.present("present") {
        let came = p.ids::<Person>("present")?;
        for a in &mut m.attendees {
            a.present = came.contains(&a.person);
        }
    }
    if p.present("agenda") {
        let mut out = Vec::new();
        for r in rows(p, "agenda")? {
            let title = r.line_or_empty("title", text::SHORT)?;
            let notes = r.text_or_empty("notes", text::LONG)?;
            if title.is_empty() && notes.is_empty() {
                continue;
            }
            if title.is_empty() {
                return Err(r.bad("title", "is required for an agenda item"));
            }
            out.push(AgendaItem { title, notes });
        }
        m.agenda = out;
    }
    if p.present("actions") {
        let mut out = Vec::new();
        for r in rows(p, "actions")? {
            let description = r.line_or_empty("description", text::SHORT)?;
            if description.is_empty() {
                continue;
            }
            let assignee: Option<Id<Person>> = r.opt_id("assignee")?;
            if let Some(a) = assignee {
                ctx.check_person(a)?;
            }
            let due: Option<Date> = r.date("due")?;
            out.push(MeetingAction { description, assignee, due, done: r.flag_or("done", false)? });
        }
        m.actions = out;
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.meetings.iter().filter(|m| m.project == project).map(|m| m.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut m = Meeting {
        id: ctx.state.meetings.peek_next(),
        project,
        number,
        title: String::new(),
        category: String::new(),
        date: p.req_date("date")?,
        time: String::new(),
        location: String::new(),
        status: MeetingStatus::Scheduled,
        attendees: Vec::new(),
        agenda: Vec::new(),
        actions: Vec::new(),
        minutes: String::new(),
        attachments: Vec::new(),
        issued: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut m)?;
    if m.title.is_empty() {
        return Err(p.bad("title", "is required"));
    }
    let id = ctx.state.meetings.alloc();
    m.id = id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    if !can_edit(ctx, &m) {
        return Err(EngineError::forbidden(format!("you can't edit {}", m.label())));
    }
    apply(ctx, p, &mut m)?;
    m.updated = ctx.stamp();
    let id = m.id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id }))
}

fn link(m: &Meeting) -> String {
    format!("/projects/{}/meetings/{}", m.project, m.id)
}

fn issue(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    if !can_edit(ctx, &m) {
        return Err(EngineError::forbidden(format!("you can't issue the minutes of {}", m.label())));
    }
    if m.status == MeetingStatus::Issued {
        return Err(EngineError::conflict("the minutes were already issued"));
    }
    m.status = MeetingStatus::Issued;
    m.issued = Some(ctx.stamp());
    m.updated = ctx.stamp();
    let mut audience: Vec<Id<Person>> = m.attendees.iter().map(|a| a.person).collect();
    let (project, l) = (m.project, link(&m));
    let text = format!("Minutes of {} are out: {}", m.label(), m.title);
    let assigned: Vec<Id<Person>> = m.actions.iter().filter(|a| !a.done).filter_map(|a| a.assignee).collect();
    let id = m.id;
    ctx.state.meetings.put(m);
    audience.retain(|x| !assigned.contains(x));
    notify(ctx, &audience, project, &text, &l);
    notify(ctx, &assigned, project, &format!("{text} (you have an action)"), &l);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    if m.status != MeetingStatus::Issued {
        return Err(EngineError::conflict("the minutes are not issued"));
    }
    m.status = MeetingStatus::Scheduled;
    m.issued = None;
    m.updated = ctx.stamp();
    let id = m.id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id }))
}

fn set_action(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    let index = usize::try_from(p.int("index", 0..=1_000)?.ok_or_else(|| p.bad("index", "is required"))?).unwrap_or(usize::MAX);
    let done = p.flag("done")?.ok_or_else(|| p.bad("done", "is required"))?;
    let me = ctx.person();
    let allowed = is_admin(ctx, m.project) || is_author(ctx, &m) || m.actions.get(index).is_some_and(|a| a.assignee.is_some() && a.assignee == me);
    let Some(action) = m.actions.get_mut(index) else {
        return Err(EngineError::not_found(format!("{} has no action {index}", m.label())));
    };
    if !allowed {
        return Err(EngineError::forbidden("only the person it is assigned to, the author or an admin can change an action"));
    }
    action.done = done;
    m.updated = ctx.stamp();
    let id = m.id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id }))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    if !can_edit(ctx, &m) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", m.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|x| !x.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if m.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": m.id }));
    }
    if m.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", m.label())));
    }
    m.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    m.updated = ctx.stamp();
    let id = m.id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut m = load(ctx, p)?;
    if !can_edit(ctx, &m) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", m.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = m.attachments.len();
    m.attachments.retain(|a| a.sha256 != sha256);
    if m.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this meeting"));
    }
    m.updated = ctx.stamp();
    let id = m.id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let m = load(ctx, p)?;
    if !can_edit(ctx, &m) {
        return Err(EngineError::forbidden("only the author before the minutes are issued, or an admin, can delete a meeting"));
    }
    let stamp = ctx.stamp();
    ctx.state.meetings.update(m.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": m.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Meeting> = p.id("id")?;
    let m = in_project(&ctx.state.meetings, project, id, "meeting")?;
    if m.deleted.is_none() {
        return Err(EngineError::conflict("the meeting is not in the recycle bin"));
    }
    ctx.state.meetings.update(id, |m| m.deleted = None);
    Ok(json!({ "id": id }))
}

/// The meetings of a project, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&Meeting> {
    let mut v: Vec<&Meeting> = state.meetings.iter().filter(|m| m.project == project && m.deleted.is_none()).collect();
    v.sort_by_key(|m| std::cmp::Reverse((m.date, m.number)));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Meetings) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Meeting> = v.state.meetings.iter().filter(|m| m.project == project && m.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<MeetingStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&Meeting> = visible(v.state, project)
        .into_iter()
        .filter(|m| status.is_none_or(|s| m.status == s))
        .filter(|m| {
            q.as_ref().is_none_or(|q| {
                m.title.to_lowercase().contains(q.as_str())
                    || m.category.to_lowercase().contains(q.as_str())
                    || m.agenda.iter().any(|a| a.title.to_lowercase().contains(q.as_str()))
                    || m.actions.iter().any(|a| a.description.to_lowercase().contains(q.as_str()))
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Meeting> = p.id("id")?;
    let m = in_project(&v.state.meetings, project, id, "meeting")?;
    if m.deleted.is_some() {
        return Err(EngineError::not_found(format!("meeting {id} not found in this project")));
    }
    json(m)
}
