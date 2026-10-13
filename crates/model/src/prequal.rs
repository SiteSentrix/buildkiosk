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
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PrequalForm, "prequal_form", trash);

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
