//! Company reports: a table of projects with the columns someone chose, saved to run again.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::project::ProjectStage;
use crate::{choice, record};

choice! {
    /// A column of the company report (the project's name always comes first).
    pub enum ReportColumn {
        Number = ("number", "Number"),
        Stage = ("stage", "Stage"),
        Start = ("start", "Start"),
        Completion = ("completion", "Completion"),
        Open = ("open", "Needs action"),
        Overdue = ("overdue", "Past due"),
        Rfis = ("rfis", "Open RFIs"),
        RfisOverdue = ("rfis_overdue", "Past-due RFIs"),
        RfiOldest = ("rfi_oldest", "Oldest open RFI (days)"),
        RfiAge = ("rfi_age", "Average age of open RFIs (days)"),
        Submittals = ("submittals", "Submittals in review"),
        Punch = ("punch", "Open punch items"),
        LateTasks = ("late_tasks", "Late schedule tasks"),
        Tasks = ("tasks", "Open tasks"),
        Observations = ("observations", "Open observations"),
        Inspections = ("inspections", "Open inspections"),
        Incidents = ("incidents", "Open incidents"),
        MeetingActions = ("meeting_actions", "Open meeting actions"),
        Correspondence = ("correspondence", "Open correspondence"),
        Transmittals = ("transmittals", "Draft transmittals"),
        Forms = ("forms", "Forms in draft"),
        ScheduleOpen = ("schedule_open", "Unfinished schedule tasks"),
        Commitments = ("commitments", "Open commitments"),
        ChangeEvents = ("change_events", "Open change events"),
        HoursWaiting = ("hours_waiting", "Hours waiting for approval"),
        Original = ("original", "Original budget"),
        Revised = ("revised", "Revised budget"),
        Committed = ("committed", "Committed"),
        Cost = ("cost", "Cost to date"),
        Projected = ("projected", "Projected cost"),
        Variance = ("variance", "Projected over / under"),
    }
}

impl ReportColumn {
    /// The columns of the portfolio table on the home page.
    pub const STANDARD: &'static [ReportColumn] =
        &[ReportColumn::Stage, ReportColumn::Open, ReportColumn::Overdue, ReportColumn::Revised, ReportColumn::Variance];

    /// Whether the column holds money (the rest of the numbers are counts).
    pub fn is_money(self) -> bool {
        matches!(
            self,
            ReportColumn::Original
                | ReportColumn::Revised
                | ReportColumn::Committed
                | ReportColumn::Cost
                | ReportColumn::Projected
                | ReportColumn::Variance
        )
    }

    /// Whether the column holds a number that adds up across projects.
    pub fn adds_up(self) -> bool {
        !matches!(
            self,
            ReportColumn::Number
                | ReportColumn::Stage
                | ReportColumn::Start
                | ReportColumn::Completion
                | ReportColumn::RfiOldest
                | ReportColumn::RfiAge
        )
    }
}

choice! {
    /// How often a saved report is emailed.
    pub enum DeliveryRhythm {
        Daily = ("daily", "Every day"),
        Weekly = ("weekly", "Every week"),
        Monthly = ("monthly", "Every month"),
    }
}

/// Most people one report is emailed to.
pub const MAX_RECIPIENTS: usize = 25;

/// A saved report sent by email on a rhythm. Each person gets the report as they themselves would see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReportDelivery {
    pub rhythm: DeliveryRhythm,
    /// For a weekly rhythm: the weekday, 0 for Monday to 6 for Sunday.
    #[serde(default)]
    pub weekday: u8,
    /// For a monthly rhythm: the day of the month, 1 to 28.
    #[serde(default = "first_day")]
    pub day: u8,
    pub recipients: Vec<Id<Person>>,
    /// The day it was last sent (or when the rhythm was set, so the first email is the next one due).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sent: Option<buildkiosk_core::Date>,
}

fn first_day() -> u8 {
    1
}

impl ReportDelivery {
    /// The latest day on or before `today` that the rhythm calls for a delivery.
    pub fn scheduled_on(&self, today: buildkiosk_core::Date) -> Option<buildkiosk_core::Date> {
        match self.rhythm {
            DeliveryRhythm::Daily => Some(today),
            DeliveryRhythm::Weekly => {
                let back = (today.weekday().index() + 7 - usize::from(self.weekday.min(6))) % 7;
                today.add_days(-i64::try_from(back).ok()?)
            }
            DeliveryRhythm::Monthly => {
                let day = u32::from(self.day.clamp(1, 28));
                let (y, m, d) = today.ymd();
                if d >= day {
                    buildkiosk_core::Date::from_ymd(y, m, day)
                } else {
                    let (py, pm, _) = today.month_start().add_months(-1)?.ymd();
                    buildkiosk_core::Date::from_ymd(py, pm, day)
                }
            }
        }
    }

    /// Whether a delivery has come due and not been made: the latest scheduled day is after the last send.
    pub fn due(&self, today: buildkiosk_core::Date) -> bool {
        !self.recipients.is_empty() && self.scheduled_on(today).is_some_and(|s| self.last_sent.is_none_or(|l| l < s))
    }
}

/// A report definition someone saved: which columns, which projects, and how to sort.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedReport {
    pub id: Id<SavedReport>,
    pub name: String,
    pub columns: Vec<ReportColumn>,
    /// Only projects in this stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<ProjectStage>,
    /// Sort by this column (by name when none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<ReportColumn>,
    #[serde(default)]
    pub descending: bool,
    /// Everyone in the company can run it; otherwise only its owner and company admins.
    #[serde(default)]
    pub shared: bool,
    pub owner: Id<Person>,
    /// Emailed on a rhythm, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<ReportDelivery>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(SavedReport, "saved_report", trash);
