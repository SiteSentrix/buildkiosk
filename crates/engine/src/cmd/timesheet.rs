//! Timesheets: hours by person, day and (optionally) cost code, submitted for approval.
//!
//! People log and submit their own hours. A Timesheets admin can log hours for anyone, approves
//! or sends back submitted entries, and sees everyone's. Everyone else sees only their own.
//! A day's hours for one person on one project total at most 24.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id, Money, Percent, text};
use buildkiosk_model::daily_log::parse_hours;
use buildkiosk_model::directory::Person;
use buildkiosk_model::finance::DirectCost;
use buildkiosk_model::project::CostType;
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::timesheet::{OvertimeRules, PayRate, TimeEntry, TimeStatus};
use buildkiosk_model::{CompanyState, Level, Tool};
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
        "project: project id; date: YYYY-MM-DD; hours: 0 to 24 (one decimal); cost_code?: cost code id; description?: text; person?: person id (admins only; default yourself); people?: [person ids] (admins: the same hours for a whole crew, one entry each)",
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
    CommandSpec::mutate(
        "timesheet.set_rate",
        "Set pay rate",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; person: person id; hourly: amount per hour (above 0, up to 10,000); from?: YYYY-MM-DD the rate applies from (default: today; a rate already set for that day is replaced)",
        set_rate,
    ),
    CommandSpec::mutate("timesheet.delete_rate", "Remove pay rate", Tool::Timesheets, Access::Project(Level::Admin), "project, id: ids", delete_rate),
    CommandSpec::query(
        "timesheet.rates",
        "List pay rates",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; person?: person id",
        rates,
    ),
    CommandSpec::mutate(
        "timesheet.set_rules",
        "Set overtime rules",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; daily_hours?: hours per person per day after which time is overtime (0 for none); weekly_hours?: the same per Monday-to-Sunday week; multiplier?: overtime pay as a percentage of the rate (default 150)",
        set_rules,
    ),
    CommandSpec::query(
        "timesheet.cost_report",
        "Labour cost report",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; from?, to?: dates; include?: approved (default) | all (also submitted and draft hours)",
        cost_report,
    ),
    CommandSpec::mutate(
        "timesheet.post_costs",
        "Post labour to direct costs",
        Tool::Timesheets,
        Access::Project(Level::Admin),
        "project: project id; from: YYYY-MM-DD; to?: YYYY-MM-DD (default: six days after from); default_cost_code?: cost code id for hours with no code (approved hours not posted before become one labour direct cost per cost code; every person needs a pay rate)",
        post_costs,
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
const MAX_CREW: usize = 100;
const MAX_RATE_CENTS: i64 = 1_000_000;

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
    let crew: Vec<Id<Person>> = if p.present("people") { p.ids("people")? } else { Vec::new() };
    let people: Vec<Id<Person>> = if !crew.is_empty() {
        if !is_admin(ctx, project) {
            return Err(EngineError::forbidden("only Timesheets admins can log hours for a crew"));
        }
        if crew.len() > MAX_CREW {
            return Err(p.bad("people", format!("names more than {MAX_CREW} people")));
        }
        for x in &crew {
            ctx.check_person(*x)?;
        }
        let mut c = crew;
        c.sort();
        c.dedup();
        c
    } else {
        vec![match p.opt_id::<Person>("person")? {
            Some(x) if Some(x) != ctx.person() => {
                if !is_admin(ctx, project) {
                    return Err(EngineError::forbidden("only Timesheets admins can log hours for someone else"));
                }
                ctx.check_person(x)?;
                x
            }
            _ => ctx.person().ok_or_else(|| EngineError::forbidden("hours belong to a person: name one with `person`"))?,
        }]
    };
    let date = p.req_date("date")?;
    let tenths = hours(p)?;
    for person in &people {
        if day_total(ctx, project, *person, date, None).saturating_add(tenths) > 240 {
            return Err(EngineError::conflict(format!(
                "that makes more than 24 hours on one day for {}",
                crate::view::person_name(ctx.state, *person)
            )));
        }
    }
    let cost_code: Option<Id<CostCode>> = p.opt_id("cost_code")?;
    if let Some(c) = cost_code {
        usable_code(ctx.state, project, c)?;
    }
    let description = p.line_or_empty("description", text::SHORT)?;
    let stamp = ctx.stamp();
    let mut ids = Vec::new();
    for person in people {
        let id = ctx.state.time_entries.alloc();
        ctx.state.time_entries.put(TimeEntry {
            id,
            project,
            person,
            date,
            tenths,
            cost_code,
            description: description.clone(),
            status: TimeStatus::Draft,
            returned_note: String::new(),
            submitted: None,
            approved: None,
            posted: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        ids.push(id);
    }
    Ok(json!({ "id": ids.first(), "ids": ids }))
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
            e.submitted = Some(stamp);
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
                e.submitted = None;
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

// -- pay and overtime

fn set_rate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = p.id("person")?;
    ctx.check_person(person)?;
    let hourly = p.money("hourly")?.ok_or_else(|| p.bad("hourly", "is required"))?;
    if hourly.cents() <= 0 || hourly.cents() > MAX_RATE_CENTS {
        return Err(p.bad("hourly", "must be above 0 and at most 10,000 an hour"));
    }
    let from = p.date("from")?.unwrap_or_else(|| ctx.today());
    let same: Vec<Id<PayRate>> = ctx
        .state
        .pay_rates
        .iter()
        .filter(|r| r.project == project && r.person == person && r.from == from && r.deleted.is_none())
        .map(|r| r.id)
        .collect();
    if ctx.state.pay_rates.iter().filter(|r| r.project == project && r.deleted.is_none()).count() >= 5_000 && same.is_empty() {
        return Err(EngineError::conflict("a project keeps at most 5,000 pay rates"));
    }
    let stamp = ctx.stamp();
    for id in same {
        ctx.state.pay_rates.update(id, |r| r.deleted = Some(stamp));
    }
    let id = ctx.state.pay_rates.alloc();
    ctx.state.pay_rates.put(PayRate { id, project, person, from, hourly, created: stamp, deleted: None });
    Ok(json!({ "id": id }))
}

fn delete_rate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<PayRate> = p.id("id")?;
    if in_project(&ctx.state.pay_rates, project, id, "pay rate")?.deleted.is_some() {
        return Err(EngineError::not_found(format!("pay rate {id} not found in this project")));
    }
    let stamp = ctx.stamp();
    ctx.state.pay_rates.update(id, |r| r.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// The project's current pay rates, by person then date.
pub fn rate_list(state: &CompanyState, project: Id<Project>) -> Vec<&PayRate> {
    let mut v: Vec<&PayRate> = state.pay_rates.iter().filter(|r| r.project == project && r.deleted.is_none()).collect();
    v.sort_by_key(|r| (r.person, r.from, r.id));
    v
}

fn rates(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Option<Id<Person>> = p.opt_id("person")?;
    let list: Vec<&PayRate> = rate_list(v.state, project).into_iter().filter(|r| person.is_none_or(|x| r.person == x)).collect();
    json(&list)
}

fn set_rules(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut rules = ctx.state.projects.get(project).map(|x| x.overtime).unwrap_or_default();
    let tenths = |key: &str| -> Result<Option<u32>> {
        match p.line(key, 10)? {
            None => Ok(None),
            Some(raw) if raw.trim().is_empty() || raw.trim() == "0" => Ok(Some(0)),
            Some(raw) => parse_hours(&raw).map(Some).ok_or_else(|| p.bad(key, format!("`{raw}` is not a number of hours (0 to 24, one decimal)"))),
        }
    };
    if let Some(t) = tenths("daily_hours")? {
        rules.daily_tenths = t;
    }
    if p.present("weekly_hours") {
        let raw = p.line_or_empty("weekly_hours", 10)?;
        rules.weekly_tenths = if raw.trim().is_empty() || raw.trim() == "0" {
            0
        } else {
            let f: f64 = raw.trim().parse().map_err(|_| p.bad("weekly_hours", format!("`{raw}` is not a number of hours")))?;
            if !f.is_finite() || !(0.0..=168.0).contains(&f) {
                return Err(p.bad("weekly_hours", "must be between 0 and 168"));
            }
            (f * 10.0).round() as u32
        };
    }
    if p.present("multiplier") {
        let m = p.percent("multiplier")?.ok_or_else(|| p.bad("multiplier", "is required"))?;
        if m < Percent::HUNDRED || m.hundredths() > 50_000 {
            return Err(p.bad("multiplier", "must be between 100% and 500%"));
        }
        rules.multiplier = Some(m);
    }
    ctx.state.projects.update(project, |x| x.overtime = rules);
    Ok(json!({ "daily_tenths": rules.daily_tenths, "weekly_tenths": rules.weekly_tenths }))
}

/// One entry split into regular and overtime hours and priced.
#[derive(Clone, Debug, PartialEq)]
pub struct Labour {
    pub entry: Id<TimeEntry>,
    pub person: Id<Person>,
    pub date: Date,
    pub cost_code: Option<Id<CostCode>>,
    pub status: TimeStatus,
    pub posted: bool,
    pub regular: u32,
    pub overtime: u32,
    /// The hourly rate in force on the day; none when the person has no rate yet (the cost is then zero).
    pub rate: Option<Money>,
    pub cost: Money,
}

fn tenths_of(rate: Money, tenths: u32) -> Money {
    // The rate is capped at 10,000 an hour and a day's entries at 240 tenths, so this cannot overflow;
    // zero is the harmless answer if it ever did.
    rate.checked_mul_int(i64::from(tenths)).and_then(|m| m.checked_div_int(10)).unwrap_or(Money::ZERO)
}

/// Every non-deleted entry of the project split into regular and overtime hours under the project's rules and
/// priced at the rate in force on its day. Overtime counts everything a person worked, whatever the status.
pub fn labour(state: &CompanyState, project: Id<Project>) -> Vec<Labour> {
    let rules: OvertimeRules = state.projects.get(project).map(|x| x.overtime).unwrap_or_default();
    let mult = rules.pay_multiplier();
    let rates = rate_list(state, project);
    let mut entries: Vec<&TimeEntry> = state.time_entries.iter().filter(|e| e.project == project && e.deleted.is_none()).collect();
    entries.sort_by_key(|e| (e.person, e.date, e.id));
    let mut out = Vec::with_capacity(entries.len());
    let mut key: Option<(Id<Person>, Date)> = None;
    let (mut day_before, mut week_regular, mut week_start, mut last_day) = (0u32, 0u32, None::<Date>, None::<Date>);
    for e in entries {
        let monday = e.date.add_days(-(e.date.weekday() as i64)).unwrap_or(e.date);
        if key.map(|k| k.0) != Some(e.person) || week_start != Some(monday) {
            week_regular = 0;
            week_start = Some(monday);
            last_day = None;
        }
        if last_day != Some(e.date) {
            day_before = 0;
            last_day = Some(e.date);
        }
        key = Some((e.person, e.date));
        let t = e.tenths;
        let after_daily = if rules.daily_tenths > 0 { rules.daily_tenths.saturating_sub(day_before).min(t) } else { t };
        let regular = if rules.weekly_tenths > 0 { rules.weekly_tenths.saturating_sub(week_regular).min(after_daily) } else { after_daily };
        let overtime = t - regular;
        day_before = day_before.saturating_add(t);
        week_regular = week_regular.saturating_add(regular);
        let rate = rates.iter().filter(|r| r.person == e.person && r.from <= e.date).max_by_key(|r| (r.from, r.id)).map(|r| r.hourly);
        let cost = match rate {
            Some(r) => {
                let ot_rate = r.percent(mult).unwrap_or(r);
                tenths_of(r, regular).checked_add(tenths_of(ot_rate, overtime)).unwrap_or(Money::ZERO)
            }
            None => Money::ZERO,
        };
        out.push(Labour {
            entry: e.id,
            person: e.person,
            date: e.date,
            cost_code: e.cost_code,
            status: e.status,
            posted: e.posted.is_some(),
            regular,
            overtime,
            rate,
            cost,
        });
    }
    out
}

/// Labour totals for one cost code (or for no code) or one person.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LabourTotal {
    pub regular: u32,
    pub overtime: u32,
    pub cost: Money,
}

impl LabourTotal {
    fn add(&mut self, l: &Labour) {
        self.regular = self.regular.saturating_add(l.regular);
        self.overtime = self.overtime.saturating_add(l.overtime);
        self.cost = self.cost.checked_add(l.cost).unwrap_or(self.cost);
    }
}

/// The cost report for a range: totals by cost code and by person, and the people with hours but no rate.
#[derive(Clone, Debug, Default)]
pub struct CostReport {
    pub by_code: BTreeMap<Option<Id<CostCode>>, LabourTotal>,
    pub by_person: BTreeMap<Id<Person>, LabourTotal>,
    pub total: LabourTotal,
    pub no_rate: Vec<Id<Person>>,
    pub lines: Vec<Labour>,
}

pub fn report(state: &CompanyState, project: Id<Project>, from: Option<Date>, to: Option<Date>, approved_only: bool) -> CostReport {
    let mut r = CostReport::default();
    for l in labour(state, project) {
        if from.is_some_and(|f| l.date < f) || to.is_some_and(|t| l.date > t) || (approved_only && l.status != TimeStatus::Approved) {
            continue;
        }
        r.by_code.entry(l.cost_code).or_default().add(&l);
        r.by_person.entry(l.person).or_default().add(&l);
        r.total.add(&l);
        if l.rate.is_none() && !r.no_rate.contains(&l.person) {
            r.no_rate.push(l.person);
        }
        r.lines.push(l);
    }
    r
}

fn total_json(t: &LabourTotal) -> Value {
    json!({ "regular_tenths": t.regular, "overtime_tenths": t.overtime, "cost": t.cost })
}

fn keyed_json<K: serde::Serialize>(key: &str, k: &K, t: &LabourTotal) -> Value {
    json!({ key: k, "regular_tenths": t.regular, "overtime_tenths": t.overtime, "cost": t.cost })
}

fn cost_report(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let (from, to) = (p.date("from")?, p.date("to")?);
    let all = match p.line("include", 20)?.as_deref() {
        None | Some("") | Some("approved") => false,
        Some("all") => true,
        Some(other) => return Err(p.bad("include", format!("`{other}` is not approved or all"))),
    };
    let r = report(v.state, project, from, to, !all);
    Ok(json!({
        "by_cost_code": r.by_code.iter().map(|(c, t)| keyed_json("cost_code", c, t)).collect::<Vec<_>>(),
        "by_person": r.by_person.iter().map(|(x, t)| keyed_json("person", x, t)).collect::<Vec<_>>(),
        "total": total_json(&r.total),
        "people_without_rate": r.no_rate,
    }))
}

fn post_costs(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let from = p.req_date("from")?;
    let to = range_end(p, from)?;
    let fallback: Option<Id<CostCode>> = p.opt_id("default_cost_code")?;
    if let Some(c) = fallback {
        usable_code(ctx.state, project, c)?;
    }
    let lines: Vec<Labour> =
        labour(ctx.state, project).into_iter().filter(|l| l.date >= from && l.date <= to && l.status == TimeStatus::Approved && !l.posted).collect();
    if lines.is_empty() {
        return Err(EngineError::conflict("there are no approved hours in that range that were not posted already"));
    }
    let mut unpaid: Vec<Id<Person>> = lines.iter().filter(|l| l.rate.is_none()).map(|l| l.person).collect();
    unpaid.sort();
    unpaid.dedup();
    if !unpaid.is_empty() {
        let names: Vec<String> = unpaid.iter().take(5).map(|x| crate::view::person_name(ctx.state, *x)).collect();
        return Err(EngineError::conflict(format!("set a pay rate first for {}", names.join(", "))));
    }
    let mut by_code: BTreeMap<Id<CostCode>, (Money, Vec<Id<TimeEntry>>)> = BTreeMap::new();
    for l in &lines {
        let Some(code) = l.cost_code.or(fallback) else {
            return Err(EngineError::conflict("some hours have no cost code: pass default_cost_code or give them one"));
        };
        let slot = by_code.entry(code).or_insert((Money::ZERO, Vec::new()));
        slot.0 = slot.0.checked_add(l.cost).ok_or_else(|| EngineError::conflict("the labour cost is too large to post"))?;
        slot.1.push(l.entry);
    }
    let stamp = ctx.stamp();
    let mut made = Vec::new();
    let mut amount = Money::ZERO;
    for (code, (sum, entries)) in by_code {
        let id = ctx.state.direct_costs.alloc();
        ctx.state.direct_costs.put(DirectCost {
            id,
            project,
            date: to,
            cost_code: code,
            kind: CostType::Labor,
            vendor: None,
            description: format!("Labour {from} to {to}"),
            amount: sum,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        for e in entries {
            ctx.state.time_entries.update(e, |x| x.posted = Some(id));
        }
        amount = amount.checked_add(sum).unwrap_or(amount);
        made.push(id);
    }
    Ok(json!({ "posted": lines.len(), "direct_costs": made, "amount": amount }))
}
