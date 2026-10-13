//! Schedule arithmetic in working days (Monday to Friday) and the critical path.

use std::collections::HashMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_model::CompanyState;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::ScheduleTask;
use serde::Serialize;

/// Summary tasks nest at most this deep.
pub const MAX_DEPTH: usize = 6;
/// Most tasks in one project's schedule.
pub const MAX_TASKS: usize = 5_000;

/// Moves `from` by `n` working days (negative goes back). A weekend start first moves to the nearest
/// working day in the direction of travel.
pub fn shift(from: Date, n: i64) -> Option<Date> {
    if n >= 0 {
        return from.add_business_days(u32::try_from(n).ok()?);
    }
    let mut d = from;
    while d.weekday().is_weekend() {
        d = d.add_days(-1)?;
    }
    let m = n.checked_neg()?;
    d = d.add_days(-(m / 5).checked_mul(7)?)?;
    let mut left = m % 5;
    while left > 0 {
        d = d.add_days(-1)?;
        if !d.weekday().is_weekend() {
            left -= 1;
        }
    }
    Some(d)
}

/// Working days from `a` to `b` inclusive of both ends (1 for the same working day); 0 when `b` is
/// before `a`.
pub fn days_inclusive(a: Date, b: Date) -> i64 {
    if b < a {
        return 0;
    }
    let total = a.days_until(b).saturating_add(1);
    let (weeks, rest) = (total / 7, total % 7);
    let mut n = weeks * 5;
    for i in 0..rest {
        if let Some(d) = a.add_days(weeks * 7 + i)
            && !d.weekday().is_weekend()
        {
            n += 1;
        }
    }
    n
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

/// The earliest start a link allows.
pub fn earliest_start(pred_finish: Date, lag: i32) -> Option<Date> {
    shift(pred_finish, 1i64.saturating_add(i64::from(lag)))
}

/// Every live task in tree order with its computed fields. `today` marks late tasks.
pub fn rows(state: &CompanyState, project: Id<Project>, today: Date) -> Vec<Row> {
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
    // Leaf float by the critical path method over finish-to-start links.
    let leaves: Vec<&ScheduleTask> = tasks.iter().copied().filter(|t| !children.contains_key(&Some(t.id))).collect();
    let floats = float_by_task(&leaves);

    let mut out = Vec::new();
    fn walk(
        parent: Option<Id<ScheduleTask>>,
        depth: usize,
        children: &HashMap<Option<Id<ScheduleTask>>, Vec<&ScheduleTask>>,
        by_id: &HashMap<Id<ScheduleTask>, &ScheduleTask>,
        floats: &HashMap<Id<ScheduleTask>, i64>,
        today: Date,
        out: &mut Vec<Row>,
    ) -> Option<(Date, Date, i64, i64)> {
        // Returns the span (start, finish) and (weighted percent, weight) of what it walked.
        let mut span: Option<(Date, Date)> = None;
        let (mut weighted, mut weight) = (0i64, 0i64);
        for t in children.get(&parent).map(Vec::as_slice).unwrap_or_default() {
            let at = out.len();
            let is_summary = children.contains_key(&Some(t.id));
            out.push(row_of(t, depth, is_summary, floats, by_id, today));
            let (start, finish, w_pct, w) = if is_summary && depth < MAX_DEPTH + 2 {
                match walk(Some(t.id), depth + 1, children, by_id, floats, today, out) {
                    Some((s, f, p, w)) => {
                        if let Some(r) = out.get_mut(at) {
                            r.start = s;
                            r.finish = f;
                            r.duration = days_inclusive(s, f);
                            r.percent = u8::try_from(if w > 0 { p / w } else { 0 }).unwrap_or(0).min(100);
                            r.late = f < today && r.percent < 100;
                        }
                        (s, f, p, w)
                    }
                    None => (t.start, t.finish, i64::from(t.percent), 1),
                }
            } else {
                let w = days_inclusive(t.start, t.finish).max(1);
                (t.start, t.finish, i64::from(t.percent) * w, w)
            };
            span = Some(match span {
                Some((s, f)) => (s.min(start), f.max(finish)),
                None => (start, finish),
            });
            weighted += w_pct;
            weight += w;
        }
        span.map(|(s, f)| (s, f, weighted, weight))
    }
    walk(None, 0, &children, &by_id, &floats, today, &mut out);
    out
}

fn row_of(
    t: &ScheduleTask,
    depth: usize,
    summary: bool,
    floats: &HashMap<Id<ScheduleTask>, i64>,
    by_id: &HashMap<Id<ScheduleTask>, &ScheduleTask>,
    today: Date,
) -> Row {
    let float = if summary { None } else { floats.get(&t.id).copied() };
    let conflict =
        !summary && t.predecessors.iter().any(|l| by_id.get(&l.task).is_some_and(|p| earliest_start(p.finish, l.lag).is_some_and(|e| t.start < e)));
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
        duration: if t.milestone { 0 } else { days_inclusive(t.start, t.finish) },
        float,
        critical: float.is_some_and(|f| f <= 0),
        late: t.finish < today && t.percent < 100,
        conflict,
        slip: t.baseline_finish.map(|b| b.days_until(t.finish)),
    }
}

/// Total float in working days for each task: how far the finish can move before it pushes the
/// project's last finish. Links to tasks outside `leaves` are ignored.
fn float_by_task(leaves: &[&ScheduleTask]) -> HashMap<Id<ScheduleTask>, i64> {
    let ids: HashMap<Id<ScheduleTask>, &ScheduleTask> = leaves.iter().map(|t| (t.id, *t)).collect();
    let Some(end) = leaves.iter().map(|t| t.finish).max() else { return HashMap::new() };
    // Successors of each task.
    let mut succ: HashMap<Id<ScheduleTask>, Vec<(Id<ScheduleTask>, i32)>> = HashMap::new();
    for t in leaves {
        for l in &t.predecessors {
            if ids.contains_key(&l.task) {
                succ.entry(l.task).or_default().push((t.id, l.lag));
            }
        }
    }
    // Latest finish by a backward pass in order of decreasing finish date (a successor never
    // finishes before its predecessor when the dates are consistent; for inconsistent ones the
    // repeat below converges anyway within the task count).
    let mut order: Vec<&&ScheduleTask> = leaves.iter().collect();
    order.sort_by_key(|t| std::cmp::Reverse((t.finish, t.start, t.number)));
    let mut late_finish: HashMap<Id<ScheduleTask>, Date> = HashMap::new();
    for _ in 0..3 {
        for t in &order {
            let mut lf = end;
            for (s, lag) in succ.get(&t.id).map(Vec::as_slice).unwrap_or_default() {
                let Some(st) = ids.get(s) else { continue };
                let s_lf = late_finish.get(s).copied().unwrap_or(end);
                // The successor's latest start, then back over the lag and the day between.
                let dur = if st.milestone { 1 } else { days_inclusive(st.start, st.finish).max(1) };
                let Some(s_ls) = shift(s_lf, -(dur - 1)) else { continue };
                if let Some(cand) = shift(s_ls, -(1i64.saturating_add(i64::from(*lag)))) {
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
            let float = if lf >= t.finish { days_inclusive(t.finish, lf) - 1 } else { -(days_inclusive(lf, t.finish) - 1) };
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
