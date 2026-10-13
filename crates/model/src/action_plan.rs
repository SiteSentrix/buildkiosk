//! Action plans: a checklist of work in sections, each item with an owner and a due date.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum PlanStatus {
        /// Being written; items are not yet assigned out.
        #[default]
        Draft = ("draft", "Draft"),
        Active = ("active", "In progress"),
        Complete = ("complete", "Complete"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanSection {
    pub title: String,
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub id: Id<ActionPlan>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub status: PlanStatus,
    /// Runs the plan: edits it, activates it and marks it complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    pub sections: Vec<PlanSection>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(ActionPlan, "action_plan", trash, project);

impl ActionPlan {
    pub fn label(&self) -> String {
        format!("Action plan #{}", self.number)
    }

    /// (items done, items in all).
    pub fn progress(&self) -> (usize, usize) {
        let all = self.sections.iter().map(|s| s.items.len()).sum();
        let done = self.sections.iter().flat_map(|s| s.items.iter()).filter(|i| i.done).count();
        (done, all)
    }
}
