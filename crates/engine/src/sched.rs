//! Schedule arithmetic in working days (Monday to Friday) and the critical path.

use std::collections::HashMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_model::CompanyState;
use buildkiosk_model::choice::Choice;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::{Link, LinkKind, ScheduleTask, WorkCalendar};
use serde::Serialize;

/// Summary tasks nest at most this deep.
pub const MAX_DEPTH: usize = 6;
/// Most tasks in one project's schedule.
pub const MAX_TASKS: usize = 5_000;

/// The most working days one move may cover.
const MAX_SHIFT: u64 = 50_000;

/// Moves `from` by `n` working days of `cal` (negative goes back). A start on a day off first moves to the
/// nearest working day in the direction of travel.
pub fn shift_in(cal: &WorkCalendar, from: Date, n: i64) -> Option<Date> {
    let step: i64 = if n >= 0 { 1 } else { -1 };
    let mut left = n.unsigned_abs();
    if left > MAX_SHIFT || !cal.is_valid() {
        return None;
    }
    let mut d = from;
    // Holidays can't make more than a few hundred days off in a row; stop rather than loop on bad data.
    let mut guard = 0;
    while !cal.is_working(d) {
        d = d.add_days(step)?;
        guard += 1;
        if guard > 1_000 {
            return None;
        }
    }
    // Whole weeks in one step when there are no holidays to trip over.
    let per_week = u64::try_from(cal.days_per_week()).unwrap_or(1).max(1);
    if cal.holidays.is_empty() && left >= per_week {
        d = d.add_days(step.checked_mul(i64::try_from(left / per_week).ok()?.checked_mul(7)?)?)?;
        left %= per_week;
    }
    while left > 0 {
        d = d.add_days(step)?;
        if cal.is_working(d) {
            left -= 1;
        }
    }
    Some(d)
}

/// Working days of `cal` from `a` to `b` inclusive of both ends (1 for the same working day); 0 when `b` is
/// before `a`.
pub fn days_inclusive_in(cal: &WorkCalendar, a: Date, b: Date) -> i64 {
    if b < a {
        return 0;
    }
    let total = a.days_until(b).saturating_add(1);
    let (weeks, rest) = (total / 7, total % 7);
    let mut n = weeks * i64::try_from(cal.days_per_week()).unwrap_or(0);
    for i in 0..rest {
        if let Some(d) = a.add_days(weeks * 7 + i)
            && cal.days.get(d.weekday().index()).copied().unwrap_or(false)
        {
            n += 1;
        }
    }
    // Holidays that fall on days that would have been worked.
    let from = cal.holidays.partition_point(|h| h.date < a);
    let to = cal.holidays.partition_point(|h| h.date <= b);
    let off =
        cal.holidays.get(from..to).unwrap_or_default().iter().filter(|h| cal.days.get(h.date.weekday().index()).copied().unwrap_or(false)).count();
    n - i64::try_from(off).unwrap_or(0)
}

/// [`shift_in`] for the usual Monday to Friday week with no holidays.
pub fn shift(from: Date, n: i64) -> Option<Date> {
    shift_in(&WorkCalendar::default(), from, n)
}

/// [`days_inclusive_in`] for the usual Monday to Friday week with no holidays.
pub fn days_inclusive(a: Date, b: Date) -> i64 {
    days_inclusive_in(&WorkCalendar::default(), a, b)
}

/// The calendar of a project (the usual week when it has none or the project is unknown).
pub fn calendar(state: &CompanyState, project: Id<Project>) -> WorkCalendar {
    state.projects.get(project).map(|p| p.calendar.clone()).filter(WorkCalendar::is_valid).unwrap_or_default()
}

/// How long a task is, in working days, for working out its links (a milestone counts as one day).
fn length_of(cal: &WorkCalendar, t: &ScheduleTask) -> i64 {
    if t.milestone { 1 } else { days_inclusive_in(cal, t.start, t.finish).max(1) }
}

/// The earliest day a task of `len` working days may start so that `link` to a predecessor spanning
/// `pred_start` to `pred_finish` holds.
///
/// Finish to start: the day `lag` working days after the predecessor finishes. Start to start:
/// `lag` working days after it starts. Finish to finish: finishes `lag` working days after it
/// finishes. Start to finish: finishes `lag` working days after it starts.
pub fn required_start(cal: &WorkCalendar, link: &Link, pred_start: Date, pred_finish: Date, len: i64) -> Option<Date> {
    let lag = i64::from(link.lag);
    match link.kind {
        LinkKind::FinishToStart => shift_in(cal, pred_finish, 1i64.saturating_add(lag)),
        LinkKind::StartToStart => shift_in(cal, pred_start, lag),
        LinkKind::FinishToFinish => shift_in(cal, shift_in(cal, pred_finish, lag)?, -(len - 1)),
        LinkKind::StartToFinish => shift_in(cal, shift_in(cal, pred_start, lag)?, -(len - 1)),
    }
}

/// The earliest start the links of `t` allow, given the other tasks.
pub fn earliest_by_links(cal: &WorkCalendar, t: &ScheduleTask, by_id: &HashMap<Id<ScheduleTask>, &ScheduleTask>) -> Option<Date> {
    let len = length_of(cal, t);
    t.predecessors.iter().filter_map(|l| by_id.get(&l.task).and_then(|p| required_start(cal, l, p.start, p.finish, len))).max()
}

/// `3`, `5+2`, `7SS`, `4FF-1`: a task number, an optional kind (FS, SS, FF, SF; default FS) and an optional lag in working days.
pub fn parse_link(part: &str) -> std::result::Result<(u32, LinkKind, i32), String> {
    let part = part.trim().trim_start_matches('#');
    let digits_end = part.find(|c: char| !c.is_ascii_digit()).unwrap_or(part.len());
    let (num, rest) = part.split_at(digits_end);
    let num: u32 = num.parse().map_err(|_| format!("`{part}` is not a task number"))?;
    let rest = rest.trim_start();
    let kind_end = rest.find(|c: char| !c.is_ascii_alphabetic()).unwrap_or(rest.len());
    let (kind, lag) = rest.split_at(kind_end);
    let kind = if kind.is_empty() {
        LinkKind::FinishToStart
    } else {
        LinkKind::parse(kind).ok_or_else(|| format!("`{part}`: the kind must be FS, SS, FF or SF"))?
    };
    let lag = lag.trim();
    let lag = if lag.is_empty() {
        0
    } else if lag.starts_with(['+', '-']) {
        lag.replace(' ', "").parse::<i32>().map_err(|_| format!("`{part}`: the lag must be a whole number of days"))?
    } else {
        return Err(format!("`{part}`: expected a lag such as +2 or -1"));
    };
    if !(-1_000..=1_000).contains(&lag) {
        return Err(format!("`{part}`: the lag is out of range"));
    }
    Ok((num, kind, lag))
}

/// A link as people write it: `5`, `5+2`, `5SS`, `5FF-1`.
pub fn format_link(number: u32, kind: LinkKind, lag: i32) -> String {
    let kind = if kind == LinkKind::FinishToStart { String::new() } else { kind.key().to_uppercase() };
    match lag {
        0 => format!("{number}{kind}"),
        l if l > 0 => format!("{number}{kind}+{l}"),
        l => format!("{number}{kind}{l}"),
    }
}

/// One task as the schedule shows it.
#[derive(Clone, Debug, Serialize)]
pub struct Row {
    pub id: Id<ScheduleTask>,
    pub number: u32,
    pub name: String,
    pub depth: usize,
    /// Has child tasks; its dates and progress are rolled up from them.
    pub summary: bool,
    pub milestone: bool,
    pub start: Date,
    pub finish: Date,
    pub percent: u8,
    /// Working days (0 for a milestone).
    pub duration: i64,
    /// Working days the task can slip without moving the project finish; leaf tasks only.
    pub float: Option<i64>,
    pub critical: bool,
    /// Finish date has passed and the task is not complete.
    pub late: bool,
    /// Starts before a predecessor allows.
    pub conflict: bool,
    /// Days the finish has moved from the baseline (positive is later).
    pub slip: Option<i64>,
}

fn live(state: &CompanyState, project: Id<Project>) -> Vec<&ScheduleTask> {
    state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).collect()
}

/// Every live task in tree order with its computed fields. `today` marks late tasks.
pub fn rows(state: &CompanyState, project: Id<Project>, today: Date) -> Vec<Row> {
    let cal = calendar(state, project);
    let tasks = live(state, project);
    let by_id: HashMap<Id<ScheduleTask>, &ScheduleTask> = tasks.iter().map(|t| (t.id, *t)).collect();
    let mut children: HashMap<Option<Id<ScheduleTask>>, Vec<&ScheduleTask>> = HashMap::new();
    for t in &tasks {
        // A parent that is deleted or missing makes the task top-level.
        let parent = t.parent.filter(|p| by_id.contains_key(p));
        children.entry(parent).or_default().push(t);
    }
    for list in children.values_mut() {
        list.sort_by_key(|t| (t.start, t.number));
    }
    // Leaf float by the critical path method over the links.
    let leaves: Vec<&ScheduleTask> = tasks.iter().copied().filter(|t| !children.contains_key(&Some(t.id))).collect();
    let floats = float_by_task(&cal, &leaves);

    let mut out = Vec::new();
    struct Walk<'a> {
        cal: &'a WorkCalendar,
        children: &'a HashMap<Option<Id<ScheduleTask>>, Vec<&'a ScheduleTask>>,
        by_id: &'a HashMap<Id<ScheduleTask>, &'a ScheduleTask>,
        floats: &'a HashMap<Id<ScheduleTask>, i64>,
        today: Date,
    }
    fn walk(w: &Walk<'_>, parent: Option<Id<ScheduleTask>>, depth: usize, out: &mut Vec<Row>) -> Option<(Date, Date, i64, i64)> {
        // Returns the span (start, finish) and (weighted percent, weight) of what it walked.
        let mut span: Option<(Date, Date)> = None;
        let (mut weighted, mut weight) = (0i64, 0i64);
        for t in w.children.get(&parent).map(Vec::as_slice).unwrap_or_default() {
            let at = out.len();
            let is_summary = w.children.contains_key(&Some(t.id));
            out.push(row_of(w.cal, t, depth, is_summary, w.floats, w.by_id, w.today));
            let (start, finish, w_pct, wt) = if is_summary && depth < MAX_DEPTH + 2 {
                match walk(w, Some(t.id), depth + 1, out) {
                    Some((s, f, p, wt)) => {
                        if let Some(r) = out.get_mut(at) {
                            r.start = s;
                            r.finish = f;
                            r.duration = days_inclusive_in(w.cal, s, f);
                            r.percent = u8::try_from(if wt > 0 { p / wt } else { 0 }).unwrap_or(0).min(100);
                            r.late = f < w.today && r.percent < 100;
                        }
                        (s, f, p, wt)
                    }
                    None => (t.start, t.finish, i64::from(t.percent), 1),
                }
            } else {
                let wt = days_inclusive_in(w.cal, t.start, t.finish).max(1);
                (t.start, t.finish, i64::from(t.percent) * wt, wt)
            };
            span = Some(match span {
                Some((s, f)) => (s.min(start), f.max(finish)),
                None => (start, finish),
            });
            weighted += w_pct;
            weight += wt;
        }
        span.map(|(s, f)| (s, f, weighted, weight))
    }
    walk(&Walk { cal: &cal, children: &children, by_id: &by_id, floats: &floats, today }, None, 0, &mut out);
    out
}

fn row_of(
    cal: &WorkCalendar,
    t: &ScheduleTask,
    depth: usize,
    summary: bool,
    floats: &HashMap<Id<ScheduleTask>, i64>,
    by_id: &HashMap<Id<ScheduleTask>, &ScheduleTask>,
    today: Date,
) -> Row {
    let float = if summary { None } else { floats.get(&t.id).copied() };
    let conflict = !summary && earliest_by_links(cal, t, by_id).is_some_and(|e| t.start < e);
    Row {
        id: t.id,
        number: t.number,
        name: t.name.clone(),
        depth,
        summary,
        milestone: t.milestone,
        start: t.start,
        finish: t.finish,
        percent: t.percent,
        duration: if t.milestone { 0 } else { days_inclusive_in(cal, t.start, t.finish) },
        float,
        critical: float.is_some_and(|f| f <= 0),
        late: t.finish < today && t.percent < 100,
        conflict,
        slip: t.baseline_finish.map(|b| b.days_until(t.finish)),
    }
}

/// Total float in working days for each task: how far the finish can move before it pushes the
/// project's last finish. Links to tasks outside `leaves` are ignored.
fn float_by_task(cal: &WorkCalendar, leaves: &[&ScheduleTask]) -> HashMap<Id<ScheduleTask>, i64> {
    let ids: HashMap<Id<ScheduleTask>, &ScheduleTask> = leaves.iter().map(|t| (t.id, *t)).collect();
    let Some(end) = leaves.iter().map(|t| t.finish).max() else { return HashMap::new() };
    // The links that follow each task: (the successor, the link).
    let mut succ: HashMap<Id<ScheduleTask>, Vec<(Id<ScheduleTask>, Link)>> = HashMap::new();
    for t in leaves {
        for l in &t.predecessors {
            if ids.contains_key(&l.task) {
                succ.entry(l.task).or_default().push((t.id, *l));
            }
        }
    }
    // Latest finish by a backward pass in order of decreasing finish date (a successor rarely
    // finishes before its predecessor when the dates are consistent; for inconsistent ones the
    // repeats below converge anyway).
    let mut order: Vec<&&ScheduleTask> = leaves.iter().collect();
    order.sort_by_key(|t| std::cmp::Reverse((t.finish, t.start, t.number)));
    let mut late_finish: HashMap<Id<ScheduleTask>, Date> = HashMap::new();
    for _ in 0..5 {
        for t in &order {
            let len = length_of(cal, t);
            let mut lf = end;
            for (s, link) in succ.get(&t.id).map(Vec::as_slice).unwrap_or_default() {
                let Some(st) = ids.get(s) else { continue };
                let s_lf = late_finish.get(s).copied().unwrap_or(end);
                // The successor's latest start, from its latest finish.
                let Some(s_ls) = shift_in(cal, s_lf, -(length_of(cal, st) - 1)) else { continue };
                let lag = i64::from(link.lag);
                // The latest this task may finish for the link to hold.
                let cand = match link.kind {
                    LinkKind::FinishToStart => shift_in(cal, s_ls, -(1i64.saturating_add(lag))),
                    LinkKind::FinishToFinish => shift_in(cal, s_lf, -lag),
                    LinkKind::StartToStart => shift_in(cal, s_ls, -lag).and_then(|start| shift_in(cal, start, len - 1)),
                    LinkKind::StartToFinish => shift_in(cal, s_lf, -lag).and_then(|start| shift_in(cal, start, len - 1)),
                };
                if let Some(cand) = cand {
                    lf = lf.min(cand);
                }
            }
            late_finish.insert(t.id, lf);
        }
    }
    leaves
        .iter()
        .map(|t| {
            let lf = late_finish.get(&t.id).copied().unwrap_or(end);
            let float = if lf >= t.finish { days_inclusive_in(cal, t.finish, lf) - 1 } else { -(days_inclusive_in(cal, lf, t.finish) - 1) };
            (t.id, float)
        })
        .collect()
}

/// Whether `from` can reach `target` by following predecessor links (used to refuse a cycle).
pub fn reaches(state: &CompanyState, from: Id<ScheduleTask>, target: Id<ScheduleTask>) -> bool {
    let mut stack = vec![from];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if id == target {
            return true;
        }
        if !seen.insert(id) || seen.len() > MAX_TASKS + 1 {
            continue;
        }
        if let Some(t) = state.schedule_tasks.get(id) {
            stack.extend(t.predecessors.iter().map(|l| l.task));
        }
    }
    false
}
