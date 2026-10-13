//! The schedule: tasks with working-day dates, finish-to-start links, progress and a baseline.
//!
//! Links are by task number. Summary tasks (those with children) roll up their children, and the
//! critical path is computed from the leaf tasks' links (see `sched`).

use std::collections::HashMap;

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::{Link, ScheduleTask};
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
            "name: text; parent?: parent task id (empty for top level); start: YYYY-MM-DD; finish?: YYYY-MM-DD; days?: working days (when finish is blank; default 1); milestone?: bool; percent?: 0-100; predecessors?: text such as `3, 5+2, 7-1` (task numbers, with lag in working days); company?: company id; notes?: text"
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
        "project: project id; text: lines of `id, name, start, finish, percent, predecessors, parent` (comma or tab separated; predecessors separated by ; and may carry a lag such as 4+2; ids are the file's own and are renumbered)",
        import,
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

/// `3, 5+2, 7-1` → links by task number.
fn parse_links(ctx: &Ctx<'_>, project: Id<Project>, key: &str, raw: &str, me: Option<Id<ScheduleTask>>, p: &Params<'_>) -> Result<Vec<Link>> {
    let mut out: Vec<Link> = Vec::new();
    for part in raw.split([',', ';']).map(str::trim).filter(|s| !s.is_empty()) {
        let (num, lag) = match part.find(['+', '-']) {
            Some(i) => {
                let (n, l) = part.split_at(i);
                (n.trim(), l.trim().parse::<i32>().map_err(|_| p.bad(key, format!("`{part}`: the lag must be a whole number of days")))?)
            }
            None => (part, 0),
        };
        let num: u32 = num.trim_start_matches('#').parse().map_err(|_| p.bad(key, format!("`{part}` is not a task number")))?;
        if !(-1_000..=1_000).contains(&lag) {
            return Err(p.bad(key, format!("`{part}`: the lag is out of range")));
        }
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
        out.push(Link { task: task.id, lag });
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

fn dates(p: &Params<'_>, current: Option<&ScheduleTask>) -> Result<(Date, Date, bool)> {
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
        sched::shift(start, days - 1).ok_or_else(|| p.bad("days", "runs past the supported dates"))?
    } else if let Some(t) = current {
        // Keep the task's length when only the start moved.
        if p.present("start") {
            let len = sched::days_inclusive(t.start, t.finish).max(1);
            sched::shift(start, len - 1).ok_or_else(|| p.bad("start", "runs past the supported dates"))?
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
    let (start, finish, milestone) = dates(p, None)?;
    let predecessors = if p.present("predecessors") {
        parse_links(ctx, project, "predecessors", &p.text_or_empty("predecessors", 500)?, None, p)?
    } else {
        Vec::new()
    };
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
        let (s, f, m) = dates(p, Some(&t))?;
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
    t.updated = ctx.stamp();
    ctx.state.schedule_tasks.put(t);
    Ok(json!({ "id": id }))
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
    // Repeat until nothing moves; every pass settles at least one more level of the chain.
    let mut moved: HashMap<Id<ScheduleTask>, ()> = HashMap::new();
    for _ in 0..=MAX_TASKS.min(200) {
        let tasks: Vec<ScheduleTask> = ctx.state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).cloned().collect();
        let by_id: HashMap<Id<ScheduleTask>, &ScheduleTask> = tasks.iter().map(|t| (t.id, t)).collect();
        let mut changed = false;
        for t in &tasks {
            let need = t.predecessors.iter().filter_map(|l| by_id.get(&l.task).and_then(|p| sched::earliest_start(p.finish, l.lag))).max();
            let Some(need) = need.filter(|n| *n > t.start) else { continue };
            let len = if t.milestone { 1 } else { sched::days_inclusive(t.start, t.finish).max(1) };
            let Some(finish) = sched::shift(need, len - 1) else { continue };
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
    struct Pending {
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
        rows.push(Pending {
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
            milestone: r.start == r.finish && r.name.to_lowercase().contains("milestone"),
            percent: r.percent,
            predecessors: Vec::new(),
            company: None,
            notes: String::new(),
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
            let (num, lag) = match part.find(['+', '-']) {
                Some(i) => {
                    let (n, l) = part.split_at(i);
                    (n.trim(), l.trim().parse::<i32>().map_err(|_| p.bad("text", format!("task {}: `{part}` has a bad lag", r.file_id)))?)
                }
                None => (part, 0),
            };
            let pred = *map.get(num).ok_or_else(|| p.bad("text", format!("task {}: predecessor {num} is not in the file", r.file_id)))?;
            if pred == *id || sched::reaches(ctx.state, pred, *id) {
                return Err(p.bad("text", format!("task {}: predecessor {num} would make a loop", r.file_id)));
            }
            if !links.iter().any(|l: &Link| l.task == pred) {
                links.push(Link { task: pred, lag });
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
