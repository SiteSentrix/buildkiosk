//! The project schedule: tasks with dates, links and progress.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Vendor;
use crate::project::Project;
use crate::{choice, record};

choice! {
    /// How a task depends on its predecessor.
    #[derive(Default)]
    pub enum LinkKind {
        /// The task can't start until the predecessor has finished (the usual one).
        #[default]
        FinishToStart = ("fs", "Finish to start"),
        /// The task can't start until the predecessor has started.
        StartToStart = ("ss", "Start to start"),
        /// The task can't finish until the predecessor has finished.
        FinishToFinish = ("ff", "Finish to finish"),
        /// The task can't finish until the predecessor has started.
        StartToFinish = ("sf", "Start to finish"),
    }
}

choice! {
    /// A date a task is held to, besides its links.
    #[derive(Default)]
    pub enum Constraint {
        /// Start as early as the links allow.
        #[default]
        None = ("none", "As soon as possible"),
        /// Never start before the date.
        StartNoEarlierThan = ("snet", "Start no earlier than"),
        /// Start on the date, whatever the links say.
        MustStartOn = ("mso", "Must start on"),
        /// A deadline: finish on or before the date.
        FinishNoLaterThan = ("fnlt", "Finish no later than"),
        /// Finish on the date, whatever the links say.
        MustFinishOn = ("mfo", "Must finish on"),
        /// Start on or before the date (a late start is flagged).
        StartNoLaterThan = ("snlt", "Start no later than"),
        /// Never finish before the date.
        FinishNoEarlierThan = ("fnet", "Finish no earlier than"),
        /// Start as late as the float allows without delaying anything after it.
        AsLateAsPossible = ("alap", "As late as possible"),
    }
}

impl Constraint {
    /// Whether the constraint is tied to a date (the first two kinds are not).
    pub fn needs_date(self) -> bool {
        !matches!(self, Constraint::None | Constraint::AsLateAsPossible)
    }
}

/// A crew or kind of worker a task needs: "Carpenter", 4 of them each working day.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub name: String,
    pub units: u32,
}

/// Most resources one task names, and most of one kind.
pub const MAX_ASSIGNMENTS: usize = 20;
pub const MAX_UNITS: u32 = 10_000;

/// A link to a predecessor: what it holds the task to depends on the `kind`, plus `lag` working
/// days (negative for an overlap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub task: Id<ScheduleTask>,
    #[serde(default)]
    pub lag: i32,
    #[serde(default)]
    pub kind: LinkKind,
}

/// A day off for the whole project, with what it is ("Labor Day").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holiday {
    pub date: Date,
    #[serde(default)]
    pub name: String,
}

/// A company-wide list of days off ("Federal holidays 2027") that projects subscribe to; each project's calendar
/// adds them to its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HolidayList {
    pub id: Id<HolidayList>,
    pub name: String,
    /// Oldest first, one per date.
    pub holidays: Vec<Holiday>,
    pub created: crate::common::Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<crate::common::Stamp>,
}
crate::record!(HolidayList, "holiday_list", trash);

/// The days a project's work is done on: which weekdays, minus the holidays. Schedule dates and
/// durations are counted in these days.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCalendar {
    /// Monday first: whether work is done on that weekday.
    pub days: [bool; 7],
    /// Oldest first, one per date.
    #[serde(default)]
    pub holidays: Vec<Holiday>,
}

impl Default for WorkCalendar {
    /// Monday to Friday.
    fn default() -> Self {
        WorkCalendar { days: [true, true, true, true, true, false, false], holidays: Vec::new() }
    }
}

/// Most holidays one calendar keeps.
pub const MAX_HOLIDAYS: usize = 400;

impl WorkCalendar {
    /// Whether work is done on `d`.
    pub fn is_working(&self, d: Date) -> bool {
        self.days.get(d.weekday().index()).copied().unwrap_or(false) && self.holiday(d).is_none()
    }

    /// Whether `d` is a listed day off (even on a weekend).
    pub fn holiday_on(&self, d: Date) -> bool {
        self.holiday(d).is_some()
    }

    pub fn holiday(&self, d: Date) -> Option<&Holiday> {
        self.holidays.binary_search_by_key(&d, |h| h.date).ok().and_then(|i| self.holidays.get(i))
    }

    /// How many days a week are worked (before holidays).
    pub fn days_per_week(&self) -> usize {
        self.days.iter().filter(|d| **d).count()
    }

    /// A calendar needs at least one working day a week.
    pub fn is_valid(&self) -> bool {
        self.days_per_week() > 0 && self.holidays.len() <= MAX_HOLIDAYS && self.holidays.windows(2).all(|w| matches!(w, [a, b] if a.date < b.date))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScheduleTask {
    pub id: Id<ScheduleTask>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1; what people call a task in links.
    pub number: u32,
    pub name: String,
    /// The summary task it belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id<ScheduleTask>>,
    pub start: Date,
    /// The last day of work, inclusive; equal to `start` for a milestone.
    pub finish: Date,
    #[serde(default)]
    pub milestone: bool,
    /// 0 to 100.
    #[serde(default)]
    pub percent: u8,
    #[serde(default)]
    pub predecessors: Vec<Link>,
    /// The company doing the work.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<Id<Vendor>>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub constraint: Constraint,
    /// The date of the constraint (none for "as soon as possible").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint_date: Option<Date>,
    /// What the task needs each working day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<Assignment>,
    /// The dates when the baseline was saved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_start: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_finish: Option<Date>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(ScheduleTask, "schedule_task", trash, project);
