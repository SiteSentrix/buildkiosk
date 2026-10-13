//! The project schedule: tasks with dates, links and progress.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Vendor;
use crate::project::Project;
use crate::record;

/// A finish-to-start link: the task can't start until `task` has finished, plus `lag` working
/// days (negative for an overlap).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub task: Id<ScheduleTask>,
    #[serde(default)]
    pub lag: i32,
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
