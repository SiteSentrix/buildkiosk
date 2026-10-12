//! Project cost records: budget lines, commitments (subcontracts and purchase orders) and direct
//! costs. All amounts are [`Money`]; everything is tied to a cost code.

use buildkiosk_core::{Date, Id, Money, Percent};
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

/// One line of a schedule of values or a change: an amount against a cost code.
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

/// The same line shape, under the name the change records use.
pub type CostLine = CommitmentLine;

choice! {
    #[derive(Default)]
    pub enum ChangeEventStatus {
        #[default]
        Open = ("open", "Open"),
        /// Settled: its changes were made, or it needs none.
        Closed = ("closed", "Closed"),
        Void = ("void", "Void"),
    }
}

/// A possible change to the project's cost, tracked before it is priced or approved. While open,
/// its estimated amounts show as pending exposure on the budget.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangeEvent {
    pub id: Id<ChangeEvent>,
    pub project: Id<Project>,
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Why it came up: owner request, field condition, design change… (free text).
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub status: ChangeEventStatus,
    /// The estimated cost, by cost code.
    pub lines: Vec<CostLine>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(ChangeEvent, "change_event", trash, project);

choice! {
    #[derive(Default)]
    pub enum ChangeStatus {
        #[default]
        Draft = ("draft", "Draft"),
        Approved = ("approved", "Approved"),
        Void = ("void", "Void"),
    }
}

/// An approved change to what a commitment costs, by cost code (amounts may be negative).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentChange {
    pub id: Id<CommitmentChange>,
    pub project: Id<Project>,
    pub number: u32,
    pub commitment: Id<Commitment>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_event: Option<Id<ChangeEvent>>,
    #[serde(default)]
    pub status: ChangeStatus,
    pub lines: Vec<CostLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(CommitmentChange, "commitment_change", trash, project);

/// An approved change to the budget, by cost code: add money, take it away, or move it between codes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetChange {
    pub id: Id<BudgetChange>,
    pub project: Id<Project>,
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_event: Option<Id<ChangeEvent>>,
    #[serde(default)]
    pub status: ChangeStatus,
    pub lines: Vec<CostLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BudgetChange, "budget_change", trash, project);

// -- owner billing

/// The contract with the owner: its schedule of values (lines by cost code) is what gets billed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrimeContract {
    pub id: Id<PrimeContract>,
    pub project: Id<Project>,
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// The owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Id<Vendor>>,
    /// Withheld from each invoice until the work is finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retainage: Option<Percent>,
    #[serde(default)]
    pub status: ChangeStatus,
    /// The schedule of values.
    pub lines: Vec<CostLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PrimeContract, "prime_contract", trash, project);

/// An approved change to the prime contract's value; its lines become billable once approved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrimeChange {
    pub id: Id<PrimeChange>,
    pub project: Id<Project>,
    pub number: u32,
    pub contract: Id<PrimeContract>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_event: Option<Id<ChangeEvent>>,
    #[serde(default)]
    pub status: ChangeStatus,
    pub lines: Vec<CostLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PrimeChange, "prime_change", trash, project);

choice! {
    #[derive(Default)]
    pub enum InvoiceStatus {
        #[default]
        Draft = ("draft", "Draft"),
        /// Sent to the owner for approval.
        Submitted = ("submitted", "Submitted"),
        Approved = ("approved", "Approved"),
        Paid = ("paid", "Paid"),
        Void = ("void", "Void"),
    }
}

impl InvoiceStatus {
    /// Whether the invoice's work counts as billed to date.
    pub fn billed(self) -> bool {
        matches!(self, InvoiceStatus::Submitted | InvoiceStatus::Approved | InvoiceStatus::Paid)
    }
}

/// Work completed this period on one billable row: a line of the contract's schedule of values
/// (`change` empty) or of an approved prime change order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InvoiceLine {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Id<PrimeChange>>,
    /// The line's position in the contract or change order, from 0.
    pub line: u32,
    pub this_period: Money,
}

/// An invoice (payment application) to the owner for the work completed in a period.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OwnerInvoice {
    pub id: Id<OwnerInvoice>,
    pub project: Id<Project>,
    pub number: u32,
    pub contract: Id<PrimeContract>,
    /// The last day of the period it covers.
    pub period_end: Date,
    #[serde(default)]
    pub status: InvoiceStatus,
    #[serde(default)]
    pub notes: String,
    /// Only the rows with an amount.
    pub lines: Vec<InvoiceLine>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paid_on: Option<Date>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(OwnerInvoice, "owner_invoice", trash, project);
