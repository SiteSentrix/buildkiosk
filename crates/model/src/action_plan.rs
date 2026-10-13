//! Action plans: a checklist of work in sections, each item with an owner and a due date.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
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
    /// Remarks people made on the item, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<PlanComment>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanComment {
    pub text: String,
    pub stamp: Stamp,
}

/// An approver's answer on a plan.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanApproval {
    pub person: Id<Person>,
    pub approved: bool,
    #[serde(default)]
    pub note: String,
    pub stamp: Stamp,
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
    /// People who must approve the plan before it starts (none: no approval is needed).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvers: Vec<Id<Person>>,
    /// The latest answer of each approver; any change to the items clears them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub approvals: Vec<PlanApproval>,
    /// Files attached to the plan itself (the programme, a site map, a method statement).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<FileRef>,
    /// Who marked the plan complete, and when (the sign-off).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(ActionPlan, "action_plan", trash, project);

/// A plan's sections kept without owners, dates or progress, to start other plans from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanTemplate {
    pub id: Id<PlanTemplate>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub sections: Vec<PlanSection>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PlanTemplate, "plan_template", trash);

impl ActionPlan {
    pub fn label(&self) -> String {
        format!("Action plan #{}", self.number)
    }

    /// Whether every named approver has approved the plan as it stands.
    pub fn approved(&self) -> bool {
        self.approvers.iter().all(|a| self.approvals.iter().any(|x| x.person == *a && x.approved))
    }

    /// (items done, items in all).
    pub fn progress(&self) -> (usize, usize) {
        let all = self.sections.iter().map(|s| s.items.len()).sum();
        let done = self.sections.iter().flat_map(|s| s.items.iter()).filter(|i| i.done).count();
        (done, all)
    }
}
