//! Inspections: a checklist walked on site, answered item by item.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::punch::PunchItem;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum InspectionStatus {
        #[default]
        Open = ("open", "In progress"),
        Completed = ("completed", "Completed"),
    }
}

choice! {
    /// The answer to one checklist item.
    pub enum Answer {
        Pass = ("pass", "Pass"),
        Fail = ("fail", "Fail"),
        NotApplicable = ("na", "N/A"),
    }
}

/// One line of a template or of an inspection's checklist, under a section heading.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TemplateItem {
    #[serde(default)]
    pub section: String,
    pub text: String,
}

/// A reusable checklist.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct InspectionTemplate {
    pub id: Id<InspectionTemplate>,
    pub project: Id<Project>,
    pub name: String,
    /// Free text: concrete, framing, safety…
    #[serde(default)]
    pub kind: String,
    pub items: Vec<TemplateItem>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(InspectionTemplate, "inspection_template", trash, project);

/// A checklist line of an inspection: copied from the template when the inspection starts, so
/// later template changes never rewrite a finished inspection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    #[serde(default)]
    pub section: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Answer>,
    #[serde(default)]
    pub note: String,
    /// The punch item raised for this failed line, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub punch: Option<Id<PunchItem>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub id: Id<Inspection>,
    pub project: Id<Project>,
    /// Numbered within the project.
    pub number: u32,
    pub title: String,
    /// The template it was made from, by name (kept as text).
    #[serde(default)]
    pub template_name: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub status: InspectionStatus,
    /// Walks the checklist and answers it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspector: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scheduled: Option<Date>,
    pub items: Vec<ChecklistItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed: Option<Stamp>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Inspection, "inspection", trash, project);

impl Inspection {
    pub fn label(&self) -> String {
        format!("Inspection #{}", self.number)
    }

    pub fn answered(&self) -> usize {
        self.items.iter().filter(|i| i.answer.is_some()).count()
    }

    pub fn failed(&self) -> usize {
        self.items.iter().filter(|i| i.answer == Some(Answer::Fail)).count()
    }

    /// Failed lines that still have no punch item.
    pub fn unraised(&self) -> usize {
        self.items.iter().filter(|i| i.answer == Some(Answer::Fail) && i.punch.is_none()).count()
    }

    pub fn is_overdue(&self, today: Date) -> bool {
        self.status == InspectionStatus::Open && self.scheduled.is_some_and(|d| d < today)
    }
}
