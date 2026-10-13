//! Tasks: to-do items with an owner and a due date, outside the punch list and observations.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::meeting::Meeting;
use crate::project::Project;
use crate::rfi::Rfi;
use crate::work::WorkCore;
use crate::{choice, record, work_deref};

pub use crate::work::WorkComment as TaskComment;
pub use crate::work::WorkStatus as TaskStatus;

choice! {
    /// How often a task comes round again once it is closed.
    #[derive(Default)]
    pub enum Repeat {
        #[default]
        Never = ("never", "Does not repeat"),
        Daily = ("daily", "Daily"),
        Weekly = ("weekly", "Weekly"),
        Biweekly = ("biweekly", "Every two weeks"),
        Monthly = ("monthly", "Monthly"),
        MonthEnd = ("month_end", "Last day of each month"),
        Quarterly = ("quarterly", "Every three months"),
        Yearly = ("yearly", "Yearly"),
    }
}

impl Repeat {
    /// The due date after `due`, one step on (monthly keeps the day of the month, or the month's last day).
    pub fn step(self, due: Date) -> Option<Date> {
        match self {
            Repeat::Never => None,
            Repeat::Daily => due.add_days(1),
            Repeat::Weekly => due.add_days(7),
            Repeat::Biweekly => due.add_days(14),
            Repeat::Monthly => due.add_months(1),
            Repeat::MonthEnd => due.month_start().add_months(2)?.add_days(-1),
            Repeat::Quarterly => due.add_months(3),
            Repeat::Yearly => due.add_months(12),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id<Task>,
    pub project: Id<Project>,
    #[serde(flatten)]
    pub core: WorkCore,
    /// The task this is a subtask of (one level only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id<Task>>,
    /// Closing the task makes the next one, due a step later.
    #[serde(default)]
    pub repeat: Repeat,
    /// No new occurrence is made once its due date would pass this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_until: Option<Date>,
    /// The occurrence made when this one was closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Id<Task>>,
    /// The RFI this task comes from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfi: Option<Id<Rfi>>,
    /// The meeting this task comes from (an action item).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<Id<Meeting>>,
}
record!(Task, "task", trash, project);
work_deref!(Task);

impl Task {
    pub fn label(&self) -> String {
        format!("Task #{}", self.number)
    }
}
