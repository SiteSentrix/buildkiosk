//! Observations: small items of work to fix or finish before a project closes out.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Priority, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum ObservationStatus {
        #[default]
        Open = ("open", "Open"),
        /// The responsible person says the work is done; the manager still has to accept it.
        ReadyForReview = ("ready_for_review", "Ready for review"),
        Closed = ("closed", "Closed"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObservationComment {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Observation {
    pub id: Id<Observation>,
    pub project: Id<Project>,
    /// Numbered within the project.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Free text: the category (safety, quality, housekeeping…).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub priority: Priority,
    /// The day it was seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Date>,
    /// What will be done about it.
    #[serde(default)]
    pub corrective_action: String,
    #[serde(default)]
    pub status: ObservationStatus,
    /// Accepts the work and closes the item.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    /// The people who have to do the work.
    #[serde(default)]
    pub assignees: Vec<Id<Person>>,
    #[serde(default)]
    pub ball_in_court: Vec<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_done: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Stamp>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default)]
    pub comments: Vec<ObservationComment>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Observation, "observation", trash, project);

impl Observation {
    pub fn label(&self) -> String {
        format!("Observation #{}", self.number)
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.status != ObservationStatus::Closed && self.due.is_some_and(|d| d < today)
    }

    /// The assignees while work is open (the manager if nobody is assigned); the manager once
    /// the work is ready for review.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            ObservationStatus::Closed => Vec::new(),
            ObservationStatus::ReadyForReview => self.manager.into_iter().collect(),
            ObservationStatus::Open if self.assignees.is_empty() => self.manager.into_iter().collect(),
            ObservationStatus::Open => self.assignees.clone(),
        };
    }
}
