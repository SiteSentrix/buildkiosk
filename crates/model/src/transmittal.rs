//! Transmittals: a formal record that documents were sent to someone, and why.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::document::Document;
use crate::drawing::Drawing;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum TransmittalStatus {
        /// Being written; only its author and admins see it.
        #[default]
        Draft = ("draft", "Draft"),
        Sent = ("sent", "Sent"),
    }
}

choice! {
    /// Why the items are being sent.
    #[derive(Default)]
    pub enum Purpose {
        #[default]
        ForInformation = ("for_information", "For information"),
        ForReview = ("for_review", "For review"),
        ForApproval = ("for_approval", "For approval"),
        ForConstruction = ("for_construction", "For construction"),
        ForRecord = ("for_record", "For the record"),
        AsRequested = ("as_requested", "As requested"),
    }
}

/// One thing being sent: a drawing set, a letter, a box of samples.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TransmittalItem {
    pub description: String,
    #[serde(default)]
    pub copies: u32,
    /// The project document this item is (its current version when the transmittal was sent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<Id<Document>>,
    /// The drawing sheet this item is (its current revision when the transmittal was sent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drawing: Option<Id<Drawing>>,
    /// The version or revision that went out ("v3", "rev C"); filled in from the document or drawing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub revision: String,
}

/// A recipient saying (or being recorded as saying) the transmittal reached them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub person: Id<Person>,
    /// When it was recorded and by whom (the recipient, or the sender for a paper or phone receipt).
    pub stamp: Stamp,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Transmittal {
    pub id: Id<Transmittal>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub subject: String,
    #[serde(default)]
    pub status: TransmittalStatus,
    #[serde(default)]
    pub purpose: Purpose,
    #[serde(default)]
    pub to: Vec<Id<Person>>,
    #[serde(default)]
    pub cc: Vec<Id<Person>>,
    /// When a response is wanted by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub items: Vec<TransmittalItem>,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    /// Who has acknowledged receiving it.
    #[serde(default)]
    pub receipts: Vec<Receipt>,
    /// Only the author, the recipients and admins see a private transmittal.
    #[serde(default)]
    pub private: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Transmittal, "transmittal", trash, project);

impl Transmittal {
    pub fn label(&self) -> String {
        format!("Transmittal #{}", self.number)
    }
}
