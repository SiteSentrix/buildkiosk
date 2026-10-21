//! Prequalification: whether a company has been vetted to work for this company.

use buildkiosk_core::{Date, Id, Money};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Vendor;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum PrequalStatus {
        /// The record exists; the company has not been put forward.
        #[default]
        Pending = ("pending", "Pending"),
        /// Waiting for a reviewer.
        Submitted = ("submitted", "Submitted"),
        Approved = ("approved", "Approved"),
        Rejected = ("rejected", "Rejected"),
    }
}

/// One question on a questionnaire, worth up to `max_points`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrequalQuestion {
    pub text: String,
    pub max_points: u32,
}

/// A group of questions that is scored together ("Safety", "Financial strength").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrequalSection {
    pub title: String,
    pub questions: Vec<PrequalQuestion>,
}

/// A questionnaire companies are asked to answer, written by the company running the prequalification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrequalForm {
    pub id: Id<PrequalForm>,
    pub name: String,
    pub sections: Vec<PrequalSection>,
    /// 1 for the first draft; a change to a questionnaire that companies have answered makes a new version.
    #[serde(default = "first_version")]
    pub version: u32,
    /// The version this one replaced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Id<PrequalForm>>,
    /// The version that replaced this one (companies that answered this one keep their answers on it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<Id<PrequalForm>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PrequalForm, "prequal_form", trash);

fn first_version() -> u32 {
    1
}

impl PrequalForm {
    /// The points the whole questionnaire is worth.
    pub fn possible(&self) -> u32 {
        self.sections.iter().flat_map(|s| &s.questions).fold(0u32, |a, q| a.saturating_add(q.max_points))
    }
}

/// A company's answer to one question, and the points the reviewer gave it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrequalAnswer {
    pub section: u32,
    pub question: u32,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
}

/// A warning about a date that was sent, so it isn't sent again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReminderSent {
    /// "approval", "insurance" or "license".
    pub what: String,
    /// The date it was about when the reminder went out (a renewed date starts afresh).
    pub date: Date,
    /// How many days ahead the warning was for: 60, 30, 7, or 0 for lapsed.
    pub days: u32,
}

/// A link a company's contact uses to fill in the prequalification without an account. The link itself is worked out
/// by the server from its secret and is never stored, so nothing here (or in the journal) can open it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrequalInvitation {
    /// Counts up from 1 within the record; names the link.
    pub number: u32,
    /// Where the link is sent.
    pub email: String,
    /// Bumped when the link is sent again, which makes the earlier link stop working.
    #[serde(default)]
    pub generation: u32,
    /// The link stops working after this day; none means until it is cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Date>,
    pub created: Stamp,
    /// When the server emailed the current link; none while it still has to be sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked: Option<Stamp>,
}

impl PrequalInvitation {
    /// Whether the link opens the record on `today`.
    pub fn active(&self, today: Date) -> bool {
        self.revoked.is_none() && self.expires.is_none_or(|e| today <= e)
    }
}

/// One company's prequalification record (at most one live record per company).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prequalification {
    pub id: Id<Prequalification>,
    pub vendor: Id<Vendor>,
    #[serde(default)]
    pub status: PrequalStatus,
    /// 0 to 100, from the reviewer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<u8>,
    /// The approval lapses after this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insurance_expires: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license_expires: Option<Date>,
    /// The company's experience modification rate, in hundredths (0.85 is 85).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emr_hundredths: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bonding_capacity: Option<Money>,
    #[serde(default)]
    pub notes: String,
    /// The questionnaire this company answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub form: Option<Id<PrequalForm>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<PrequalAnswer>,
    /// Insurance certificates, financial statements and the like.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<ReminderSent>,
    /// Links given to the company's contacts to fill the record in themselves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invitations: Vec<PrequalInvitation>,
    /// Warnings emailed to the company's own contact (the admins' are in `reminders`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub company_reminders: Vec<ReminderSent>,
    /// The link (its number) the record was last put forward through, when the company did it itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted_via: Option<u32>,
    /// Hashes of the files the company attached itself, which are the only ones it can take off again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub portal_files: Vec<String>,
    /// The latest message the company left for the reviewers.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub company_message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Prequalification, "prequalification", trash);

/// How a record stands on a given day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Standing {
    /// Approved and nothing has lapsed.
    Current,
    /// Approved, but something lapses within the warning period.
    ExpiringSoon,
    /// Approved, but the approval, insurance or licence has lapsed.
    Lapsed,
    /// Not approved (yet).
    NotApproved,
}

impl Prequalification {
    /// Days of warning before something lapses.
    pub const WARN_DAYS: i64 = 60;

    /// The points given so far, and what the questionnaire is worth, for one section or all of them.
    pub fn points(&self, form: &PrequalForm, section: Option<u32>) -> (u32, u32) {
        let mut earned = 0u32;
        let mut possible = 0u32;
        for (si, s) in form.sections.iter().enumerate() {
            if section.is_some_and(|only| usize::try_from(only).ok() != Some(si)) {
                continue;
            }
            for (qi, q) in s.questions.iter().enumerate() {
                possible = possible.saturating_add(q.max_points);
                let got = self
                    .answers
                    .iter()
                    .find(|a| usize::try_from(a.section).ok() == Some(si) && usize::try_from(a.question).ok() == Some(qi))
                    .and_then(|a| a.points);
                earned = earned.saturating_add(got.unwrap_or(0).min(q.max_points));
            }
        }
        (earned, possible)
    }

    pub fn standing(&self, today: Date) -> Standing {
        if self.status != PrequalStatus::Approved {
            return Standing::NotApproved;
        }
        let dates = [self.valid_until, self.insurance_expires, self.license_expires];
        if dates.iter().flatten().any(|d| *d < today) {
            Standing::Lapsed
        } else if dates.iter().flatten().any(|d| today.days_until(*d) <= Self::WARN_DAYS) {
            Standing::ExpiringSoon
        } else {
            Standing::Current
        }
    }
}
