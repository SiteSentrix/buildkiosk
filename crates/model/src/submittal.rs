//! Submittals: documents a contractor sends for review before work proceeds.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum SubmittalStatus {
        #[default]
        Draft = ("draft", "Draft"),
        InReview = ("in_review", "In review"),
        Closed = ("closed", "Closed"),
    }
}

choice! {
    /// A reviewer's answer, and the final answer on a closed submittal.
    pub enum Decision {
        Approved = ("approved", "Approved"),
        ApprovedAsNoted = ("approved_as_noted", "Approved as noted"),
        ReviseResubmit = ("revise_resubmit", "Revise and resubmit"),
        Rejected = ("rejected", "Rejected"),
    }
}

/// One reviewer's answer for one revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubmittalReview {
    pub id: u32,
    /// The revision it answers.
    pub revision: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub decision: Decision,
    #[serde(default)]
    pub comment: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Submittal {
    pub id: Id<Submittal>,
    pub project: Id<Project>,
    /// Numbered within the project.
    pub number: u32,
    /// Starts at 0; each resubmittal adds one.
    #[serde(default)]
    pub revision: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub spec_section: String,
    /// Free text: shop drawing, product data, sample…
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub status: SubmittalStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitter: Option<Id<Person>>,
    #[serde(default)]
    pub reviewers: Vec<Id<Person>>,
    #[serde(default)]
    pub distribution: Vec<Id<Person>>,
    #[serde(default)]
    pub ball_in_court: Vec<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_decision: Option<Decision>,
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default)]
    pub reviews: Vec<SubmittalReview>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Submittal, "submittal", trash, project);

impl Submittal {
    /// "Submittal #4" or, after a resubmittal, "Submittal #4 (revision 1)".
    pub fn label(&self) -> String {
        if self.revision == 0 { format!("Submittal #{}", self.number) } else { format!("Submittal #{} (revision {})", self.number, self.revision) }
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.status == SubmittalStatus::InReview && self.due.is_some_and(|d| d < today)
    }

    pub fn involves(&self, person: Id<Person>) -> bool {
        self.created.by == Some(person)
            || self.manager == Some(person)
            || self.submitter == Some(person)
            || self.reviewers.contains(&person)
            || self.distribution.contains(&person)
    }

    /// The reviews of the current revision.
    pub fn current_reviews(&self) -> impl Iterator<Item = &SubmittalReview> {
        self.reviews.iter().filter(|r| r.revision == self.revision)
    }

    pub fn has_reviewed(&self, person: Id<Person>) -> bool {
        self.current_reviews().any(|r| r.by == Some(person))
    }

    /// Reviewers first; once all have answered, the manager.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            SubmittalStatus::Draft | SubmittalStatus::Closed => Vec::new(),
            SubmittalStatus::InReview => {
                let waiting: Vec<Id<Person>> = self.reviewers.iter().copied().filter(|r| !self.has_reviewed(*r)).collect();
                if waiting.is_empty() { self.manager.into_iter().collect() } else { waiting }
            }
        };
    }
}
