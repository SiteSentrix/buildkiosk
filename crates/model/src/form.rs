//! Forms: a project's own fill-in templates (checklists, reports) and the records filled in from them.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
pub use crate::task::Repeat;
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
    /// The question is asked only when an earlier question has this answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show_if: Option<ShowIf>,
}

/// A condition on an earlier question's answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShowIf {
    /// The key of the earlier question ("f2").
    pub field: String,
    /// The answer that shows the question (compared without regard to case).
    pub equals: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormTemplate {
    pub id: Id<FormTemplate>,
    pub project: Id<Project>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub fields: Vec<FormField>,
    /// Roles that sign a submitted form ("Inspector", "Superintendent").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signers: Vec<String>,
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
    /// The roles that must sign, copied from the template.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<FormSignature>,
    /// Who fills it in when it was made by a schedule (they may edit the draft like its author).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Id<Person>>,
    /// The schedule that made it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<Id<FormSchedule>>,
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

/// A role's signature on a submitted form: the signer typed their own name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormSignature {
    pub role: String,
    pub person: Id<Person>,
    pub name: String,
    pub stamp: Stamp,
}

impl FormRecord {
    /// The questions that apply given the answers so far (a conditional question whose condition is not met is left out).
    pub fn shown_fields(&self) -> Vec<&FormField> {
        let mut out: Vec<&FormField> = Vec::new();
        for f in &self.fields {
            let on = f.show_if.as_ref().is_none_or(|c| {
                out.iter().any(|x| x.key == c.field) && self.answers.get(&c.field).is_some_and(|a| a.eq_ignore_ascii_case(&c.equals))
            });
            if on {
                out.push(f);
            }
        }
        out
    }

    /// The roles that have not signed yet.
    pub fn unsigned(&self) -> Vec<&str> {
        self.signers.iter().filter(|r| !self.signatures.iter().any(|s| s.role.eq_ignore_ascii_case(r))).map(String::as_str).collect()
    }
}

/// A form that is started again on a rhythm for someone to fill in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FormSchedule {
    pub id: Id<FormSchedule>,
    pub project: Id<Project>,
    pub template: Id<FormTemplate>,
    #[serde(default)]
    pub title: String,
    pub every: Repeat,
    /// The next day a form is made.
    pub next: Date,
    pub assignee: Id<Person>,
    #[serde(default = "yes")]
    pub active: bool,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(FormSchedule, "form_schedule", trash, project);

fn yes() -> bool {
    true
}
