//! Submittals: shop drawings, product data, samples and the other documents a trade sends for
//! review before it buys, makes or installs something, and the packages that group them.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::{Person, Vendor};
use crate::project::Project;
use crate::specification::SpecSection;
use crate::{choice, record};

choice! {
    /// What kind of document a submittal is.
    #[derive(Default)]
    pub enum SubmittalKind {
        #[default]
        ShopDrawing = ("shop_drawing", "Shop drawing"),
        ProductData = ("product_data", "Product data"),
        Sample = ("sample", "Sample"),
        MockUp = ("mock_up", "Mock-up"),
        TestReport = ("test_report", "Test report"),
        Certificate = ("certificate", "Certificate"),
        OperationMaintenance = ("om_data", "Operation and maintenance data"),
        Warranty = ("warranty", "Warranty"),
        Other = ("other", "Other"),
    }
}

choice! {
    /// Where a submittal is in its life.
    #[derive(Default)]
    pub enum SubmittalStatus {
        /// Being put together; only its author, its manager and submittal admins see it.
        #[default]
        Draft = ("draft", "Draft"),
        /// Sent out: the review workflow is running, or waiting for the manager to return it.
        Open = ("open", "In review"),
        /// Returned: accepted as submitted.
        Approved = ("approved", "Approved"),
        /// Returned: accepted subject to the reviewers' notes, which the work must follow.
        ApprovedAsNoted = ("approved_as_noted", "Approved as noted"),
        /// Returned: the submitter corrects it and sends a new revision.
        ReviseResubmit = ("revise_resubmit", "Revise and resubmit"),
        /// Returned: not accepted.
        Rejected = ("rejected", "Rejected"),
        /// Finished without a review outcome (no longer needed, or replaced by a revision while in
        /// review).
        Closed = ("closed", "Closed"),
        /// Cancelled; kept on the log for the record.
        Void = ("void", "Void"),
    }
}

impl SubmittalStatus {
    /// The outcomes a manager can return a submittal with.
    pub const OUTCOMES: [SubmittalStatus; 4] =
        [SubmittalStatus::Approved, SubmittalStatus::ApprovedAsNoted, SubmittalStatus::ReviseResubmit, SubmittalStatus::Rejected];

    /// Returned to the submitter with a review outcome.
    pub fn is_returned(self) -> bool {
        SubmittalStatus::OUTCOMES.contains(&self)
    }

    /// Finished: returned, closed or void. Only [`SubmittalStatus::Draft`] and
    /// [`SubmittalStatus::Open`] are still in progress.
    pub fn is_closed(self) -> bool {
        self.is_returned() || matches!(self, SubmittalStatus::Closed | SubmittalStatus::Void)
    }
}

choice! {
    /// A reviewer's answer on one workflow step.
    pub enum ReviewOutcome {
        Approved = ("approved", "Approved"),
        ApprovedAsNoted = ("approved_as_noted", "Approved as noted"),
        ReviseResubmit = ("revise_resubmit", "Revise and resubmit"),
        Rejected = ("rejected", "Rejected"),
        /// Seen, with no decision: the reviewer only takes note of it.
        ForRecord = ("for_record", "For record only"),
    }
}

impl ReviewOutcome {
    /// The returned status that matches this answer; `None` for answers that decide nothing.
    pub fn status(self) -> Option<SubmittalStatus> {
        match self {
            ReviewOutcome::Approved => Some(SubmittalStatus::Approved),
            ReviewOutcome::ApprovedAsNoted => Some(SubmittalStatus::ApprovedAsNoted),
            ReviewOutcome::ReviseResubmit => Some(SubmittalStatus::ReviseResubmit),
            ReviewOutcome::Rejected => Some(SubmittalStatus::Rejected),
            ReviewOutcome::ForRecord => None,
        }
    }
}

/// One reviewer's answer on a workflow step.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewResponse {
    /// The reviewer.
    pub by: Id<Person>,
    pub outcome: ReviewOutcome,
    #[serde(default)]
    pub comment: String,
    pub at: Timestamp,
    /// Who entered it, when the manager recorded an answer the reviewer gave outside BuildKiosk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entered_by: Option<Id<Person>>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
}

/// One step of a review workflow. Everyone on a step reviews at the same time; the workflow moves
/// to the next step once all of them have answered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviewStep {
    pub reviewers: Vec<Id<Person>>,
    /// When the step's answers are due (set when the submittal is sent, unless given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// At most one per reviewer.
    #[serde(default)]
    pub responses: Vec<ReviewResponse>,
}

impl ReviewStep {
    pub fn response_of(&self, person: Id<Person>) -> Option<&ReviewResponse> {
        self.responses.iter().find(|r| r.by == person)
    }

    /// The reviewers who haven't answered yet, in order.
    pub fn waiting(&self) -> impl Iterator<Item = Id<Person>> + '_ {
        self.reviewers.iter().copied().filter(|r| self.response_of(*r).is_none())
    }

    pub fn is_complete(&self) -> bool {
        self.waiting().next().is_none()
    }
}

/// A document sent for review, with its ordered review workflow. Each resubmission is a separate
/// record with the same number and the next revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Submittal {
    pub id: Id<Submittal>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1; every revision keeps its original's number.
    pub number: u32,
    /// 0 for the original, then 1, 2… for each resubmission.
    #[serde(default)]
    pub revision: u32,
    /// The revision this one replaces.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_of: Option<Id<Submittal>>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub kind: SubmittalKind,
    #[serde(default)]
    pub status: SubmittalStatus,
    /// The specification section it answers, as text ("08 71 00 Door hardware").
    #[serde(default)]
    pub spec_section: String,
    /// The same section in the Specifications tool, when linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Id<SpecSection>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package: Option<Id<SubmittalPackage>>,
    /// The company that owes the submittal (usually a subcontractor or supplier).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsible_vendor: Option<Id<Vendor>>,
    /// Who submitted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_from: Option<Id<Person>>,
    /// Runs it: sends it out, returns it with the final outcome, closes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    /// People copied on it.
    #[serde(default)]
    pub distribution: Vec<Id<Person>>,
    /// Only the people on it and submittal admins can see it.
    #[serde(default)]
    pub private: bool,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    /// When the submitter has to hand it in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submit_by: Option<Date>,
    /// When it was received from the submitter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received: Option<Date>,
    /// When the material or work it covers has to be on site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_on_site: Option<Date>,
    /// Calendar days from approval to delivery (fabrication, shipping).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lead_time_days: Option<u32>,
    /// The review workflow, in order.
    #[serde(default)]
    pub steps: Vec<ReviewStep>,
    /// Who has to act next: the current step's reviewers who haven't answered, else the manager.
    /// Nobody unless it's in review.
    #[serde(default)]
    pub ball_in_court: Vec<Id<Person>>,
    /// When the workflow started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Stamp>,
    /// When the manager returned it with an outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub returned: Option<Stamp>,
    /// The manager's note with the outcome.
    #[serde(default)]
    pub return_comment: String,
    /// When it was returned, closed or voided.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Submittal, "submittal", trash, project);

impl Submittal {
    /// "Submittal #12", or "Submittal #12 rev 2" for a resubmission.
    pub fn label(&self) -> String {
        if self.revision == 0 { format!("Submittal #{}", self.number) } else { format!("Submittal #{} rev {}", self.number, self.revision) }
    }

    /// The index of the step under review: the first one with a reviewer who hasn't answered.
    /// `None` when every step is complete (or there are none). Meaningful while in review.
    pub fn current_step(&self) -> Option<usize> {
        self.steps.iter().position(|s| !s.is_complete())
    }

    /// Everyone on the workflow, once each, in step order.
    pub fn reviewers(&self) -> Vec<Id<Person>> {
        let mut out: Vec<Id<Person>> = Vec::new();
        for r in self.steps.iter().flat_map(|s| s.reviewers.iter().copied()) {
            if !out.contains(&r) {
                out.push(r);
            }
        }
        out
    }

    /// When the review is due while it's in review: the current step's due date, or the last
    /// step's once every step has answered (the manager still has to return it).
    pub fn review_due(&self) -> Option<Date> {
        if self.status != SubmittalStatus::Open {
            return None;
        }
        match self.current_step() {
            Some(i) => self.steps.get(i).and_then(|s| s.due),
            None => self.steps.last().and_then(|s| s.due),
        }
    }

    /// Late: a draft past its submit-by date, or a review past its due date.
    pub fn is_overdue(&self, today: Date) -> bool {
        match self.status {
            SubmittalStatus::Draft => self.submit_by.is_some_and(|d| d < today),
            SubmittalStatus::Open => self.review_due().is_some_and(|d| d < today),
            _ => false,
        }
    }

    /// Whether `person` is part of the submittal (sees it even when it's private).
    pub fn involves(&self, person: Id<Person>) -> bool {
        self.created.by == Some(person)
            || self.manager == Some(person)
            || self.received_from == Some(person)
            || self.distribution.contains(&person)
            || self.steps.iter().any(|s| s.reviewers.contains(&person))
    }

    /// The outcome the answers point to, for preselecting the manager's choice: the strictest
    /// decision on the last step that has one (rejected, then revise and resubmit, then approved as
    /// noted, then approved). `None` when no reviewer has decided anything.
    pub fn suggested_outcome(&self) -> Option<SubmittalStatus> {
        let rank = |s: SubmittalStatus| match s {
            SubmittalStatus::Rejected => 4,
            SubmittalStatus::ReviseResubmit => 3,
            SubmittalStatus::ApprovedAsNoted => 2,
            _ => 1,
        };
        self.steps.iter().rev().find_map(|step| step.responses.iter().filter_map(|r| r.outcome.status()).max_by_key(|s| rank(*s)))
    }

    /// Recomputes who has to act: while in review, the current step's reviewers who haven't
    /// answered, else the manager; nobody in any other status.
    pub fn recompute_ball_in_court(&mut self) {
        self.ball_in_court = match self.status {
            SubmittalStatus::Open => match self.current_step().and_then(|i| self.steps.get(i)) {
                Some(step) => step.waiting().collect(),
                None => self.manager.into_iter().collect(),
            },
            _ => Vec::new(),
        };
    }
}

/// A group of submittals that travel together (everything for one spec section, or one trade's
/// doors and hardware).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubmittalPackage {
    pub id: Id<SubmittalPackage>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The specification section it covers, as text.
    #[serde(default)]
    pub spec_section: String,
    /// The same section in the Specifications tool, when linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec: Option<Id<SpecSection>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(SubmittalPackage, "submittal_package", trash, project);

impl SubmittalPackage {
    /// "Package #3: Doors and hardware".
    pub fn label(&self) -> String {
        format!("Package #{}: {}", self.number, self.title)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Choice;

    fn stamp() -> Stamp {
        Stamp { by: Some(Id::new(1)), at: Timestamp::from_millis(0) }
    }

    fn response(by: u64, outcome: ReviewOutcome) -> ReviewResponse {
        ReviewResponse { by: Id::new(by), outcome, comment: String::new(), at: Timestamp::from_millis(0), entered_by: None, attachments: Vec::new() }
    }

    fn submittal(steps: Vec<ReviewStep>) -> Submittal {
        Submittal {
            id: Id::new(1),
            project: Id::new(1),
            number: 1,
            revision: 0,
            revision_of: None,
            title: "Door hardware".into(),
            description: String::new(),
            kind: SubmittalKind::default(),
            status: SubmittalStatus::Open,
            spec_section: String::new(),
            spec: None,
            package: None,
            responsible_vendor: None,
            received_from: None,
            manager: Some(Id::new(9)),
            distribution: Vec::new(),
            private: false,
            attachments: Vec::new(),
            submit_by: None,
            received: None,
            required_on_site: None,
            lead_time_days: None,
            steps,
            ball_in_court: Vec::new(),
            sent: None,
            returned: None,
            return_comment: String::new(),
            closed: None,
            created: stamp(),
            updated: stamp(),
            deleted: None,
        }
    }

    fn step(reviewers: &[u64], due: &str, responses: Vec<ReviewResponse>) -> ReviewStep {
        ReviewStep { reviewers: reviewers.iter().map(|r| Id::new(*r)).collect(), due: Date::parse(due), responses }
    }

    #[test]
    fn ball_in_court_follows_the_steps_then_the_manager() {
        let mut s = submittal(vec![step(&[2, 3], "2026-10-26", vec![response(2, ReviewOutcome::Approved)]), step(&[4], "2026-11-09", vec![])]);
        s.recompute_ball_in_court();
        assert_eq!(s.current_step(), Some(0));
        assert_eq!(s.ball_in_court, vec![Id::new(3)]);
        assert_eq!(s.review_due(), Date::parse("2026-10-26"));
        s.steps[0].responses.push(response(3, ReviewOutcome::ApprovedAsNoted));
        s.recompute_ball_in_court();
        assert_eq!(s.ball_in_court, vec![Id::new(4)]);
        s.steps[1].responses.push(response(4, ReviewOutcome::ForRecord));
        s.recompute_ball_in_court();
        assert_eq!(s.current_step(), None);
        assert_eq!(s.ball_in_court, vec![Id::new(9)]);
        // Due for the manager's return: the last step's date.
        assert_eq!(s.review_due(), Date::parse("2026-11-09"));
        assert!(s.is_overdue(Date::parse("2026-11-10").unwrap()));
        assert!(!s.is_overdue(Date::parse("2026-11-09").unwrap()));
        s.status = SubmittalStatus::Approved;
        s.recompute_ball_in_court();
        assert!(s.ball_in_court.is_empty());
        assert!(!s.is_overdue(Date::parse("2030-01-01").unwrap()));
    }

    #[test]
    fn suggested_outcome_takes_the_strictest_answer_of_the_last_deciding_step() {
        let s = submittal(vec![
            step(&[2], "2026-10-26", vec![response(2, ReviewOutcome::Rejected)]),
            step(&[3, 4], "2026-11-09", vec![response(3, ReviewOutcome::Approved), response(4, ReviewOutcome::ApprovedAsNoted)]),
            step(&[5], "2026-11-23", vec![response(5, ReviewOutcome::ForRecord)]),
        ]);
        assert_eq!(s.suggested_outcome(), Some(SubmittalStatus::ApprovedAsNoted));
        assert_eq!(submittal(vec![step(&[2], "2026-10-26", vec![])]).suggested_outcome(), None);
    }

    #[test]
    fn drafts_are_late_after_their_submit_by_date() {
        let mut s = submittal(vec![]);
        s.status = SubmittalStatus::Draft;
        s.submit_by = Date::parse("2026-10-12");
        assert!(s.is_overdue(Date::parse("2026-10-13").unwrap()));
        assert!(!s.is_overdue(Date::parse("2026-10-12").unwrap()));
    }

    #[test]
    fn statuses_and_labels() {
        assert!(SubmittalStatus::ReviseResubmit.is_returned() && SubmittalStatus::ReviseResubmit.is_closed());
        assert!(SubmittalStatus::Void.is_closed() && !SubmittalStatus::Void.is_returned());
        assert!(!SubmittalStatus::Open.is_closed() && !SubmittalStatus::Draft.is_closed());
        assert_eq!(SubmittalStatus::parse("In review"), Some(SubmittalStatus::Open));
        let mut s = submittal(vec![step(&[2], "2026-10-26", vec![])]);
        assert_eq!(s.label(), "Submittal #1");
        s.revision = 2;
        assert_eq!(s.label(), "Submittal #1 rev 2");
        assert!(s.involves(Id::new(2)) && s.involves(Id::new(9)) && !s.involves(Id::new(7)));
        assert_eq!(serde_json::to_value(SubmittalKind::OperationMaintenance).unwrap(), "om_data");
    }

    #[test]
    fn old_rows_without_optional_fields_still_read() {
        let v = serde_json::json!({
            "id": 3, "project": 1, "number": 2, "title": "Anchors",
            "created": {"at": "2026-10-12T09:00:00Z"}, "updated": {"at": "2026-10-12T09:00:00Z"}
        });
        let s: Submittal = serde_json::from_value(v).unwrap();
        assert_eq!(s.status, SubmittalStatus::Draft);
        assert_eq!(s.revision, 0);
        assert!(s.steps.is_empty());
    }
}
