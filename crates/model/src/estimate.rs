//! Estimating: a priced list of work by cost code with markups, which can become the budget.

use buildkiosk_core::{Id, Money, Percent};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::project::{CostCode, Project};
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum EstimateStatus {
        #[default]
        Draft = ("draft", "Draft"),
        /// Priced and issued; no longer edited.
        Final = ("final", "Final"),
    }
}

/// One priced line: a quantity of something at a unit cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EstimateLine {
    pub cost_code: Id<CostCode>,
    pub description: String,
    /// The quantity in thousandths: 2.5 is 2500.
    pub quantity_milli: i64,
    /// "sf", "cy", "ea", "ls".
    #[serde(default)]
    pub unit: String,
    pub unit_cost: Money,
}

impl EstimateLine {
    /// Quantity times unit cost, rounded to the cent.
    pub fn total(&self) -> Option<Money> {
        self.unit_cost.checked_mul_int(self.quantity_milli)?.checked_div_int(1000)
    }
}

/// An option priced apart from the base estimate (an add or a deduct); the ones accepted are counted in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alternate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub cost_code: Id<CostCode>,
    /// Added to the subtotal when accepted; negative for a deduct.
    pub amount: Money,
    #[serde(default)]
    pub accepted: bool,
}

/// A thing the company prices again and again: a unit of work or material at a unit cost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: Id<CatalogItem>,
    pub name: String,
    /// "sf", "cy", "ea", "lf".
    #[serde(default)]
    pub unit: String,
    pub unit_cost: Money,
    /// The cost code a line made from it goes under, when it is one the project has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_code: Option<Id<CostCode>>,
    /// Free text for finding it: "Concrete", "Electrical".
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub notes: String,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(CatalogItem, "catalog_item", trash);

/// One catalogue item in an assembly, with how much of it goes into one unit of the assembly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AssemblyPart {
    pub item: Id<CatalogItem>,
    /// Quantity in thousandths per unit of the assembly.
    pub quantity_milli: i64,
}

/// A group of catalogue items priced together: "Slab on grade, per sf" is concrete, mesh and finishing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Assembly {
    pub id: Id<Assembly>,
    pub name: String,
    /// What one unit of the assembly is: "sf", "lf", "ea".
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub notes: String,
    pub parts: Vec<AssemblyPart>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Assembly, "assembly", trash);

/// A percentage added to the subtotal: overhead, profit, contingency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Markup {
    pub name: String,
    pub percent: Percent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub id: Id<Estimate>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub name: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub status: EstimateStatus,
    #[serde(default)]
    pub lines: Vec<EstimateLine>,
    #[serde(default)]
    pub markups: Vec<Markup>,
    #[serde(default)]
    pub alternates: Vec<Alternate>,
    /// The estimate this one was copied from: a later version of the same pricing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on: Option<Id<Estimate>>,
    /// The estimate was put into the budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Estimate, "estimate", trash, project);

/// An estimate's figures.
#[derive(Clone, Debug, PartialEq)]
pub struct EstimateTotals {
    /// The lines and the accepted alternates; the markups are a share of this.
    pub subtotal: Money,
    /// The accepted alternates, already in the subtotal.
    pub alternates: Money,
    /// Each markup's amount, in order.
    pub markups: Vec<Money>,
    pub total: Money,
}

impl Estimate {
    pub fn label(&self) -> String {
        format!("Estimate #{}", self.number)
    }

    /// The subtotal, each markup (a percentage of the subtotal) and the grand total.
    pub fn totals(&self) -> Option<EstimateTotals> {
        let lines = Money::sum(self.lines.iter().map(EstimateLine::total).collect::<Option<Vec<_>>>()?)?;
        let alternates = Money::sum(self.alternates.iter().filter(|a| a.accepted).map(|a| a.amount))?;
        let subtotal = lines.checked_add(alternates)?;
        let markups: Vec<Money> = self.markups.iter().map(|m| subtotal.percent(m.percent)).collect::<Option<Vec<_>>>()?;
        let total = subtotal.checked_add(Money::sum(markups.iter().copied())?)?;
        Some(EstimateTotals { subtotal, alternates, markups, total })
    }
}
