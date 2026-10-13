//! Correspondence: letters, notices and emails kept on the project record, with replies.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::document::Document;
use crate::project::Project;
use crate::rfi::Rfi;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum CorrespondenceStatus {
        /// Being written; only its author and admins see it.
        #[default]
        Draft = ("draft", "Draft"),
        /// Sent; a reply may be wanted.
        Open = ("open", "Open"),
        Closed = ("closed", "Closed"),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Correspondence {
    pub id: Id<Correspondence>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub subject: String,
    /// "Letter", "Notice", "Email", "Memo"…
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub status: CorrespondenceStatus,
    #[serde(default)]
    pub to: Vec<Id<Person>>,
    #[serde(default)]
    pub cc: Vec<Id<Person>>,
    /// The date on the letter.
    pub date: Date,
    /// A reply is wanted by this day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_by: Option<Date>,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub replies: Vec<Reply>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    /// Only the author, the recipients and admins see a private item.
    #[serde(default)]
    pub private: bool,
    /// The configured type it was filed under, and its number within that type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<Id<CorrespondenceType>>,
    #[serde(default)]
    pub type_number: u32,
    /// "LTR-003": the type's prefix and number; empty when the project has no configured types.
    #[serde(default)]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rfis: Vec<Id<Rfi>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub documents: Vec<Id<Document>>,
    /// Set for a message that came from outside the company and was filed here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incoming: Option<Sender>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Stamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Correspondence, "correspondence", trash, project);

impl Correspondence {
    pub fn label(&self) -> String {
        if self.reference.is_empty() { format!("Correspondence #{}", self.number) } else { self.reference.clone() }
    }
}

/// Who an incoming message is from, when they are not in the directory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sender {
    pub name: String,
    #[serde(default)]
    pub email: String,
}

/// A kind of correspondence a project has configured, with its own numbering ("Letter", "LTR").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CorrespondenceType {
    pub id: Id<CorrespondenceType>,
    pub project: Id<Project>,
    pub name: String,
    /// Starts the reference: LTR-001, LTR-002…
    pub prefix: String,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(CorrespondenceType, "correspondence_type", trash, project);
