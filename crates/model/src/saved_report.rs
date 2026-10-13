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
        Submittals = ("submittals", "Submittals in review"),
        Punch = ("punch", "Open punch items"),
        LateTasks = ("late_tasks", "Late schedule tasks"),
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

    /// Whether the column holds a number that adds up across projects.
    pub fn adds_up(self) -> bool {
        !matches!(self, ReportColumn::Number | ReportColumn::Stage | ReportColumn::Start | ReportColumn::Completion)
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
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(SavedReport, "saved_report", trash);
