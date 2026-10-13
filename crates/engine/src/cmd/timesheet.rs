//! Timesheets: hours by person, day and (optionally) cost code, submitted for approval.
//!
//! People log and submit their own hours. A Timesheets admin can log hours for anyone, approves
//! or sends back submitted entries, and sees everyone's. Everyone else sees only their own.
//! A day's hours for one person on one project total at most 24.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::daily_log::parse_hours;
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::timesheet::{TimeEntry, TimeStatus};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::usable_code;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "timesheet.add",
        "Log hours",
        Tool::Timesheets,
        Access::Project(Level::Standard),
        "project: project id; date: YYYY-MM-DD; hours: 0 to 24 (one decimal); cost_code?: cost code id; description?: text; person?: person id (admins only; default yourself)",
        add,
    ),
    CommandSpec::mutate(
        "timesheet.update",
        "Edit hours",
        Tool::Timesheets,
        Access::Project(Level::Standard),
        "project, id: ids; date?, hours?, cost_code?, description? (own drafts, or an admin until approved)",
        update,
    ),
    CommandSpec::mutate(
        "timesheet.delete",
        "Delete hours",
        Tool::Timesheets,
        Access::Project(Level::Standard),
        "project, id: ids (own drafts, or an admin until approved)",
        delete,
    ),
    CommandSpec::mutate(
        "timesheet.submit",
        "Submit hours",
        Tool::Timesheets,
        Access::Project(Level::Standard),
        "project: project id; from: YYYY-MM-DD; to?: YYYY-MM-DD (default: six days after from) (submits your draft entries in that range; an admin may add person: person id)",
        submit,
    ),
    CommandSpec::mutate(
        "timesheet.approve",
        "Approve hours",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; ids: [entry ids] or person: person id with from, to?: dates (approves the submitted entries)",
        approve,
    ),
    CommandSpec::mutate(
        "timesheet.return",
        "Send hours back",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; ids: [entry ids] or person with from, to?; note?: text (submitted entries go back to draft)",
        return_back,
    ),
    CommandSpec::query(
        "timesheet.list",
        "List hours",
        Tool::Timesheets,
        Access::Project(Level::Standard),
        "project: project id; person?: person id (admins; default yourself); from?, to?: dates; status?: draft|submitted|approved; deleted?: bool (admins)",
        list,
    ),
];

const MAX_BATCH: usize = 500;

fn is_admin(ctx: &impl Read, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Timesheets) == Level::Admin
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<TimeEntry> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<TimeEntry> = p.id("id")?;
    let e = in_project(&ctx.state.time_entries, project, id, "time entry")?;
    if e.deleted.is_some() || !(is_admin(ctx, project) || ctx.person() == Some(e.person)) {
        return Err(EngineError::not_found(format!("time entry {id} not found in this project")));
    }
    Ok(e.clone())
}

/// Owner while it is a draft, an admin until approved.
fn can_edit(ctx: &Ctx<'_>, e: &TimeEntry) -> bool {
    match e.status {
        TimeStatus::Approved => false,
        TimeStatus::Draft => is_admin(ctx, e.project) || ctx.person() == Some(e.person),
        TimeStatus::Submitted => is_admin(ctx, e.project),
    }
}

fn day_total(ctx: &Ctx<'_>, project: Id<Project>, person: Id<Person>, date: Date, except: Option<Id<TimeEntry>>) -> u32 {
    ctx.state
        .time_entries
        .iter()
        .filter(|e| e.project == project && e.person == person && e.date == date && e.deleted.is_none() && Some(e.id) != except)
        .map(|e| e.tenths)
        .sum()
}

fn hours(p: &Params<'_>) -> Result<u32> {
    let raw = p.line("hours", 10)?.ok_or_else(|| p.bad("hours", "is required"))?;
    parse_hours(&raw).filter(|t| *t > 0).ok_or_else(|| p.bad("hours", format!("`{raw}` is not a number of hours (above 0 to 24, one decimal)")))
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = match p.opt_id::<Person>("person")? {
        Some(x) if Some(x) != ctx.person() => {
            if !is_admin(ctx, project) {
                return Err(EngineError::forbidden("only Timesheets admins can log hours for someone else"));
            }
            ctx.check_person(x)?;
            x
        }
        _ => ctx.person().ok_or_else(|| EngineError::forbidden("hours belong to a person: name one with `person`"))?,
    };
    let date = p.req_date("date")?;
    let tenths = hours(p)?;
    if day_total(ctx, project, person, date, None).saturating_add(tenths) > 240 {
        return Err(EngineError::conflict("that makes more than 24 hours on one day"));
    }
    let cost_code: Option<Id<CostCode>> = p.opt_id("cost_code")?;
    if let Some(c) = cost_code {
        usable_code(ctx.state, project, c)?;
    }
    let id = ctx.state.time_entries.alloc();
    let stamp = ctx.stamp();
    ctx.state.time_entries.put(TimeEntry {
        id,
        project,
        person,
        date,
        tenths,
        cost_code,
        description: p.line_or_empty("description", text::SHORT)?,
        status: TimeStatus::Draft,
        returned_note: String::new(),
        approved: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if !can_edit(ctx, &e) {
        return Err(EngineError::forbidden("these hours can't be changed now"));
    }
    if p.present("date") {
        e.date = p.req_date("date")?;
    }
    if p.present("hours") {
        e.tenths = hours(p)?;
    }
    if p.present("cost_code") {
        let c: Option<Id<CostCode>> = p.opt_id("cost_code")?;
        if let Some(c) = c {
            usable_code(ctx.state, e.project, c)?;
        }
        e.cost_code = c;
    }
    if p.present("description") {
        e.description = p.line_or_empty("description", text::SHORT)?;
    }
    if day_total(ctx, e.project, e.person, e.date, Some(e.id)).saturating_add(e.tenths) > 240 {
        return Err(EngineError::conflict("that makes more than 24 hours on one day"));
    }
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.time_entries.put(e);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e = load(ctx, p)?;
    if !can_edit(ctx, &e) {
        return Err(EngineError::forbidden("these hours can't be deleted now"));
    }
    let stamp = ctx.stamp();
    ctx.state.time_entries.update(e.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": e.id }))
}

/// The entries a batch command names: `ids`, or `person` with a date range.
fn picked(ctx: &Ctx<'_>, p: &Params<'_>, project: Id<Project>, person: Option<Id<Person>>) -> Result<Vec<Id<TimeEntry>>> {
    if p.present("ids") && !p.ids::<TimeEntry>("ids")?.is_empty() {
        let ids = p.ids::<TimeEntry>("ids")?;
        if ids.len() > MAX_BATCH {
            return Err(p.bad("ids", format!("lists more than {MAX_BATCH} entries")));
        }
        for id in &ids {
            in_project(&ctx.state.time_entries, project, *id, "time entry")?;
        }
        return Ok(ids);
    }
    let from = p.req_date("from")?;
    let to = range_end(p, from)?;
    Ok(ctx
        .state
        .time_entries
        .iter()
        .filter(|e| e.project == project && e.deleted.is_none() && person.is_none_or(|x| e.person == x) && e.date >= from && e.date <= to)
        .map(|e| e.id)
        .take(MAX_BATCH)
        .collect())
}

fn range_end(p: &Params<'_>, from: Date) -> Result<Date> {
    let to = match p.date("to")? {
        Some(t) => t,
        None => from.add_days(6).ok_or_else(|| p.bad("from", "is out of range"))?,
    };
    if to < from || from.days_until(to) > 366 {
        return Err(p.bad("to", "must be on or after from, within a year"));
    }
    Ok(to)
}

fn submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = match p.opt_id::<Person>("person")? {
        Some(x) if Some(x) != ctx.person() => {
            if !is_admin(ctx, project) {
                return Err(EngineError::forbidden("only Timesheets admins can submit someone else's hours"));
            }
            x
        }
        _ => ctx.person().ok_or_else(|| EngineError::forbidden("hours belong to a person: name one with `person`"))?,
    };
    let from = p.req_date("from")?;
    let to = range_end(p, from)?;
    let ids: Vec<Id<TimeEntry>> = ctx
        .state
        .time_entries
        .iter()
        .filter(|e| {
            e.project == project && e.person == person && e.deleted.is_none() && e.status == TimeStatus::Draft && e.date >= from && e.date <= to
        })
        .map(|e| e.id)
        .collect();
    if ids.is_empty() {
        return Err(EngineError::conflict("there are no draft hours in that range to submit"));
    }
    let stamp = ctx.stamp();
    for id in &ids {
        ctx.state.time_entries.update(*id, |e| {
            e.status = TimeStatus::Submitted;
            e.returned_note.clear();
            e.updated = stamp;
        });
    }
    Ok(json!({ "submitted": ids.len() }))
}

fn approve(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Option<Id<Person>> = p.opt_id("person")?;
    let ids = picked(ctx, p, project, person)?;
    let stamp = ctx.stamp();
    let mut n = 0;
    for id in ids {
        let ok = ctx.state.time_entries.get(id).is_some_and(|e| e.deleted.is_none() && e.status == TimeStatus::Submitted);
        if ok {
            ctx.state.time_entries.update(id, |e| {
                e.status = TimeStatus::Approved;
                e.approved = Some(stamp);
                e.updated = stamp;
            });
            n += 1;
        }
    }
    if n == 0 {
        return Err(EngineError::conflict("there are no submitted hours to approve"));
    }
    Ok(json!({ "approved": n }))
}

fn return_back(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Option<Id<Person>> = p.opt_id("person")?;
    let note = p.line_or_empty("note", text::SHORT)?;
    let ids = picked(ctx, p, project, person)?;
    let stamp = ctx.stamp();
    let mut n = 0;
    for id in ids {
        let ok = ctx.state.time_entries.get(id).is_some_and(|e| e.deleted.is_none() && e.status == TimeStatus::Submitted);
        if ok {
            let note = note.clone();
            ctx.state.time_entries.update(id, |e| {
                e.status = TimeStatus::Draft;
                e.returned_note = note;
                e.updated = stamp;
            });
            n += 1;
        }
    }
    if n == 0 {
        return Err(EngineError::conflict("there are no submitted hours to send back"));
    }
    Ok(json!({ "returned": n }))
}

/// The entries the actor may see: everyone's for an admin, their own otherwise.
pub fn visible(state: &buildkiosk_model::CompanyState, actor: Actor, project: Id<Project>) -> Vec<&TimeEntry> {
    let admin = view_level(state, actor, project) == Level::Admin;
    let me = actor.person();
    let mut v: Vec<&TimeEntry> =
        state.time_entries.iter().filter(|e| e.project == project && e.deleted.is_none() && (admin || Some(e.person) == me)).collect();
    v.sort_by_key(|e| (std::cmp::Reverse(e.date), e.person, e.id));
    v
}

fn view_level(state: &buildkiosk_model::CompanyState, actor: Actor, project: Id<Project>) -> Level {
    crate::view::level(state, actor, project, Tool::Timesheets)
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let admin = view_level(v.state, v.actor, project) == Level::Admin;
    if p.flag_or("deleted", false)? {
        if !admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&TimeEntry> = v.state.time_entries.iter().filter(|e| e.project == project && e.deleted.is_some()).collect();
        return json(&bin);
    }
    let person: Option<Id<Person>> = p.opt_id("person")?;
    if person.is_some() && person != v.person() && !admin {
        return Err(EngineError::forbidden("only Timesheets admins can see other people's hours"));
    }
    let (from, to) = (p.date("from")?, p.date("to")?);
    let status = p.choice::<TimeStatus>("status")?;
    let list: Vec<&TimeEntry> = visible(v.state, v.actor, project)
        .into_iter()
        .filter(|e| person.is_none_or(|x| e.person == x))
        .filter(|e| from.is_none_or(|f| e.date >= f) && to.is_none_or(|t| e.date <= t))
        .filter(|e| status.is_none_or(|s| e.status == s))
        .collect();
    json(&list)
}
