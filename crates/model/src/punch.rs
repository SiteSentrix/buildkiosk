//! Punch list: small items of work to fix or finish before a project closes out.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Priority, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum PunchStatus {
        #[default]
        Open = ("open", "Open"),
        /// The responsible person says the work is done; the manager still has to accept it.
        ReadyForReview = ("ready_for_review", "Ready for review"),
        Closed = ("closed", "Closed"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PunchComment {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PunchItem {
    pub id: Id<PunchItem>,
    pub project: Id<Project>,
    /// Numbered within the project.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// Free text: the trade or kind of work (paint, electrical…).
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub status: PunchStatus,
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
    pub comments: Vec<PunchComment>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PunchItem, "punch_item", trash, project);

impl PunchItem {
    pub fn label(&self) -> String {
        format!("Punch item #{}", self.number)
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.status != PunchStatus::Closed && self.due.is_some_and(|d| d < today)
    }

    /// The assignees while work is open (the manager if nobody is assigned); the manager once
    /// the work is ready for review.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            PunchStatus::Closed => Vec::new(),
            PunchStatus::ReadyForReview => self.manager.into_iter().collect(),
            PunchStatus::Open if self.assignees.is_empty() => self.manager.into_iter().collect(),
            PunchStatus::Open => self.assignees.clone(),
        };
    }
}
