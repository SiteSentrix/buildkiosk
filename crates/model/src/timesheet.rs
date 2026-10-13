//! Timesheets: hours people worked on a project, by day, submitted for approval.

use buildkiosk_core::{Date, Id, Money, Percent};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::finance::DirectCost;
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
    /// Who sent the entry for approval and when: the worker's sign-off on their hours.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    /// The direct cost the entry's labour was posted to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted: Option<Id<DirectCost>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(TimeEntry, "time_entry", trash, project);

/// What one person is paid per hour on a project, from a date on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PayRate {
    pub id: Id<PayRate>,
    pub project: Id<Project>,
    pub person: Id<Person>,
    pub from: Date,
    pub hourly: Money,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PayRate, "pay_rate", trash, project);

/// When hours count as overtime on a project. A threshold of 0 switches that rule off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OvertimeRules {
    /// Tenths of an hour per person per day after which hours are overtime.
    #[serde(default)]
    pub daily_tenths: u32,
    /// Tenths of an hour per person per Monday-to-Sunday week (not counting daily overtime) after which hours are overtime.
    #[serde(default)]
    pub weekly_tenths: u32,
    /// Overtime pay as a percentage of the hourly rate; 150% when unset.
    #[serde(default)]
    pub multiplier: Option<Percent>,
}

impl OvertimeRules {
    pub fn pay_multiplier(&self) -> Percent {
        self.multiplier.unwrap_or(Percent::from_hundredths(15_000))
    }
}

/// A named group of people who are logged together ("Concrete crew").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Crew {
    pub id: Id<Crew>,
    pub project: Id<Project>,
    pub name: String,
    pub members: Vec<Id<Person>>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Crew, "crew", trash, project);
