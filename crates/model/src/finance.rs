//! Project cost records: budget lines, commitments (subcontracts and purchase orders) and direct
//! costs. All amounts are [`Money`]; everything is tied to a cost code.

use buildkiosk_core::{Date, Id, Money};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Vendor;
use crate::project::{CostCode, CostType, Project};
use crate::{choice, record};

/// What a project plans to spend on one cost code.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetLine {
    pub id: Id<BudgetLine>,
    pub project: Id<Project>,
    pub cost_code: Id<CostCode>,
    /// The original budget amount.
    pub original: Money,
    #[serde(default)]
    pub notes: String,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BudgetLine, "budget_line", trash, project);

choice! {
    #[derive(Default)]
    pub enum CommitmentKind {
        #[default]
        Subcontract = ("subcontract", "Subcontract"),
        PurchaseOrder = ("purchase_order", "Purchase order"),
    }
}

choice! {
    #[derive(Default)]
    pub enum CommitmentStatus {
        #[default]
        Draft = ("draft", "Draft"),
        Approved = ("approved", "Approved"),
        Complete = ("complete", "Complete"),
        Void = ("void", "Void"),
    }
}

impl CommitmentStatus {
    /// Whether the amounts count as committed cost.
    pub fn counts(self) -> bool {
        matches!(self, CommitmentStatus::Approved | CommitmentStatus::Complete)
    }
}

/// One line of a commitment's schedule of values.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentLine {
    pub cost_code: Id<CostCode>,
    #[serde(default)]
    pub description: String,
    pub amount: Money,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Commitment {
    pub id: Id<Commitment>,
    pub project: Id<Project>,
    /// Numbered within the project.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub kind: CommitmentKind,
    #[serde(default)]
    pub status: CommitmentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<Id<Vendor>>,
    #[serde(default)]
    pub description: String,
    pub lines: Vec<CommitmentLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Commitment, "commitment", trash, project);

impl Commitment {
    pub fn label(&self) -> String {
        format!("{} #{}", self.kind.label_text(), self.number)
    }

    /// The commitment's total, if it fits in the money range.
    pub fn total(&self) -> Option<Money> {
        Money::sum(self.lines.iter().map(|l| l.amount))
    }
}

impl CommitmentKind {
    fn label_text(self) -> &'static str {
        match self {
            CommitmentKind::Subcontract => "Subcontract",
            CommitmentKind::PurchaseOrder => "Purchase order",
        }
    }
}

/// A cost paid without a commitment: a purchase, a payroll cost, a rental.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DirectCost {
    pub id: Id<DirectCost>,
    pub project: Id<Project>,
    pub date: Date,
    pub cost_code: Id<CostCode>,
    #[serde(default)]
    pub kind: CostType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<Id<Vendor>>,
    pub description: String,
    pub amount: Money,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(DirectCost, "direct_cost", trash, project);
