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
    pub subtotal: Money,
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
        let subtotal = Money::sum(self.lines.iter().map(EstimateLine::total).collect::<Option<Vec<_>>>()?)?;
        let markups: Vec<Money> = self.markups.iter().map(|m| subtotal.percent(m.percent)).collect::<Option<Vec<_>>>()?;
        let total = subtotal.checked_add(Money::sum(markups.iter().copied())?)?;
        Some(EstimateTotals { subtotal, markups, total })
    }
}
