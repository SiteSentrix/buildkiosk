//! Timesheets: hours people worked on a project, by day, submitted for approval.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::project::{CostCode, Project};
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum TimeStatus {
        /// Still being filled in, or sent back for changes.
        #[default]
        Draft = ("draft", "Draft"),
        Submitted = ("submitted", "Submitted"),
        Approved = ("approved", "Approved"),
    }
}

/// One person's hours on one day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TimeEntry {
    pub id: Id<TimeEntry>,
    pub project: Id<Project>,
    pub person: Id<Person>,
    pub date: Date,
    /// Tenths of an hour: 75 is seven and a half hours.
    pub tenths: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_code: Option<Id<CostCode>>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: TimeStatus,
    /// Why an approver sent it back.
    #[serde(default)]
    pub returned_note: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(TimeEntry, "time_entry", trash, project);
