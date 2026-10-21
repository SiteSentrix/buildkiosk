//! Resource levelling: delay tasks so that no limited resource is needed beyond its limit on any working day.
//!
//! The method is the usual serial one. Work that is finished, under way or pinned to a date (must start on, must finish
//! on) stays where it is and uses its resources first. The rest is placed one task at a time, earliest start first and
//! the least float first among equals, each at the earliest working day that its links allow and where its resources
//! fit under the limits. Tasks only move later, never earlier. Resources with no limit are not levelled.

use std::collections::{BTreeMap, HashMap, HashSet};

use buildkiosk_core::{Date, Id};
use buildkiosk_model::CompanyState;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::{Constraint, ScheduleTask, WorkCalendar};
use serde::Serialize;

use crate::sched;

/// How far past the day its links allow a task may be pushed to find room, in working days.
const MAX_PUSH: i64 = 1_500;
/// Most placements tried in one plan, so a large project can't hold the server.
const MAX_TRIALS: u64 = 3_000_000;

/// A task that moves later.
#[derive(Clone, Debug, Serialize)]
pub struct Move {
    pub id: Id<ScheduleTask>,
    pub number: u32,
    pub name: String,
    pub from_start: Date,
    pub from_finish: Date,
    pub start: Date,
    pub finish: Date,
    /// Working days later it starts.
    pub days: i64,
}

/// A task that stays over a limit, and why.
#[derive(Clone, Debug, Serialize)]
pub struct Unresolved {
    pub id: Id<ScheduleTask>,
    pub number: u32,
    pub name: String,
    pub resource: String,
    pub reason: String,
}

/// What levelling would do.
#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub moves: Vec<Move>,
    pub unresolved: Vec<Unresolved>,
    /// The last finish of any task before and after.
    pub finish_before: Option<Date>,
    pub finish_after: Option<Date>,
    /// Working days the project finish moves later.
    pub finish_slip: i64,
}

/// What the project may use on one working day, by lower-case resource name.
fn limits_of(state: &CompanyState, project: Id<Project>) -> HashMap<String, u32> {
    let mut out = HashMap::new();
    if let Some(p) = state.projects.get(project) {
        for a in &p.resource_limits {
            out.entry(a.name.to_lowercase()).or_insert(a.units);
        }
    }
    out
}

/// The working days of `cal` from `start`, `len` of them.
fn span(cal: &WorkCalendar, start: Date, len: i64) -> Vec<Date> {
    let mut out = Vec::new();
    let mut d = start;
    while i64::try_from(out.len()).unwrap_or(i64::MAX) < len {
        if cal.is_working(d) {
            out.push(d);
        }
        match d.add_days(1) {
            Some(n) => d = n,
            None => break,
        }
    }
    out
}

/// Adds what `t` needs on each of `days` to the day-by-day totals.
fn add_usage(usage: &mut BTreeMap<(Date, String), u32>, t: &ScheduleTask, days: &[Date]) {
    for a in &t.resources {
        for d in days {
            let e = usage.entry((*d, a.name.to_lowercase())).or_default();
            *e = e.saturating_add(a.units);
        }
    }
}

/// Works out the moves that bring the project's resources under their limits. With `within_float` a task is only
/// delayed as far as its float allows (so the project finish stays); otherwise it is pushed as far as it takes, and the
/// project finish may move.
///
/// Fails when the links form a loop or the project is too large to level.
pub fn plan(state: &CompanyState, project: Id<Project>, today: Date, within_float: bool) -> Result<Plan, String> {
    let cal = sched::calendar(state, project);
    let limits = limits_of(state, project);
    let float: HashMap<Id<ScheduleTask>, i64> =
        sched::rows(state, project, today).iter().filter(|r| !r.summary).filter_map(|r| Some((r.id, r.float?))).collect();

    let all: Vec<&ScheduleTask> = state.schedule_tasks.iter().filter(|t| t.project == project && t.deleted.is_none()).collect();
    let parents: HashSet<Id<ScheduleTask>> = all.iter().filter_map(|t| t.parent).collect();
    let mut leaves: Vec<&ScheduleTask> = all.into_iter().filter(|t| !parents.contains(&t.id)).collect();
    leaves.sort_by_key(|t| t.number);
    let leaf_ids: HashSet<Id<ScheduleTask>> = leaves.iter().map(|t| t.id).collect();

    // Where each task is, as the plan goes on.
    let mut pos: HashMap<Id<ScheduleTask>, (Date, Date)> = leaves.iter().map(|t| (t.id, (t.start, t.finish))).collect();
    let mut usage: BTreeMap<(Date, String), u32> = BTreeMap::new();
    // Finished work uses nothing; work under way or pinned to a date holds its place and its resources.
    let mut placed: HashSet<Id<ScheduleTask>> = HashSet::new();
    for t in &leaves {
        let pinned = matches!(t.constraint, Constraint::MustStartOn | Constraint::MustFinishOn);
        if t.percent >= 100 {
            placed.insert(t.id);
        } else if t.percent > 0 || pinned {
            let days = span(&cal, t.start, sched::days_inclusive_in(&cal, t.start, t.finish).max(1));
            add_usage(&mut usage, t, &days);
            placed.insert(t.id);
        }
    }

    let mut unresolved: Vec<Unresolved> = Vec::new();
    let mut trials: u64 = 0;
    let total = leaves.len();
    while placed.len() < total {
        // The tasks whose predecessors are all placed; the earliest start (then least float) goes first.
        let mut best: Option<(&ScheduleTask, Date, (Date, i64, u32))> = None;
        for t in leaves.iter().filter(|t| !placed.contains(&t.id)) {
            if t.predecessors.iter().any(|l| leaf_ids.contains(&l.task) && !placed.contains(&l.task)) {
                continue;
            }
            let len = if t.milestone { 1 } else { sched::days_inclusive_in(&cal, t.start, t.finish).max(1) };
            let by_links = t
                .predecessors
                .iter()
                .filter_map(|l| {
                    let (ps, pf) = pos.get(&l.task).copied().or_else(|| state.schedule_tasks.get(l.task).map(|x| (x.start, x.finish)))?;
                    sched::required_start(&cal, l, ps, pf, len)
                })
                .max();
            let want = by_links.map_or(t.start, |b| b.max(t.start));
            let Some(want) = sched::shift_in(&cal, want, 0) else { continue };
            let key = (want, float.get(&t.id).copied().unwrap_or(i64::MAX), t.number);
            if best.as_ref().is_none_or(|(_, _, k)| key < *k) {
                best = Some((t, want, key));
            }
        }
        let Some((t, want, _)) = best else { return Err("the links between the tasks form a loop".to_string()) };

        let len = if t.milestone { 1 } else { sched::days_inclusive_in(&cal, t.start, t.finish).max(1) };
        // Resources that have a limit are the ones that can be over it.
        let limited: Vec<(&str, u32, u32)> =
            t.resources.iter().filter_map(|a| limits.get(&a.name.to_lowercase()).map(|l| (a.name.as_str(), a.units, *l))).collect();
        let too_big = limited.iter().find(|(_, units, limit)| units > limit);
        let latest = if within_float {
            let allowed = float.get(&t.id).copied().unwrap_or(0).max(0);
            sched::shift_in(&cal, t.start, allowed).unwrap_or(t.start)
        } else {
            sched::shift_in(&cal, want, MAX_PUSH).unwrap_or(want)
        };

        let mut chosen = want;
        let mut fitted = limited.is_empty();
        if let Some((name, units, limit)) = too_big {
            unresolved.push(Unresolved {
                id: t.id,
                number: t.number,
                name: t.name.clone(),
                resource: (*name).to_string(),
                reason: format!("needs {units} a day, more than the limit of {limit}"),
            });
            fitted = true;
        } else if !fitted {
            let mut s = want;
            loop {
                if s > latest {
                    break;
                }
                trials += 1;
                if trials > MAX_TRIALS {
                    return Err("the project is too large to level in one go; give it a nearer finish or fewer limits".to_string());
                }
                let days = span(&cal, s, len);
                let ok = limited.iter().all(|(name, units, limit)| {
                    let key = name.to_lowercase();
                    days.iter().all(|d| usage.get(&(*d, key.clone())).copied().unwrap_or(0).saturating_add(*units) <= *limit)
                });
                if ok {
                    chosen = s;
                    fitted = true;
                    break;
                }
                match sched::shift_in(&cal, s, 1) {
                    Some(n) if n > s => s = n,
                    _ => break,
                }
            }
            if !fitted {
                let (name, _, limit) = limited.first().map(|(n, u, l)| (*n, *u, *l)).unwrap_or(("", 0, 0));
                unresolved.push(Unresolved {
                    id: t.id,
                    number: t.number,
                    name: t.name.clone(),
                    resource: name.to_string(),
                    reason: if within_float {
                        format!("no room under the limit of {limit} before its float runs out")
                    } else {
                        format!("no room under the limit of {limit} within {MAX_PUSH} working days")
                    },
                });
            }
        }
        let finish = if t.milestone { chosen } else { sched::shift_in(&cal, chosen, len - 1).unwrap_or(chosen) };
        pos.insert(t.id, (chosen, finish));
        let days = span(&cal, chosen, len);
        add_usage(&mut usage, t, &days);
        placed.insert(t.id);
    }

    let mut moves: Vec<Move> = leaves
        .iter()
        .filter_map(|t| {
            let (s, f) = pos.get(&t.id).copied()?;
            (s != t.start).then(|| Move {
                id: t.id,
                number: t.number,
                name: t.name.clone(),
                from_start: t.start,
                from_finish: t.finish,
                start: s,
                finish: f,
                days: if s > t.start { sched::days_inclusive_in(&cal, t.start, s).saturating_sub(1) } else { 0 },
            })
        })
        .collect();
    moves.sort_by_key(|m| (m.start, m.number));
    let unfinished = |t: &&&ScheduleTask| t.percent < 100;
    let finish_before = leaves.iter().filter(unfinished).map(|t| t.finish).max();
    let finish_after = leaves.iter().filter(unfinished).filter_map(|t| pos.get(&t.id).map(|p| p.1)).max();
    let finish_slip = match (finish_before, finish_after) {
        (Some(a), Some(b)) if b > a => sched::days_inclusive_in(&cal, a, b).saturating_sub(1),
        _ => 0,
    };
    Ok(Plan { moves, unresolved, finish_before, finish_after, finish_slip })
}
