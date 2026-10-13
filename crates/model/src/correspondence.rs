//! Correspondence: letters, notices and emails kept on the project record, with replies.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
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
        format!("Correspondence #{}", self.number)
    }
}
