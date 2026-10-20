//! Project cost records: budget lines, commitments (subcontracts and purchase orders) and direct
//! costs. All amounts are [`Money`]; everything is tied to a cost code.

use buildkiosk_core::{Date, Id, Money, Percent};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::{Person, Vendor};
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
    /// The cost still to come, when someone has estimated it; the projected cost is then the cost to
    /// date plus this. Without it the budget is assumed to be spent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast_to_complete: Option<Money>,
    /// How much of the code's work is done, for the percent-complete forecasting method.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent_complete: Option<Percent>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BudgetLine, "budget_line", trash, project);

/// One cost code's figures when a snapshot was taken.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SnapshotRow {
    pub cost_code: Id<CostCode>,
    pub original: Money,
    pub changes: Money,
    pub committed: Money,
    pub direct: Money,
    pub pending: Money,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast_to_complete: Option<Money>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent_complete: Option<Percent>,
}

/// The budget as it stood on a day, kept to compare with later.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetSnapshot {
    pub id: Id<BudgetSnapshot>,
    pub project: Id<Project>,
    pub name: String,
    #[serde(default)]
    pub note: String,
    pub rows: Vec<SnapshotRow>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BudgetSnapshot, "budget_snapshot", trash, project);

choice! {
    /// A column of the budget table that a budget view can show.
    pub enum BudgetColumn {
        Original = ("original", "Original budget"),
        Changes = ("changes", "Budget changes"),
        Revised = ("revised", "Revised budget"),
        Committed = ("committed", "Committed"),
        Direct = ("direct", "Direct costs"),
        Cost = ("cost", "Total cost"),
        Pending = ("pending", "Pending change events"),
        Remaining = ("remaining", "Remaining"),
        Percent = ("percent", "Percent complete"),
        Forecast = ("forecast", "Forecast to complete"),
        Projected = ("projected", "Projected cost"),
        Variance = ("variance", "Over / under"),
    }
}

choice! {
    /// How a budget view groups cost codes.
    #[derive(Default)]
    pub enum BudgetGroup {
        #[default]
        None = ("none", "No grouping"),
        /// By the first part of the code ("03" of "03-300"), with a subtotal for each.
        Division = ("division", "By division"),
    }
}

/// A saved way of looking at the budget: which columns, and how the cost codes are grouped.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BudgetView {
    pub id: Id<BudgetView>,
    pub project: Id<Project>,
    pub name: String,
    pub columns: Vec<BudgetColumn>,
    #[serde(default)]
    pub group: BudgetGroup,
    /// The people who open the budget in this view by default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub default_for: Vec<Id<Person>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BudgetView, "budget_view", trash, project);

impl BudgetColumn {
    /// The columns shown when a project has no view chosen.
    pub const DEFAULT: &'static [BudgetColumn] = &[
        BudgetColumn::Original,
        BudgetColumn::Changes,
        BudgetColumn::Revised,
        BudgetColumn::Committed,
        BudgetColumn::Direct,
        BudgetColumn::Cost,
        BudgetColumn::Remaining,
        BudgetColumn::Forecast,
        BudgetColumn::Projected,
        BudgetColumn::Variance,
    ];
}

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

choice! {
    /// Where a commitment's contract is in being signed.
    #[derive(Default)]
    pub enum SignatureStatus {
        #[default]
        NotSent = ("not_sent", "Not sent"),
        /// Out with the other party for signature.
        Sent = ("sent", "Out for signature"),
        Signed = ("signed", "Signed"),
        /// Returned unsigned (the reason is in the note).
        Declined = ("declined", "Declined"),
    }
}

/// A link the other party opens to read the contract and sign it in BuildKiosk, with no account. The link itself is
/// worked out by the server from its secret and never stored, so nothing here (or in the journal) can open it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignatureLink {
    /// Where the link is sent.
    pub email: String,
    /// Names the link; it changes every time the contract is sent for signature again, so an earlier link stops working.
    pub generation: u32,
    /// The link stops working after this day; none means until the signature is cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<Date>,
    pub created: Stamp,
    /// When the server emailed the link; none while it still has to be sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Stamp>,
}

impl SignatureLink {
    /// Whether the link opens the contract on `today`.
    pub fn active(&self, today: Date) -> bool {
        self.expires.is_none_or(|e| today <= e)
    }
}

/// The signing of a contract: recorded by the team, or signed by the other party themselves through a link.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContractSignature {
    #[serde(default)]
    pub status: SignatureStatus,
    /// Who the contract was sent to.
    #[serde(default)]
    pub sent_to: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent_on: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_on: Option<Date>,
    /// Who signed for the other party.
    #[serde(default)]
    pub signer: String,
    #[serde(default)]
    pub note: String,
    /// A link for the other party to sign through, when the team sent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<SignatureLink>,
    /// The moment it was signed, when the other party signed through the link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_at: Option<Stamp>,
    /// The contract's last change when it was signed through the link: what was on the page when they signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_terms: Option<Stamp>,
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
    /// Withheld from each invoice until the work is finished.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retainage: Option<Percent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved: Option<Stamp>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default)]
    pub signature: ContractSignature,
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
    /// Where the signing of the contract with the owner stands.
    #[serde(default)]
    pub signature: ContractSignature,
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
    #[serde(default)]
    pub signature: ContractSignature,
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
    /// The raw id of the change order whose line this is (a prime change order on an owner
    /// invoice, a commitment change order on a commitment invoice).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<u64>,
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

/// An invoice from a subcontractor or supplier against a commitment, laid out like an owner
/// invoice: work completed this period on the commitment's rows, with retainage withheld.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommitmentInvoice {
    pub id: Id<CommitmentInvoice>,
    pub project: Id<Project>,
    pub number: u32,
    pub commitment: Id<Commitment>,
    pub period_end: Date,
    #[serde(default)]
    pub status: InvoiceStatus,
    #[serde(default)]
    pub notes: String,
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
record!(CommitmentInvoice, "commitment_invoice", trash, project);
