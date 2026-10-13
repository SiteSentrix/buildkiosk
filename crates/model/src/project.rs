//! Projects and cost codes.

use buildkiosk_core::{Date, Id, Money};
use serde::{Deserialize, Serialize};

use crate::common::{Address, Stamp};
use crate::directory::Vendor;
use crate::{choice, record};

choice! {
    /// Where a project is in its life.
    #[derive(Default)]
    pub enum ProjectStage {
        Bidding = ("bidding", "Bidding"),
        Preconstruction = ("preconstruction", "Preconstruction"),
        #[default]
        Construction = ("construction", "Construction"),
        Closeout = ("closeout", "Closeout"),
        Warranty = ("warranty", "Warranty"),
        Complete = ("complete", "Complete"),
        OnHold = ("on_hold", "On hold"),
        Cancelled = ("cancelled", "Cancelled"),
    }
}

/// A construction project.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub id: Id<Project>,
    /// The company's job number ("2026-014").
    #[serde(default)]
    pub number: String,
    pub name: String,
    #[serde(default)]
    pub stage: ProjectStage,
    /// Project type ("Commercial", "Healthcare", "Multifamily").
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub address: Address,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<Date>,
    /// Estimated total value, for the portfolio (the prime contract holds the contract value).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Money>,
    #[serde(default)]
    pub description: String,
    /// The owner / client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Id<Vendor>>,
    /// Working days to an RFI's default due date on this project (else the company's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rfi_due_days: Option<u32>,
    #[serde(default = "yes")]
    pub active: bool,
    /// The original budget is locked: only budget changes alter it.
    #[serde(default)]
    pub budget_locked: bool,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Project, "project", trash);

fn yes() -> bool {
    true
}

impl Project {
    /// "2026-014 Riverside Clinic", or just the name.
    pub fn display_name(&self) -> String {
        if self.number.is_empty() { self.name.clone() } else { format!("{} {}", self.number, self.name) }
    }
}

choice! {
    /// The kind of cost a budget or commitment line is for (the second part of a cost code).
    #[derive(Default)]
    pub enum CostType {
        Labor = ("labor", "Labor"),
        Material = ("material", "Material"),
        Equipment = ("equipment", "Equipment"),
        #[default]
        Subcontract = ("subcontract", "Subcontract"),
        Other = ("other", "Other"),
    }
}

/// One code of a cost breakdown ("03-300 Cast-in-place concrete"). Codes with no project are the
/// company's standard list, copied into new projects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostCode {
    pub id: Id<CostCode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id<Project>>,
    pub code: String,
    pub description: String,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(CostCode, "cost_code", trash);

impl CostCode {
    pub fn display(&self) -> String {
        format!("{} {}", self.code, self.description)
    }
}
