//! Bidding: a package of work sent out to bid, the companies invited, the bids received and the award.

use buildkiosk_core::{Date, Id, Money};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
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

    /// The bids that stand (not withdrawn), lowest first.
    pub fn standing(&self) -> Vec<&Bid> {
        let mut v: Vec<&Bid> = self.bids.iter().filter(|b| !b.withdrawn).collect();
        v.sort_by_key(|b| b.amount);
        v
    }
}
