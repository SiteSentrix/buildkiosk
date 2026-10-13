//! Meetings: plan with an agenda and invitees, record who came, what was said and who agreed to
//! do what, then issue the minutes. Issuing tells the attendees and the people with actions.
//!
//! Who may do what: the author (or a Meetings admin) edits a meeting until its minutes are issued;
//! an admin changes issued minutes. Anyone given an action may tick it off, at any time.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::meeting::{AgendaItem, Attendee, Meeting, MeetingAction, MeetingStatus, MeetingTemplate};
use buildkiosk_model::project::Project;
use buildkiosk_model::task::Task;
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
        "project: project id; title: text; date: YYYY-MM-DD; category?: text; time?: text; location?: text; template?: meeting template id (fills in the type, time, place, invitees and agenda topics you leave blank); invitees?: [person ids]; agenda?: [{title, notes?}]; actions?: [{description, assignee?: person id, due?: YYYY-MM-DD, done?: bool}]; minutes?: text",
        create,
    ),
    CommandSpec::mutate(
        "meeting_template.save",
        "Save meeting template",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project: project id; name: text (a template of the same name is replaced); from_meeting?: meeting id to copy the type, time, place, invitees and agenda topics from; otherwise category?, time?, location?: text, invitees?: [person ids], agenda?: topics, one per line",
        template_save,
    ),
    CommandSpec::mutate(
        "meeting_template.delete",
        "Delete meeting template",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids",
        template_delete,
    ),
    CommandSpec::query(
        "meeting_template.list",
        "List meeting templates",
        Tool::Meetings,
        Access::Project(Level::Read),
        "project: project id",
        template_list,
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
    CommandSpec::mutate(
        "meeting.repeat",
        "Repeat meeting",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids (the meeting to repeat); every: weekly|biweekly|monthly; count: how many more to schedule (1 to 26). Each has the same title, type, time, place, invitees and agenda topics (without notes), on the following dates",
        repeat,
    ),
    CommandSpec::mutate(
        "meeting.carry_actions",
        "Carry open actions forward",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids (the meeting that takes them); from?: meeting id (default: the latest earlier meeting of the same type, else the latest earlier one). The open actions are added here and marked as carried forward there",
        carry_actions,
    ),
    CommandSpec::mutate(
        "meeting.action_to_task",
        "Make a task from an action",
        Tool::Meetings,
        Access::Project(Level::Standard),
        "project, id: ids; index: action number (from 0). Creates a task for the same work, assignee and due date that names this meeting (needs Standard access to Tasks); closing the task marks the action done",
        action_to_task,
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
            // A form posts only what it shows, so what it can't (carried, task, source) stays with the row that
            // had the same place and words.
            let kept = m.actions.get(out.len()).filter(|old| old.description == description);
            out.push(MeetingAction {
                description,
                assignee,
                due,
                done: r.flag_or("done", false)?,
                carried: kept.is_some_and(|k| k.carried),
                task: kept.and_then(|k| k.task),
                from_meeting: kept.and_then(|k| k.from_meeting),
            });
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
    if let Some(t) = p.opt_id::<MeetingTemplate>("template")? {
        let t = in_project(&ctx.state.meeting_templates, project, t, "meeting template")?;
        if t.deleted.is_some() {
            return Err(p.bad("template", format!("meeting template {} not found in this project", t.id)));
        }
        // What the form left blank comes from the template.
        if m.category.is_empty() {
            m.category = t.category.clone();
        }
        if m.time.is_empty() {
            m.time = t.time.clone();
        }
        if m.location.is_empty() {
            m.location = t.location.clone();
        }
        if m.attendees.is_empty() {
            m.attendees = t.invitees.iter().map(|person| Attendee { person: *person, present: false }).collect();
        }
        if m.agenda.is_empty() {
            m.agenda = t.agenda.iter().map(|title| AgendaItem { title: title.clone(), notes: String::new() }).collect();
        }
    }
    if m.title.is_empty() {
        return Err(p.bad("title", "is required"));
    }
    let id = ctx.state.meetings.alloc();
    m.id = id;
    ctx.state.meetings.put(m);
    Ok(json!({ "id": id, "number": number }))
}

fn template_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", text::SHORT)?;
    let (category, time, location, invitees, agenda): (String, String, String, Vec<Id<Person>>, Vec<String>) =
        match p.opt_id::<Meeting>("from_meeting")? {
            Some(mid) => {
                let m = in_project(&ctx.state.meetings, project, mid, "meeting")?;
                if m.deleted.is_some() {
                    return Err(p.bad("from_meeting", format!("meeting {mid} not found in this project")));
                }
                (
                    m.category.clone(),
                    m.time.clone(),
                    m.location.clone(),
                    m.attendees.iter().map(|a| a.person).collect(),
                    m.agenda.iter().map(|a| a.title.clone()).collect(),
                )
            }
            None => {
                let mut ids = p.ids::<Person>("invitees")?;
                ids.sort();
                ids.dedup();
                if ids.len() > MAX_ROWS {
                    return Err(p.bad("invitees", format!("lists more than {MAX_ROWS} people")));
                }
                for id in &ids {
                    ctx.check_person(*id)?;
                }
                let topics: Vec<String> = p
                    .text_or_empty("agenda", text::LONG)?
                    .lines()
                    .map(|l| l.trim().trim_start_matches(['-', '*']).trim().to_string())
                    .filter(|l| !l.is_empty())
                    .take(MAX_ROWS)
                    .collect();
                (p.line_or_empty("category", 100)?, p.line_or_empty("time", 50)?, p.line_or_empty("location", text::SHORT)?, ids, topics)
            }
        };
    let existing =
        ctx.state.meeting_templates.iter().find(|t| t.project == project && t.deleted.is_none() && t.name.eq_ignore_ascii_case(&name)).map(|t| t.id);
    if existing.is_none() && ctx.state.meeting_templates.iter().filter(|t| t.project == project && t.deleted.is_none()).count() >= 100 {
        return Err(EngineError::conflict("a project keeps at most 100 meeting templates"));
    }
    let stamp = ctx.stamp();
    let id = match existing {
        Some(id) => {
            ctx.state
                .meeting_templates
                .update(id, |t| (t.category, t.time, t.location, t.invitees, t.agenda) = (category, time, location, invitees, agenda));
            id
        }
        None => {
            let id = ctx.state.meeting_templates.alloc();
            ctx.state.meeting_templates.put(MeetingTemplate {
                id,
                project,
                name,
                category,
                time,
                location,
                invitees,
                agenda,
                created: stamp,
                deleted: None,
            });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn template_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<MeetingTemplate> = p.id("id")?;
    if in_project(&ctx.state.meeting_templates, project, id, "meeting template")?.deleted.is_some() {
        return Err(EngineError::not_found(format!("meeting template {id} not found in this project")));
    }
    let stamp = ctx.stamp();
    ctx.state.meeting_templates.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// The project's meeting templates, by name.
pub fn templates(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&MeetingTemplate> {
    let mut v: Vec<&MeetingTemplate> = state.meeting_templates.iter().filter(|t| t.project == project && t.deleted.is_none()).collect();
    v.sort_by_key(|t| t.name.to_lowercase());
    v
}

fn template_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    json(&templates(v.state, p.id("project")?))
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
    let assigned: Vec<Id<Person>> = m.actions.iter().filter(|a| a.is_open()).filter_map(|a| a.assignee).collect();
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

const MAX_SERIES: i64 = 26;

fn repeat(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let source = load(ctx, p)?;
    let every = p.line("every", 20)?.ok_or_else(|| p.bad("every", "is required"))?;
    let count = p.int("count", 1..=MAX_SERIES)?.ok_or_else(|| p.bad("count", "is required"))?;
    let step = |k: i64| -> Option<Date> {
        match every.to_ascii_lowercase().as_str() {
            "weekly" => source.date.add_days(7 * k),
            "biweekly" => source.date.add_days(14 * k),
            "monthly" => source.date.add_months(i32::try_from(k).ok()?),
            _ => None,
        }
    };
    if step(1).is_none() {
        return Err(p.bad("every", "must be weekly, biweekly or monthly (and stay inside the supported dates)"));
    }
    let mut made = Vec::new();
    for k in 1..=count {
        let date = step(k).ok_or_else(|| p.bad("count", "runs past the supported dates"))?;
        let number = ctx.state.meetings.iter().filter(|m| m.project == source.project).map(|m| m.number).max().unwrap_or(0).saturating_add(1);
        let stamp = ctx.stamp();
        let id = ctx.state.meetings.alloc();
        ctx.state.meetings.put(Meeting {
            id,
            project: source.project,
            number,
            title: source.title.clone(),
            category: source.category.clone(),
            date,
            time: source.time.clone(),
            location: source.location.clone(),
            status: MeetingStatus::Scheduled,
            attendees: source.attendees.iter().map(|a| Attendee { person: a.person, present: false }).collect(),
            agenda: source.agenda.iter().map(|a| AgendaItem { title: a.title.clone(), notes: String::new() }).collect(),
            actions: Vec::new(),
            minutes: String::new(),
            attachments: Vec::new(),
            issued: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        made.push(id);
    }
    Ok(json!({ "ids": made }))
}

fn carry_actions(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut target = load(ctx, p)?;
    if !can_edit(ctx, &target) {
        return Err(EngineError::forbidden(format!("you can't change {}", target.label())));
    }
    let from: Id<Meeting> = match p.opt_id::<Meeting>("from")? {
        Some(f) => f,
        None => {
            // The latest earlier meeting of the same type, else the latest earlier one at all.
            let earlier = |m: &&Meeting| {
                m.project == target.project && m.deleted.is_none() && m.id != target.id && (m.date, m.number) < (target.date, target.number)
            };
            let mut all: Vec<&Meeting> = ctx.state.meetings.iter().filter(earlier).collect();
            all.sort_by_key(|m| (m.date, m.number));
            all.iter()
                .rev()
                .find(|m| m.category == target.category)
                .or_else(|| all.last())
                .map(|m| m.id)
                .ok_or_else(|| EngineError::not_found("there is no earlier meeting to carry actions from"))?
        }
    };
    if from == target.id {
        return Err(p.bad("from", "is this meeting"));
    }
    let mut source = in_project(&ctx.state.meetings, target.project, from, "meeting")?.clone();
    if source.deleted.is_some() {
        return Err(EngineError::not_found(format!("meeting {from} not found in this project")));
    }
    let mut carried = 0usize;
    for a in source.actions.iter_mut().filter(|a| a.is_open()) {
        if target.actions.len() >= MAX_ROWS {
            return Err(EngineError::conflict(format!("{} would have more than {MAX_ROWS} actions", target.label())));
        }
        target.actions.push(MeetingAction { done: false, carried: false, from_meeting: Some(from), ..a.clone() });
        a.carried = true;
        carried += 1;
    }
    if carried > 0 {
        let stamp = ctx.stamp();
        source.updated = stamp;
        target.updated = stamp;
        ctx.state.meetings.put(source);
        ctx.state.meetings.put(target);
    }
    Ok(json!({ "carried": carried }))
}

fn action_to_task(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    use crate::workitem::WorkItem;
    let mut m = load(ctx, p)?;
    let index = usize::try_from(p.int("index", 0..=1_000)?.ok_or_else(|| p.bad("index", "is required"))?).unwrap_or(usize::MAX);
    let me = ctx.person();
    let allowed = is_admin(ctx, m.project) || is_author(ctx, &m) || m.actions.get(index).is_some_and(|a| a.assignee.is_some() && a.assignee == me);
    let Some(action) = m.actions.get(index).cloned() else {
        return Err(EngineError::not_found(format!("{} has no action {index}", m.label())));
    };
    if !allowed {
        return Err(EngineError::forbidden("only the person it is assigned to, the author or an admin can make a task from an action"));
    }
    if ctx.level(m.project, Tool::Tasks) < Level::Standard {
        return Err(EngineError::forbidden("making a task needs Standard access to Tasks"));
    }
    if action.task.is_some() {
        return Err(EngineError::conflict("this action already has a task"));
    }
    if action.done {
        return Err(EngineError::conflict("this action is done"));
    }
    let mut task = <Task as WorkItem>::blank(ctx, m.project);
    task.title = action.description.clone();
    task.kind = "Meeting action".into();
    task.assignees = action.assignee.into_iter().collect();
    task.due = action.due;
    task.meeting = Some(m.id);
    task.recompute_ball_in_court();
    let assignees = task.assignees.clone();
    let task_id = crate::workitem::insert(ctx, task);
    if let Some(a) = m.actions.get_mut(index) {
        a.task = Some(task_id);
    }
    m.updated = ctx.stamp();
    let (project, label) = (m.project, m.label());
    ctx.state.meetings.put(m);
    notify(
        ctx,
        &assignees,
        project,
        &format!("A task was made from an action in {label}: {}", action.description),
        &format!("/projects/{project}/tasks/{task_id}"),
    );
    Ok(json!({ "task": task_id }))
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
