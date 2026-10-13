//! Prequalification: whether a company has been vetted to work for this company.

use buildkiosk_core::{Date, Id, Money};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
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
