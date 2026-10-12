//! Observations: safety, quality and housekeeping findings made on site.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::project::Project;
use crate::work::WorkCore;
use crate::{record, work_deref};

pub use crate::work::WorkComment as ObservationComment;
pub use crate::work::WorkStatus as ObservationStatus;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: Id<Observation>,
    pub project: Id<Project>,
    #[serde(flatten)]
    pub core: WorkCore,
    /// The day it was seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Date>,
    /// What will be done about it.
    #[serde(default)]
    pub corrective_action: String,
}
record!(Observation, "observation", trash, project);
work_deref!(Observation);

impl Observation {
    pub fn label(&self) -> String {
        format!("Observation #{}", self.number)
    }
}
