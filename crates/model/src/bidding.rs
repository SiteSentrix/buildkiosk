//! Bidding: a package of work sent out to bid, the companies invited, the bids received and the award.

use buildkiosk_core::{Date, Id, Money, Percent};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Vendor;
use crate::finance::Commitment;
use crate::project::{CostCode, Project};
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum BidStatus {
        /// Being written; no one has been invited.
        #[default]
        Draft = ("draft", "Draft"),
        /// Out for bid.
        Open = ("open", "Out for bid"),
        /// No longer taking bids; being compared.
        Closed = ("closed", "Closed"),
        Awarded = ("awarded", "Awarded"),
    }
}

choice! {
    /// What a bidder's price covers of one scope item.
    pub enum ScopeAnswer {
        Included = ("included", "Included"),
        Excluded = ("excluded", "Excluded"),
        Unclear = ("unclear", "Not clear"),
    }
}

/// One line of the bid form: bidders give a unit price for it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BidItem {
    pub description: String,
    #[serde(default)]
    pub unit: String,
    /// The quantity in thousandths: 2.5 is 2500.
    pub quantity_milli: i64,
}

/// A change sent to every bidder as the package is out for bid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Addendum {
    /// Counts up from 1.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub issued: Stamp,
}

/// A bidder saying (or being recorded as saying) it has the addendum.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Acknowledgement {
    pub vendor: Id<Vendor>,
    pub number: u32,
    pub stamp: Stamp,
}

/// A way bids are judged besides price, and how much it counts. Weights add up to 100.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Criterion {
    pub name: String,
    pub weight: u32,
    /// Scored by the system from the leveled amounts (the lowest bid gets the top score); at most one criterion.
    #[serde(default)]
    pub price: bool,
}

/// The team's score of one bid on one criterion, 0 to 10.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BidScore {
    /// The criterion, from 0.
    pub criterion: u32,
    pub score: u32,
}

/// A bidder's unit price for a line of the bid form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BidLine {
    /// The line of the bid form, from 0.
    pub item: u32,
    pub unit_price: Money,
}

/// A bidder's price for an alternate the package asked for (an add, or a deduct when negative).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BidAlternate {
    /// The alternate, from 0.
    pub index: u32,
    pub amount: Money,
}

/// What a bid covers of one scope item, with the amount to add to compare it fairly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScopeResponse {
    /// The scope item, from 0.
    pub item: u32,
    pub answer: ScopeAnswer,
    /// What it would cost to bring the bid up to the full scope (for an exclusion).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjustment: Option<Money>,
}

/// The bid bond that came with a bid.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BondReceipt {
    #[serde(default)]
    pub received: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<Money>,
    #[serde(default)]
    pub note: String,
}

/// A bid from one company. Only the latest bid from a company stands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bid {
    pub vendor: Id<Vendor>,
    pub amount: Money,
    /// The day it was received.
    pub received: Date,
    /// Inclusions, exclusions, alternates.
    #[serde(default)]
    pub notes: String,
    /// The company took itself out.
    #[serde(default)]
    pub withdrawn: bool,
    /// Unit prices for the bid form's lines (the amount is their total when there are any).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<BidLine>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<BidAlternate>,
    /// What the bid covers of each scope item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<ScopeResponse>,
    #[serde(default)]
    pub bond: BondReceipt,
    /// The team's scores on the package's criteria (the price criterion is worked out, not scored).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scores: Vec<BidScore>,
    /// The proposal and anything else the company sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileRef>,
}

impl Bid {
    /// The adjustments to bring the bid up to the full scope, added together.
    pub fn adjustments(&self) -> Option<Money> {
        Money::sum(self.scope.iter().filter_map(|s| s.adjustment))
    }

    /// The amount with its adjustments: what to compare bids by.
    pub fn leveled(&self) -> Option<Money> {
        self.amount.checked_add(self.adjustments()?)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BidPackage {
    pub id: Id<BidPackage>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub title: String,
    /// What is being bought.
    #[serde(default)]
    pub scope: String,
    /// Bids are due on this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    /// The cost code the work will be bought against, to compare bids with the budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_code: Option<Id<CostCode>>,
    #[serde(default)]
    pub status: BidStatus,
    #[serde(default)]
    pub invited: Vec<Id<Vendor>>,
    /// What bids should cover, one item each, for comparing them on the same scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_items: Vec<String>,
    /// The bid form: lines to price.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub form_items: Vec<BidItem>,
    /// Alternates bidders are asked to price.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternates: Vec<String>,
    /// A bid bond of this share of the bid is required with each bid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bond_required: Option<Percent>,
    /// What bids are judged on besides price; empty means price alone.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub criteria: Vec<Criterion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addenda: Vec<Addendum>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub acknowledgements: Vec<Acknowledgement>,
    /// The bid documents: drawings lists, the scope, the form of proposal.
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default)]
    pub bids: Vec<Bid>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub awarded_to: Option<Id<Vendor>>,
    /// The draft commitment made when the package was awarded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commitment: Option<Id<Commitment>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(BidPackage, "bid_package", trash, project);

impl BidPackage {
    pub fn label(&self) -> String {
        format!("Bid package #{}", self.number)
    }

    /// The addenda a company has not yet acknowledged.
    pub fn unacknowledged(&self, vendor: Id<Vendor>) -> Vec<u32> {
        self.addenda.iter().map(|a| a.number).filter(|n| !self.acknowledgements.iter().any(|x| x.vendor == vendor && x.number == *n)).collect()
    }

    /// A bid's weighted total in hundredths of a point out of 10 (1000 is a perfect score), or none when the package has
    /// no criteria or a criterion is not scored yet. The price criterion gives the lowest leveled bid the top score and
    /// the others in proportion.
    pub fn weighted(&self, bid: &Bid) -> Option<u32> {
        if self.criteria.is_empty() {
            return None;
        }
        let amount = |b: &Bid| b.leveled().unwrap_or(b.amount).cents();
        let low = self.standing().iter().map(|b| amount(b)).filter(|c| *c > 0).min()?;
        let mut total: u64 = 0;
        for (i, c) in self.criteria.iter().enumerate() {
            let hundredths: u64 = if c.price {
                let mine = u64::try_from(amount(bid)).ok().filter(|m| *m > 0)?;
                u64::try_from(low).ok()?.checked_mul(1000)? / mine
            } else {
                let given = bid.scores.iter().find(|s| usize::try_from(s.criterion).is_ok_and(|x| x == i))?;
                u64::from(given.score.min(10)) * 100
            };
            total = total.checked_add(u64::from(c.weight).checked_mul(hundredths)? / 100)?;
        }
        u32::try_from(total).ok()
    }

    /// The bids that stand (not withdrawn), lowest first.
    pub fn standing(&self) -> Vec<&Bid> {
        let mut v: Vec<&Bid> = self.bids.iter().filter(|b| !b.withdrawn).collect();
        v.sort_by_key(|b| b.amount);
        v
    }
}
