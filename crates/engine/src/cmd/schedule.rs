//! The schedule: tasks with working-day dates, finish-to-start links, progress and a baseline.
//!
//! Links are by task number. Summary tasks (those with children) roll up their children, and the
//! critical path is computed from the leaf tasks' links (see `sched`).

use std::collections::HashMap;

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::Choice;
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::{
    Assignment, Constraint, Holiday, Link, LinkKind, MAX_ASSIGNMENTS, MAX_HOLIDAYS, MAX_UNITS, ScheduleTask, WorkCalendar,
};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::sched::{self, MAX_DEPTH, MAX_TASKS};
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "schedule.add",
        "Add schedule task",
        Tool::Schedule,
        Access::Project(Level::Standard),
        concat!(
            "project: project id; ",
            "name: text; parent?: parent task id (empty for top level); start: YYYY-MM-DD; finish?: YYYY-MM-DD; days?: working days (when finish is blank; default 1); milestone?: bool; percent?: 0-100; predecessors?: text such as `3, 5+2, 7SS, 4FF-1` (task numbers, each with an optional kind FS, SS, FF or SF (default FS) and an optional lag in working days); company?: company id; notes?: text; constraint?: none|snet|mso|fnlt (start no earlier than, must start on, finish no later than); constraint_date?: YYYY-MM-DD (required with a constraint); resources?: text such as `Carpenter 4, Laborer 2` (what the task needs each working day)"
        ),
        add,
    ),
    CommandSpec::mutate(
        "schedule.update",
        "Edit schedule task",
        Tool::Schedule,
        Access::Project(Level::Standard),
        "project: project id; id: task id; any of the fields of schedule.add",
        update,
    ),
    CommandSpec::mutate(
        "schedule.move",
        "Move or resize a task",
        Tool::Schedule,
        Access::Project(Level::Standard),
        "project: project id; id: task id; shift?: calendar days to move the whole task by (negative for earlier; it keeps its length in working days and lands on a working day); finish_shift?: calendar days to move only the finish by (a longer or shorter task). One of the two is required. Summary tasks follow their children and can't be moved.",
        move_task,
    ),
    CommandSpec::mutate(
        "schedule.progress",
        "Set task progress",
        Tool::Schedule,
        Access::Project(Level::Standard),
        "project: project id; id: task id; percent: 0-100",
        progress,
    ),
    CommandSpec::mutate(
        "schedule.delete",
        "Delete schedule task",
        Tool::Schedule,
        Access::Project(Level::Admin),
        "project: project id; id: task id (it must have no child tasks)",
        delete,
    ),
    CommandSpec::mutate(
        "schedule.restore",
        "Restore schedule task",
        Tool::Schedule,
        Access::Project(Level::Admin),
        "project: project id; id: task id",
        restore,
    ),
    CommandSpec::mutate(
        "schedule.recalculate",
        "Move tasks that start too early",
        Tool::Schedule,
        Access::Project(Level::Standard),
        "project: project id (pushes each task that starts before its predecessors allow, keeping its length)",
        recalculate,
    ),
    CommandSpec::mutate(
        "schedule.baseline",
        "Save baseline",
        Tool::Schedule,
        Access::Project(Level::Admin),
        "project: project id; clear?: bool (remove the baseline instead)",
        baseline,
    ),
    CommandSpec::mutate(
        "schedule.import",
        "Import schedule",
        Tool::Schedule,
        Access::Project(Level::Standard),
        "project: project id; text: lines of `id, name, start, finish, percent, predecessors, parent[, milestone, constraint, constraint date]` (comma or tab separated; predecessors separated by ; and may carry a kind and lag such as 4SS+2; ids are the file's own and are renumbered)",
        import,
    ),
    CommandSpec::mutate(
        "schedule.calendar_set",
        "Set the project's working days and holidays",
        Tool::Schedule,
        Access::Project(Level::Admin),
        "project: project id; days?: the weekdays worked, such as `mon-fri` or `mon, tue, wed, thu, fri, sat` (at least one); \
         holidays?: days off, one per line as `YYYY-MM-DD name` (replaces the list; up to 400). Task dates stay as they are: run schedule.recalculate to move tasks that now start too early.",
        calendar_set,
    ),
    CommandSpec::mutate(
        "schedule.resource_limits",
        "Set how many of each resource the project has",
        Tool::Schedule,
        Access::Project(Level::Admin),
        "project: project id; text: one per line or separated by commas, such as `Carpenter 6, Laborer 10` (replaces the list; empty clears it)",
        resource_limits,
    ),
    CommandSpec::query(
        "schedule.loading",
        "Resource loading by week",
        Tool::Schedule,
        Access::Project(Level::Read),
        "project: project id; weeks?: 1-26 (default 8); from?: YYYY-MM-DD (default today) (for each resource and week: the most needed on any one working day, against the limit)",
        loading,
    ),
    CommandSpec::query(
        "schedule.calendar",
        "Get the project's working days and holidays",
        Tool::Schedule,
        Access::Project(Level::Read),
        "project: project id",
        calendar_get,
    ),
    CommandSpec::query(
        "schedule.lookahead",
        "Look ahead",
        Tool::Schedule,
        Access::Project(Level::Read),
        "project: project id; weeks?: 1-12 (default 3); from?: YYYY-MM-DD (default today) (the work that is under way or starts in each week, not yet finished)",
        lookahead,
    ),
    CommandSpec::query(
        "schedule.list",
        "List schedule",
        Tool::Schedule,
        Access::Project(Level::Read),
        "project: project id; deleted?: bool (admins) (every task in order with float and critical flags)",
        list,
    ),
    CommandSpec::query("schedule.get", "Get schedule task", Tool::Schedule, Access::Project(Level::Read), "project: project id; id: task id", get),
];

fn live_task(ctx: &impl Read, project: Id<Project>, id: Id<ScheduleTask>) -> Result<ScheduleTask> {
    let t = in_project(&ctx.state().schedule_tasks, project, id, "task")?;
    if t.deleted.is_some() {
        return Err(EngineError::not_found(format!("task {id} not found in this project")));
    }
    Ok(t.clone())
}

fn next_number(ctx: &Ctx<'_>, project: Id<Project>) -> u32 {
    ctx.state.schedule_tasks.iter().filter(|t| t.project == project).map(|t| t.number).max().unwrap_or(0).saturating_add(1)
}

/// `3, 5+2, 7SS, 4FF-1` → links by task number.
fn parse_links(ctx: &Ctx<'_>, project: Id<Project>, key: &str, raw: &str, me: Option<Id<ScheduleTask>>, p: &Params<'_>) -> Result<Vec<Link>> {
    let mut out: Vec<Link> = Vec::new();
    for part in raw.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
        let (num, kind, lag) = sched::parse_link(part).map_err(|e| p.bad(key, e))?;
        let task = ctx
            .state
            .schedule_tasks
            .iter()
            .find(|t| t.project == project && t.deleted.is_none() && t.number == num)
            .ok_or_else(|| p.bad(key, format!("there is no task {num} in this schedule")))?;
        if Some(task.id) == me {
            return Err(p.bad(key, "a task can't follow itself"));
        }
        if out.iter().any(|l| l.task == task.id) {
            continue;
        }
        if me.is_some_and(|me| sched::reaches(ctx.state, task.id, me)) {
            return Err(p.bad(key, format!("task {num} already depends on this task; that would make a loop")));
        }
        out.push(Link { task: task.id, lag, kind });
        if out.len() > 50 {
            return Err(p.bad(key, "lists more than 50 predecessors"));
        }
    }
    Ok(out)
}

fn depth_of(ctx: &Ctx<'_>, mut at: Option<Id<ScheduleTask>>) -> usize {
    let mut n = 0;
    while let Some(id) = at {
        n += 1;
        if n > MAX_DEPTH + 1 {
            break;
        }
        at = ctx.state.schedule_tasks.get(id).and_then(|t| t.parent);
    }
    n
}

fn set_parent(ctx: &Ctx<'_>, project: Id<Project>, me: Option<Id<ScheduleTask>>, p: &Params<'_>) -> Result<Option<Id<ScheduleTask>>> {
    let parent: Option<Id<ScheduleTask>> = p.opt_id("parent")?;
    let Some(parent) = parent else { return Ok(None) };
    let t = live_task(ctx, project, parent)?;
    if me == Some(parent) {
        return Err(p.bad("parent", "a task can't be its own parent"));
    }
    // Not inside its own subtree.
    let mut at = t.parent;
    for _ in 0..=MAX_DEPTH + 1 {
        match at {
            Some(a) if Some(a) == me => return Err(p.bad("parent", "can't be one of the task's own children")),
            Some(a) => at = ctx.state.schedule_tasks.get(a).and_then(|x| x.parent),
            None => break,
        }
    }
    if depth_of(ctx, Some(parent)) >= MAX_DEPTH {
        return Err(EngineError::conflict(format!("summary tasks nest at most {MAX_DEPTH} deep")));
    }
    Ok(Some(parent))
}

fn dates(cal: &WorkCalendar, p: &Params<'_>, current: Option<&ScheduleTask>) -> Result<(Date, Date, bool)> {
    let milestone = p.flag("milestone")?.unwrap_or_else(|| current.is_some_and(|t| t.milestone));
    let start = match p.date("start")? {
        Some(d) => d,
        None => current.map(|t| t.start).ok_or_else(|| p.bad("start", "is required"))?,
    };
    let finish = if milestone {
        start
    } else if let Some(f) = p.date("finish")? {
        f
    } else if let Some(days) = p.int("days", 1..=5_000)? {
        sched::shift_in(cal, start, days - 1).ok_or_else(|| p.bad("days", "runs past the supported dates"))?
    } else if let Some(t) = current {
        // Keep the task's length when only the start moved.
        if p.present("start") {
            let len = sched::days_inclusive_in(cal, t.start, t.finish).max(1);
            sched::shift_in(cal, start, len - 1).ok_or_else(|| p.bad("start", "runs past the supported dates"))?
        } else {
            t.finish
        }
    } else {
        start
    };
    if finish < start {
        return Err(p.bad("finish", "is before the start"));
    }
    if start.days_until(finish) > 3_650 {
        return Err(p.bad("finish", "is more than ten years after the start"));
    }
    Ok((start, finish, milestone))
}

/// `Carpenter 4, Laborer 2` → what is needed each working day; a name alone means one, and a name
/// written twice adds up.
fn parse_resources(p: &Params<'_>, key: &str, raw: &str) -> Result<Vec<Assignment>> {
    let mut out: Vec<Assignment> = Vec::new();
    for part in raw.split([',', ';', '\n']).map(str::trim).filter(|s| !s.is_empty()) {
        let (name, units) = match part.rsplit_once(char::is_whitespace) {
            Some((n, u)) if u.bytes().all(|b| b.is_ascii_digit()) && !u.is_empty() => {
                (n.trim(), u.parse::<u32>().map_err(|_| p.bad(key, format!("`{part}`: the number is too large")))?)
            }
            _ => (part, 1),
        };
        let name: String = name.chars().take(50).collect();
        if name.is_empty() || name.bytes().all(|b| b.is_ascii_digit()) || units == 0 || units > MAX_UNITS {
            return Err(p.bad(key, format!("`{part}`: a name and a number from 1 to {MAX_UNITS}")));
        }
        match out.iter_mut().find(|a| a.name.eq_ignore_ascii_case(&name)) {
            Some(a) => a.units = a.units.saturating_add(units).min(MAX_UNITS),
            None => out.push(Assignment { name, units }),
        }
        if out.len() > MAX_ASSIGNMENTS {
            return Err(p.bad(key, format!("lists more than {MAX_ASSIGNMENTS} resources")));
        }
    }
    Ok(out)
}

/// The constraint asked for (or the task's own when the parameters leave it alone): a kind and, unless
/// it is "as soon as possible", its date.
fn constraint_of(p: &Params<'_>, current: Option<&ScheduleTask>) -> Result<(Constraint, Option<Date>)> {
    let kind = match p.choice::<Constraint>("constraint")? {
        Some(k) => k,
        None => current.map(|t| t.constraint).unwrap_or_default(),
    };
    if kind == Constraint::None {
        if p.present("constraint_date") && !p.present("constraint") {
            return Err(p.bad("constraint", "is needed with a constraint date (this task has none)"));
        }
        return Ok((kind, None));
    }
    let date = match p.date("constraint_date")? {
        Some(d) => Some(d),
        None => current.and_then(|t| t.constraint_date),
    };
    match date {
        Some(d) => Ok((kind, Some(d))),
        None => Err(p.bad("constraint_date", format!("is required for `{}`", kind.label()))),
    }
}

fn percent(p: &Params<'_>) -> Result<Option<u8>> {
    Ok(p.int("percent", 0..=100)?.and_then(|v| u8::try_from(v).ok()))
}

fn company(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<Option<Id<Vendor>>>> {
    if !p.present("company") {
        return Ok(None);
    }
    let v: Option<Id<Vendor>> = p.opt_id("company")?;
    if let Some(v) = v {
        ctx.check_vendor(v)?;
    }
    Ok(Some(v))
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if ctx.state.schedule_tasks.iter().filter(|t| t.project == project).count() >= MAX_TASKS {
        return Err(EngineError::conflict(format!("a schedule holds at most {MAX_TASKS} tasks")));
    }
    let name = p.req_line("name", text::SHORT)?;
    let parent = set_parent(ctx, project, None, p)?;
    let (start, finish, milestone) = dates(&sched::calendar(ctx.state, project), p, None)?;
    let predecessors = if p.present("predecessors") {
        parse_links(ctx, project, "predecessors", &p.text_or_empty("predecessors", 500)?, None, p)?
    } else {
        Vec::new()
    };
    let (constraint, constraint_date) = constraint_of(p, None)?;
    let resources = if p.present("resources") { parse_resources(p, "resources", &p.text_or_empty("resources", 1_000)?)? } else { Vec::new() };
    let id = ctx.state.schedule_tasks.alloc();
    let number = next_number(ctx, project);
    let stamp = ctx.stamp();
    ctx.state.schedule_tasks.put(ScheduleTask {
        id,
        project,
        number,
        name,
        parent,
        start,
        finish,
        milestone,
        percent: percent(p)?.unwrap_or(0),
        predecessors,
        company: company(ctx, p)?.flatten(),
        notes: p.text_or_empty("notes", text::LONG)?,
        constraint,
        constraint_date,
        resources,
        baseline_start: None,
        baseline_finish: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    let mut t = live_task(ctx, project, id)?;
    if p.present("name") {
        t.name = p.req_line("name", text::SHORT)?;
    }
    if p.present("parent") {
        t.parent = set_parent(ctx, project, Some(id), p)?;
    }
    if p.present("start") || p.present("finish") || p.present("days") || p.present("milestone") {
        let (s, f, m) = dates(&sched::calendar(ctx.state, project), p, Some(&t))?;
        (t.start, t.finish, t.milestone) = (s, f, m);
    }
    if p.present("predecessors") {
        t.predecessors = parse_links(ctx, project, "predecessors", &p.text_or_empty("predecessors", 500)?, Some(id), p)?;
    }
    if let Some(x) = percent(p)? {
        t.percent = x;
    }
    if let Some(c) = company(ctx, p)? {
        t.company = c;
    }
    if p.present("notes") {
        t.notes = p.text_or_empty("notes", text::LONG)?;
    }
    if p.present("constraint") || p.present("constraint_date") {
        (t.constraint, t.constraint_date) = constraint_of(p, Some(&t))?;
    }
    if p.present("resources") {
        t.resources = parse_resources(p, "resources", &p.text_or_empty("resources", 1_000)?)?;
    }
    t.updated = ctx.stamp();
    ctx.state.schedule_tasks.put(t);
    Ok(json!({ "id": id }))
}

fn move_task(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    let mut t = live_task(ctx, project, id)?;
    if ctx.state.schedule_tasks.iter().any(|x| x.project == project && x.deleted.is_none() && x.parent == Some(id)) {
        return Err(EngineError::conflict("a summary task follows its tasks; move those instead"));
    }
    let cal = sched::calendar(ctx.state, project);
    // The nearest working day at or after (or before) a date.
    let snap = |d: Date, dir: i64| -> Option<Date> {
        let mut d = d;
        for _ in 0..1_000 {
            if cal.is_working(d) {
                return Some(d);
            }
            d = d.add_days(dir)?;
        }
        None
    };
    let (shift, finish_shift) = (p.int("shift", -3_650..=3_650)?, p.int("finish_shift", -3_650..=3_650)?);
    let far = || p.bad("shift", "runs past the supported dates");
    match (shift, finish_shift) {
        (Some(by), None) => {
            let len = if t.milestone { 1 } else { sched::days_inclusive_in(&cal, t.start, t.finish).max(1) };
            let start = snap(t.start.add_days(by).ok_or_else(far)?, if by < 0 { -1 } else { 1 }).ok_or_else(far)?;
            t.finish = if t.milestone { start } else { sched::shift_in(&cal, start, len - 1).ok_or_else(far)? };
            t.start = start;
        }
        (None, Some(by)) => {
            if t.milestone {
                return Err(p.bad("finish_shift", "a milestone has no length; move it instead"));
            }
            let finish = snap(t.finish.add_days(by).ok_or_else(far)?, if by < 0 { -1 } else { 1 }).ok_or_else(far)?;
            if finish < t.start {
                return Err(p.bad("finish_shift", "would finish the task before it starts"));
            }
            t.finish = finish;
        }
        _ => return Err(p.bad("shift", "give shift (to move the task) or finish_shift (to change its length), not both")),
    }
    if t.start.days_until(t.finish) > 3_650 {
        return Err(p.bad("finish_shift", "makes the task longer than ten years"));
    }
    let (start, finish) = (t.start, t.finish);
    t.updated = ctx.stamp();
    ctx.state.schedule_tasks.put(t);
    Ok(json!({ "id": id, "start": start, "finish": finish }))
}

fn progress(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    let mut t = live_task(ctx, project, id)?;
    t.percent = percent(p)?.ok_or_else(|| p.bad("percent", "is required"))?;
    t.updated = ctx.stamp();
    ctx.state.schedule_tasks.put(t);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    live_task(ctx, project, id)?;
    if ctx.state.schedule_tasks.iter().any(|t| t.deleted.is_none() && t.parent == Some(id)) {
        return Err(EngineError::conflict("the task has child tasks; delete or move them first"));
    }
    let stamp = ctx.stamp();
    ctx.state.schedule_tasks.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    let t = in_project(&ctx.state.schedule_tasks, project, id, "task")?;
    if t.deleted.is_none() {
        return Err(EngineError::conflict("the task is not in the recycle bin"));
    }
    ctx.state.schedule_tasks.update(id, |t| t.deleted = None);
    Ok(json!({ "id": id }))
}

fn recalculate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let cal = sched::calendar(ctx.state, project);
    // Repeat until nothing moves; every pass settles at least one more level of the chain.
    let mut moved: HashMap<Id<ScheduleTask>, ()> = HashMap::new();
    for _ in 0..=MAX_TASKS.min(200) {
        let tasks: Vec<ScheduleTask> = ctx.state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).cloned().collect();
        let by_id: HashMap<Id<ScheduleTask>, &ScheduleTask> = tasks.iter().map(|t| (t.id, t)).collect();
        let mut changed = false;
        for t in &tasks {
            let from_links = sched::earliest_by_links(&cal, t, &by_id);
            // The constraint decides the start: a floor, or a fixed day.
            let (need, fixed) = match (t.constraint, t.constraint_date) {
                (Constraint::StartNoEarlierThan, Some(d)) => (from_links.into_iter().chain(sched::shift_in(&cal, d, 0)).max(), false),
                (Constraint::MustStartOn, Some(d)) => (sched::shift_in(&cal, d, 0), true),
                _ => (from_links, false),
            };
            let Some(need) = need.filter(|n| if fixed { *n != t.start } else { *n > t.start }) else { continue };
            let len = if t.milestone { 1 } else { sched::days_inclusive_in(&cal, t.start, t.finish).max(1) };
            let Some(finish) = sched::shift_in(&cal, need, len - 1) else { continue };
            let finish = if t.milestone { need } else { finish };
            ctx.state.schedule_tasks.update(t.id, |x| {
                x.start = need;
                x.finish = finish;
            });
            moved.insert(t.id, ());
            changed = true;
        }
        if !changed {
            return Ok(json!({ "moved": moved.len() }));
        }
    }
    Err(EngineError::conflict("the links don't settle; check them for a loop"))
}

fn baseline(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let clear = p.flag_or("clear", false)?;
    let ids: Vec<Id<ScheduleTask>> = ctx.state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).map(|t| t.id).collect();
    for id in &ids {
        ctx.state.schedule_tasks.update(*id, |t| {
            (t.baseline_start, t.baseline_finish) = if clear { (None, None) } else { (Some(t.start), Some(t.finish)) };
        });
    }
    Ok(json!({ "tasks": ids.len() }))
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let body = p.req_text("text", 1_000_000)?;
    import_body(ctx, project, &body, p)
}

/// Adds the tasks in `body`: lines of `id, name, start, finish, percent, predecessors, parent`, then optionally
/// `milestone` (yes/no), `constraint` (snet, mso, fnlt) and its date. Also used by the project-file import.
pub(super) fn import_body(ctx: &mut Ctx<'_>, project: Id<Project>, body: &str, p: &Params<'_>) -> Result<Value> {
    struct Pending {
        milestone: bool,
        constraint: Constraint,
        constraint_date: Option<Date>,
        file_id: String,
        name: String,
        start: Date,
        finish: Date,
        percent: u8,
        preds: String,
        parent: String,
    }
    let mut rows: Vec<Pending> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let sep = if line.contains('\t') { '\t' } else { ',' };
        let cells: Vec<&str> = line.split(sep).map(|c| c.trim().trim_matches('"')).collect();
        let cell = |i: usize| cells.get(i).copied().unwrap_or("");
        let at = |what: &str, e: String| p.bad("text", format!("line {}: {what} {e}", n + 1));
        if cell(0).eq_ignore_ascii_case("id") {
            continue; // a header line
        }
        if rows.len() >= MAX_TASKS {
            return Err(p.bad("text", format!("has more than {MAX_TASKS} tasks")));
        }
        let name = text::required_line(cell(1), text::SHORT).map_err(|e| at("name", e.to_string()))?;
        let start = Date::parse(cell(2)).ok_or_else(|| at("start", format!("`{}` is not a date (YYYY-MM-DD)", cell(2))))?;
        let finish = if cell(3).is_empty() {
            start
        } else {
            Date::parse(cell(3)).ok_or_else(|| at("finish", format!("`{}` is not a date (YYYY-MM-DD)", cell(3))))?
        };
        if finish < start {
            return Err(at("finish", "is before the start".into()));
        }
        let pct = if cell(4).is_empty() {
            0
        } else {
            cell(4).trim_end_matches('%').parse::<u8>().ok().filter(|v| *v <= 100).ok_or_else(|| at("percent", "must be 0 to 100".into()))?
        };
        let milestone = match cell(7).to_ascii_lowercase().as_str() {
            "" => start == finish && name.to_lowercase().contains("milestone"),
            v => matches!(v, "1" | "true" | "yes" | "y"),
        };
        let (constraint, constraint_date) = match Constraint::parse(cell(8)).filter(|k| *k != Constraint::None) {
            Some(k) => (k, Some(Date::parse(cell(9)).ok_or_else(|| at("constraint date", format!("`{}` is not a date (YYYY-MM-DD)", cell(9))))?)),
            None => (Constraint::None, None),
        };
        rows.push(Pending {
            milestone,
            constraint,
            constraint_date,
            file_id: cell(0).to_string(),
            name,
            start,
            finish,
            percent: pct,
            preds: cell(5).to_string(),
            parent: cell(6).to_string(),
        });
    }
    if ctx.state.schedule_tasks.iter().filter(|t| t.project == project).count() + rows.len() > MAX_TASKS {
        return Err(EngineError::conflict(format!("a schedule holds at most {MAX_TASKS} tasks")));
    }
    // Allocate in file order, then resolve links and parents by the file's ids.
    let first = next_number(ctx, project);
    let mut map: HashMap<String, Id<ScheduleTask>> = HashMap::new();
    let mut made = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        let id = ctx.state.schedule_tasks.alloc();
        if !r.file_id.is_empty() && map.insert(r.file_id.clone(), id).is_some() {
            return Err(p.bad("text", format!("the id {} is used twice", r.file_id)));
        }
        let stamp = ctx.stamp();
        let number = first.saturating_add(u32::try_from(i).unwrap_or(0));
        ctx.state.schedule_tasks.put(ScheduleTask {
            id,
            project,
            number,
            name: r.name.clone(),
            parent: None,
            start: r.start,
            finish: r.finish,
            milestone: r.milestone,
            percent: r.percent,
            predecessors: Vec::new(),
            company: None,
            notes: String::new(),
            constraint: r.constraint,
            constraint_date: r.constraint_date,
            resources: Vec::new(),
            baseline_start: None,
            baseline_finish: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        made.push(id);
    }
    for (r, id) in rows.iter().zip(&made) {
        let mut links = Vec::new();
        for part in r.preds.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            // The file's own ids may be any text, so a kind (`SS`, `FF`, `SF`, `FS`) is only read when the id before it is known.
            let (token, lag) = match part.find(['+', '-']) {
                Some(i) => {
                    let (n, l) = part.split_at(i);
                    (
                        n.trim(),
                        l.trim().replace(' ', "").parse::<i32>().map_err(|_| p.bad("text", format!("task {}: `{part}` has a bad lag", r.file_id)))?,
                    )
                }
                None => (part, 0),
            };
            if !(-1_000..=1_000).contains(&lag) {
                return Err(p.bad("text", format!("task {}: `{part}`: the lag is out of range", r.file_id)));
            }
            let (num, kind) = match map.get(token) {
                Some(_) => (token, LinkKind::FinishToStart),
                None => {
                    let cut = token.len().saturating_sub(2);
                    match (token.get(..cut), token.get(cut..).and_then(buildkiosk_model::Choice::parse)) {
                        (Some(id), Some(kind)) if map.contains_key(id) => (id, kind),
                        _ => (token, LinkKind::FinishToStart),
                    }
                }
            };
            let pred = *map.get(num).ok_or_else(|| p.bad("text", format!("task {}: predecessor {num} is not in the file", r.file_id)))?;
            if pred == *id || sched::reaches(ctx.state, pred, *id) {
                return Err(p.bad("text", format!("task {}: predecessor {num} would make a loop", r.file_id)));
            }
            if !links.iter().any(|l: &Link| l.task == pred) {
                links.push(Link { task: pred, lag, kind });
            }
        }
        let parent = if r.parent.is_empty() {
            None
        } else {
            Some(*map.get(&r.parent).ok_or_else(|| p.bad("text", format!("task {}: parent {} is not in the file", r.file_id, r.parent)))?)
        };
        ctx.state.schedule_tasks.update(*id, |t| {
            t.predecessors = links;
            t.parent = parent;
        });
    }
    // Reject parent chains that are too deep or that loop.
    for id in &made {
        if depth_of(ctx, Some(*id)) > MAX_DEPTH {
            return Err(p.bad("text", format!("tasks nest more than {MAX_DEPTH} deep")));
        }
    }
    Ok(json!({ "added": made.len() }))
}

/// `mon-fri`, `mon, wed, fri`, `monday`: the weekdays named, Monday first.
fn parse_days(p: &Params<'_>, raw: &str) -> Result<[bool; 7]> {
    const NAMES: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    // Three letters or more of a weekday's name: `mon`, `tues`, `Wednesday`.
    let day = |name: &str| -> Result<usize> {
        let n = name.trim().to_ascii_lowercase();
        NAMES.iter().position(|full| n.len() >= 3 && full.starts_with(&n)).ok_or_else(|| p.bad("days", format!("`{name}` is not a weekday")))
    };
    let mut days = [false; 7];
    for part in raw.split([',', ';', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                let (a, b) = (day(a)?, day(b)?);
                if a > b {
                    return Err(p.bad("days", format!("`{part}`: a range runs from an earlier weekday to a later one (Monday first)")));
                }
                for flag in days.iter_mut().take(b + 1).skip(a) {
                    *flag = true;
                }
            }
            None => {
                if let Some(flag) = days.get_mut(day(part)?) {
                    *flag = true;
                }
            }
        }
    }
    if !days.iter().any(|d| *d) {
        return Err(p.bad("days", "needs at least one working day"));
    }
    Ok(days)
}

fn calendar_set(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut cal = sched::calendar(ctx.state, project);
    if p.present("days") {
        cal.days = parse_days(p, &p.text_or_empty("days", 200)?)?;
    }
    if p.present("holidays") {
        let raw = p.text_or_empty("holidays", 20_000)?;
        let mut list: Vec<Holiday> = Vec::new();
        for line in raw.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let (date, name) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let date = Date::parse(date).ok_or_else(|| p.bad("holidays", format!("`{line}`: start each line with a date as YYYY-MM-DD")))?;
            let name: String = name.trim().chars().take(100).collect();
            if list.iter().any(|h| h.date == date) {
                return Err(p.bad("holidays", format!("{date} is listed twice")));
            }
            list.push(Holiday { date, name });
            if list.len() > MAX_HOLIDAYS {
                return Err(p.bad("holidays", format!("lists more than {MAX_HOLIDAYS} days off")));
            }
        }
        list.sort_by_key(|h| h.date);
        cal.holidays = list;
    }
    if !cal.is_valid() {
        return Err(p.bad("days", "needs at least one working day"));
    }
    ctx.state.projects.update(project, |x| x.calendar = cal);
    Ok(json!({ "id": project }))
}

fn resource_limits(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if ctx.state.projects.get(project).is_none() {
        return Err(EngineError::not_found("project not found"));
    }
    let limits = parse_resources(p, "text", &p.text_or_empty("text", 1_000)?)?;
    ctx.state.projects.update(project, |x| x.resource_limits = limits);
    Ok(json!({ "id": project }))
}

fn loading(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    in_project_exists(v, project)?;
    let weeks = p.int("weeks", 1..=26)?.unwrap_or(8);
    let from = p.date("from")?.unwrap_or_else(|| v.today());
    json(&sched::loading(v.state, project, from, weeks))
}

fn calendar_get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    in_project_exists(v, project)?;
    json(&sched::calendar(v.state, project))
}

fn in_project_exists(v: &View<'_>, project: Id<Project>) -> Result<()> {
    v.state.projects.get(project).map(|_| ()).ok_or_else(|| EngineError::not_found("project not found"))
}

fn lookahead(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    in_project_exists(v, project)?;
    let weeks = p.int("weeks", 1..=12)?.unwrap_or(3);
    let from = p.date("from")?.unwrap_or_else(|| v.today());
    let rows = sched::rows(v.state, project, v.today());
    let mut out = Vec::new();
    for i in 0..weeks {
        let (Some(start), Some(end)) = (from.add_days(i * 7), from.add_days(i * 7 + 6)) else { break };
        let tasks: Vec<Value> = rows
            .iter()
            .filter(|r| !r.summary && r.start <= end && r.finish >= start && (r.percent < 100 || r.milestone))
            .map(|r| {
                let company = v.state.schedule_tasks.get(r.id).and_then(|t| t.company).and_then(|c| v.state.vendors.get(c)).map(|c| c.name.clone());
                json!({
                    "number": r.number, "name": r.name, "start": r.start, "finish": r.finish, "percent": r.percent, "milestone": r.milestone,
                    "critical": r.critical, "late": r.late, "company": company,
                })
            })
            .collect();
        out.push(json!({ "start": start, "end": end, "tasks": tasks }));
    }
    Ok(json!({ "from": from, "weeks": out }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Schedule) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&ScheduleTask> = v.state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_some()).collect();
        return json(&bin);
    }
    json(&sched::rows(v.state, project, v.today()))
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ScheduleTask> = p.id("id")?;
    json(&live_task(v, project, id)?)
}
