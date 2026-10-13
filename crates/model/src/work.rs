//! What punch items and observations have in common: an item of work with a manager, people
//! assigned to do it, a review step and a comment thread.
//!
//! Both records embed a [`WorkCore`] (flattened, so the stored fields stay where they always were)
//! and reach it through `Deref`; the engine's `WorkItem` trait runs the shared workflow over it.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::choice;
use crate::common::{FileRef, Priority, Stamp};
use crate::directory::Person;

choice! {
    #[derive(Default)]
    pub enum WorkStatus {
        #[default]
        Open = ("open", "Open"),
        /// The responsible person says the work is done; the manager still has to accept it.
        ReadyForReview = ("ready_for_review", "Ready for review"),
        Closed = ("closed", "Closed"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkComment {
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub body: String,
}

/// The shared fields.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkCore {
    /// Numbered within the project.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub location: String,
    /// The drawing sheet number it refers to ("A-101"); the sheet's page lists it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub drawing_number: String,
    /// Free text: the trade, category or kind of work.
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub status: WorkStatus,
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
    pub comments: Vec<WorkComment>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}

impl WorkCore {
    pub fn is_overdue(&self, today: Date) -> bool {
        self.status != WorkStatus::Closed && self.due.is_some_and(|d| d < today)
    }

    /// The assignees while work is open (the manager if nobody is assigned); the manager once
    /// the work is ready for review.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            WorkStatus::Closed => Vec::new(),
            WorkStatus::ReadyForReview => self.manager.into_iter().collect(),
            WorkStatus::Open if self.assignees.is_empty() => self.manager.into_iter().collect(),
            WorkStatus::Open => self.assignees.clone(),
        };
    }
}

/// Gives a record `Deref` and `DerefMut` to its embedded `core`.
#[macro_export]
macro_rules! work_deref {
    ($ty:ty) => {
        impl ::std::ops::Deref for $ty {
            type Target = $crate::work::WorkCore;
            fn deref(&self) -> &Self::Target {
                &self.core
            }
        }
        impl ::std::ops::DerefMut for $ty {
            fn deref_mut(&mut self) -> &mut Self::Target {
                &mut self.core
            }
        }
    };
}
