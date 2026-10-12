//! Daily logs: what happened on site each day — weather, crews, equipment, deliveries, visitors
//! and notes.
//!
//! One log per project per day. Anyone with Standard access starts one; the author edits it while
//! it is a draft and submits it; after that only daily log admins change it (they can reopen it to
//! a draft). Drafts are visible to their author and admins only.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::daily_log::{DailyLog, Delivery, EquipmentUse, LogStatus, Manpower, Sky, Visitor, Weather, parse_hours};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "daily_log.create",
        "Start daily log",
        Tool::DailyLog,
        Access::Project(Level::Standard),
        "project: project id; date?: YYYY-MM-DD (default: today); then the log fields: sky?: clear|partly_cloudy|overcast|rain|snow|fog|windy|storm; high?, low?: integer degrees; \
         weather_notes?, work_performed?, delays?, safety?, notes?: text; manpower?: [{company, trade?, workers: integer, hours: hours each (7.5), notes?}]; \
         equipment?: [{name, hours, notes?}]; deliveries?: [{description, company?, notes?}]; visitors?: [{name, company?, purpose?}]",
        create,
    ),
    CommandSpec::mutate(
        "daily_log.update",
        "Edit daily log",
        Tool::DailyLog,
        Access::Project(Level::Standard),
        "project, id: ids; date?: YYYY-MM-DD; then any log field (see daily_log.create)",
        update,
    ),
    CommandSpec::mutate("daily_log.submit", "Submit daily log", Tool::DailyLog, Access::Project(Level::Standard), "project, id: ids", submit),
    CommandSpec::mutate(
        "daily_log.reopen",
        "Reopen daily log",
        Tool::DailyLog,
        Access::Project(Level::Admin),
        "project, id: ids (back to a draft)",
        reopen,
    ),
    CommandSpec::mutate("daily_log.delete", "Delete daily log", Tool::DailyLog, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("daily_log.restore", "Restore daily log", Tool::DailyLog, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "daily_log.list",
        "List daily logs",
        Tool::DailyLog,
        Access::Project(Level::Read),
        "project: project id; from?, to?: YYYY-MM-DD; status?: draft|submitted; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("daily_log.get", "Show daily log", Tool::DailyLog, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_ROWS: usize = 200;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<DailyLog> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<DailyLog> = p.id("id")?;
    let log = in_project(&ctx.state.daily_logs, project, id, "daily log")?;
    if !view::can_see_daily_log(ctx.state, ctx.actor, log) {
        return Err(EngineError::not_found(format!("daily log {id} not found in this project")));
    }
    Ok(log.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::DailyLog) == Level::Admin
}

/// Whether the actor may edit the log now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, log: &DailyLog) -> bool {
    let lvl = view::level(state, actor, log.project, Tool::DailyLog);
    if lvl == Level::Admin {
        return true;
    }
    lvl >= Level::Standard && log.status == LogStatus::Draft && actor.person().is_some() && log.created.by == actor.person()
}

fn hours(item: &Params<'_>, key: &str) -> Result<u32> {
    let raw = item.line(key, 10)?.ok_or_else(|| item.bad(key, "is required"))?;
    parse_hours(&raw).ok_or_else(|| item.bad(key, format!("`{raw}` is not a number of hours (0 to 24, one decimal)")))
}

/// Rows with every field blank are dropped, so forms can offer spare rows.
fn rows<'a>(p: &Params<'a>, key: &str) -> Result<Option<Vec<Params<'a>>>> {
    if !p.present(key) {
        return Ok(None);
    }
    let items = p.items(key)?;
    if items.len() > MAX_ROWS {
        return Err(p.bad(key, format!("holds more than {MAX_ROWS} rows")));
    }
    Ok(Some(items))
}

fn apply_fields(p: &Params<'_>, log: &mut DailyLog) -> Result<()> {
    if let Some(sky) = p.choice::<Sky>("sky")? {
        log.weather.sky = sky;
    }
    for (key, field) in [("high", &mut log.weather.high), ("low", &mut log.weather.low)] {
        if p.present(key) {
            *field = p.int(key, -100..=200)?.and_then(|v| i32::try_from(v).ok());
        }
    }
    if p.present("weather_notes") {
        log.weather.notes = p.text_or_empty("weather_notes", 2_000)?;
    }
    for (key, field) in
        [("work_performed", &mut log.work_performed), ("delays", &mut log.delays), ("safety", &mut log.safety), ("notes", &mut log.notes)]
    {
        if p.present(key) {
            *field = p.text_or_empty(key, text::LONG)?;
        }
    }
    if let Some(items) = rows(p, "manpower")? {
        let mut out = Vec::new();
        for item in items {
            let company = item.line_or_empty("company", 200)?;
            let workers = item.int("workers", 0..=10_000)?.unwrap_or(0);
            if company.is_empty() && workers == 0 {
                continue;
            }
            if company.is_empty() {
                return Err(item.bad("company", "is required for a crew"));
            }
            out.push(Manpower {
                company,
                trade: item.line_or_empty("trade", 100)?,
                workers: u32::try_from(workers).unwrap_or(0),
                hours_tenths: if item.present("hours") && item.line("hours", 10)?.is_some() { hours(&item, "hours")? } else { 0 },
                notes: item.line_or_empty("notes", text::SHORT)?,
            });
        }
        log.manpower = out;
    }
    if let Some(items) = rows(p, "equipment")? {
        let mut out = Vec::new();
        for item in items {
            let name = item.line_or_empty("name", 200)?;
            if name.is_empty() {
                continue;
            }
            out.push(EquipmentUse {
                name,
                hours_tenths: if item.present("hours") && item.line("hours", 10)?.is_some() { hours(&item, "hours")? } else { 0 },
                notes: item.line_or_empty("notes", text::SHORT)?,
            });
        }
        log.equipment = out;
    }
    if let Some(items) = rows(p, "deliveries")? {
        let mut out = Vec::new();
        for item in items {
            let description = item.line_or_empty("description", text::SHORT)?;
            if description.is_empty() {
                continue;
            }
            out.push(Delivery { description, company: item.line_or_empty("company", 200)?, notes: item.line_or_empty("notes", text::SHORT)? });
        }
        log.deliveries = out;
    }
    if let Some(items) = rows(p, "visitors")? {
        let mut out = Vec::new();
        for item in items {
            let name = item.line_or_empty("name", 200)?;
            if name.is_empty() {
                continue;
            }
            out.push(Visitor { name, company: item.line_or_empty("company", 200)?, purpose: item.line_or_empty("purpose", text::SHORT)? });
        }
        log.visitors = out;
    }
    Ok(())
}

fn one_per_day(ctx: &Ctx<'_>, project: Id<Project>, date: Date, except: Option<Id<DailyLog>>) -> Result<()> {
    if ctx.state.daily_logs.iter().any(|l| l.project == project && l.date == date && l.deleted.is_none() && Some(l.id) != except) {
        return Err(EngineError::conflict(format!("there is already a daily log for {}", date.pretty())));
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let date = p.date("date")?.unwrap_or_else(|| ctx.today());
    one_per_day(ctx, project, date, None)?;
    let stamp = ctx.stamp();
    let mut log = DailyLog {
        id: ctx.state.daily_logs.peek_next(),
        project,
        date,
        status: LogStatus::Draft,
        weather: Weather::default(),
        work_performed: String::new(),
        delays: String::new(),
        safety: String::new(),
        notes: String::new(),
        manpower: Vec::new(),
        equipment: Vec::new(),
        deliveries: Vec::new(),
        visitors: Vec::new(),
        submitted: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(p, &mut log)?;
    let id = ctx.state.daily_logs.alloc();
    log.id = id;
    ctx.state.daily_logs.put(log);
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut log = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &log) {
        return Err(EngineError::forbidden(match log.status {
            LogStatus::Submitted => "a submitted daily log can only be changed by a daily log admin (who can reopen it)".to_string(),
            LogStatus::Draft => "only the author or a daily log admin can edit this daily log".to_string(),
        }));
    }
    let before = log.clone();
    if let Some(date) = p.date("date")? {
        one_per_day(ctx, log.project, date, Some(log.id))?;
        log.date = date;
    }
    apply_fields(p, &mut log)?;
    let id = log.id;
    if log != before {
        log.updated = ctx.stamp();
        ctx.state.daily_logs.put(log);
    }
    Ok(json!({ "id": id }))
}

fn submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut log = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &log) {
        return Err(EngineError::forbidden("only the author or a daily log admin can submit this daily log"));
    }
    if log.status == LogStatus::Submitted {
        return Err(EngineError::conflict("this daily log is already submitted"));
    }
    log.status = LogStatus::Submitted;
    log.submitted = Some(ctx.stamp());
    log.updated = ctx.stamp();
    let id = log.id;
    ctx.state.daily_logs.put(log);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut log = load(ctx, p)?;
    if log.status != LogStatus::Submitted {
        return Err(EngineError::conflict("this daily log is already a draft"));
    }
    log.status = LogStatus::Draft;
    log.submitted = None;
    log.updated = ctx.stamp();
    let id = log.id;
    ctx.state.daily_logs.put(log);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let log = load(ctx, p)?;
    let own_draft = log.status == LogStatus::Draft && log.created.by.is_some() && log.created.by == ctx.person();
    if !is_admin(ctx, log.project) && !own_draft {
        return Err(EngineError::forbidden("only daily log admins can delete daily logs (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    ctx.state.daily_logs.update(log.id, |l| l.deleted = Some(stamp));
    Ok(json!({ "id": log.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<DailyLog> = p.id("id")?;
    let log = in_project(&ctx.state.daily_logs, project, id, "daily log")?;
    if log.deleted.is_none() {
        return Err(EngineError::conflict("this daily log is not in the recycle bin"));
    }
    one_per_day(ctx, project, log.date, Some(id))?;
    ctx.state.daily_logs.update(id, |l| l.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::DailyLog) < Level::Admin {
            return Err(EngineError::forbidden("only daily log admins can see the recycle bin"));
        }
        let bin: Vec<&DailyLog> = v.state.daily_logs.iter().filter(|l| l.project == project && l.deleted.is_some()).collect();
        return json(&bin);
    }
    let (from, to) = (p.date("from")?, p.date("to")?);
    let status = p.choice::<LogStatus>("status")?;
    let list: Vec<&DailyLog> = view::daily_logs_for(v.state, v.actor, project)
        .into_iter()
        .filter(|l| from.is_none_or(|f| l.date >= f) && to.is_none_or(|t| l.date <= t))
        .filter(|l| status.is_none_or(|s| l.status == s))
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<DailyLog> = p.id("id")?;
    let log = in_project(&v.state.daily_logs, project, id, "daily log")?;
    if !view::can_see_daily_log(v.state, v.actor, log) {
        return Err(EngineError::not_found(format!("daily log {id} not found in this project")));
    }
    json(log)
}
