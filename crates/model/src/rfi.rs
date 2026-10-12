//! Requests for information.

use buildkiosk_core::{Date, Id, Money, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Impact, Stamp};
use crate::directory::{Person, Vendor};
use crate::project::Project;
use crate::{choice, record};

choice! {
    /// Where an RFI is in its workflow.
    #[derive(Default)]
    pub enum RfiStatus {
        /// Being written; only its author and RFI admins see it.
        #[default]
        Draft = ("draft", "Draft"),
        /// Sent to the assignees for an answer.
        Open = ("open", "Open"),
        /// Answered and closed by the RFI manager.
        Closed = ("closed", "Closed"),
    }
}

/// A question to the design team (or anyone) that needs a documented answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rfi {
    pub id: Id<Rfi>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub subject: String,
    pub question: String,
    #[serde(default)]
    pub status: RfiStatus,
    /// Runs the RFI: opens it, accepts the official answer, closes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    /// Who is asked to answer.
    #[serde(default)]
    pub assignees: Vec<Id<Person>>,
    /// Who has to act next: the assignees who haven't answered, else the manager.
    #[serde(default)]
    pub ball_in_court: Vec<Id<Person>>,
    /// Who raised the question (often a subcontractor).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_from: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsible_vendor: Option<Id<Vendor>>,
    /// People copied on it.
    #[serde(default)]
    pub distribution: Vec<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Stamp>,
    #[serde(default)]
    pub spec_section: String,
    #[serde(default)]
    pub drawing_number: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub cost_impact: Impact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_amount: Option<Money>,
    #[serde(default)]
    pub schedule_impact: Impact,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_days: Option<i32>,
    /// Only the author, the manager, assignees, distribution and RFI admins can see it.
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default)]
    pub responses: Vec<RfiResponse>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Rfi, "rfi", trash, project);

/// An answer to an RFI. The manager marks the one that settles the question as official.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RfiResponse {
    /// Sequential within the RFI.
    pub id: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub body: String,
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
}

impl Rfi {
    /// "RFI #12".
    pub fn label(&self) -> String {
        format!("RFI #{}", self.number)
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.status == RfiStatus::Open && self.due.is_some_and(|d| d < today)
    }

    /// Whether `person` is part of the RFI (sees it even when it's private).
    pub fn involves(&self, person: Id<Person>) -> bool {
        self.created.by == Some(person)
            || self.manager == Some(person)
            || self.received_from == Some(person)
            || self.assignees.contains(&person)
            || self.distribution.contains(&person)
    }

    pub fn official_response(&self) -> Option<&RfiResponse> {
        self.responses.iter().find(|r| r.official)
    }

    /// Recomputes who has to act: while open, the assignees without an answer; once all have
    /// answered (or there are none), the manager. Nobody when draft or closed.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            RfiStatus::Draft | RfiStatus::Closed => Vec::new(),
            RfiStatus::Open => {
                let waiting: Vec<Id<Person>> = self.assignees.iter().copied().filter(|a| !self.responses.iter().any(|r| r.by == Some(*a))).collect();
                if waiting.is_empty() { self.manager.into_iter().collect() } else { waiting }
            }
        };
    }
}
