//! Forms: a project's own fill-in templates (checklists, reports) and the records filled in from them.

use std::collections::BTreeMap;

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::project::Project;
use crate::{choice, record};

choice! {
    /// What a field holds.
    #[derive(Default)]
    pub enum FieldKind {
        #[default]
        Text = ("text", "Short text"),
        LongText = ("long_text", "Long text"),
        Number = ("number", "Number"),
        Date = ("date", "Date"),
        YesNo = ("yes_no", "Yes or no"),
        Choice = ("choice", "One of a list"),
    }
}

/// One question on a form.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormField {
    /// Identifies the field in a record's answers ("f1", "f2"…).
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub kind: FieldKind,
    #[serde(default)]
    pub required: bool,
    /// The allowed answers of a `Choice` field.
    #[serde(default)]
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormTemplate {
    pub id: Id<FormTemplate>,
    pub project: Id<Project>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub fields: Vec<FormField>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(FormTemplate, "form_template", trash, project);

choice! {
    #[derive(Default)]
    pub enum FormStatus {
        #[default]
        Draft = ("draft", "Draft"),
        Submitted = ("submitted", "Submitted"),
    }
}

/// A form filled in. It keeps a copy of the template's fields as they were, so editing the
/// template later does not change what was recorded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormRecord {
    pub id: Id<FormRecord>,
    pub project: Id<Project>,
    pub template: Id<FormTemplate>,
    /// The template's name when the form was started.
    pub template_name: String,
    /// Sequential per project, starting at 1.
    pub number: u32,
    #[serde(default)]
    pub title: String,
    pub fields: Vec<FormField>,
    /// Field key → the answer as text.
    #[serde(default)]
    pub answers: BTreeMap<String, String>,
    #[serde(default)]
    pub status: FormStatus,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submitted: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(FormRecord, "form_record", trash, project);

impl FormRecord {
    pub fn label(&self) -> String {
        format!("{} #{}", self.template_name, self.number)
    }
}
