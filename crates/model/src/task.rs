//! Tasks: to-do items with an owner and a due date, outside the punch list and observations.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::work::WorkCore;
use crate::{record, work_deref};

pub use crate::work::WorkComment as TaskComment;
pub use crate::work::WorkStatus as TaskStatus;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id<Task>,
    pub project: Id<Project>,
    #[serde(flatten)]
    pub core: WorkCore,
}
record!(Task, "task", trash, project);
work_deref!(Task);

impl Task {
    pub fn label(&self) -> String {
        format!("Task #{}", self.number)
    }
}
