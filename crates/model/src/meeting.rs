//! Meetings: agenda, attendance, minutes and action items.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    #[derive(Default)]
    pub enum MeetingStatus {
        /// Planned or held; the minutes are still being written.
        #[default]
        Scheduled = ("scheduled", "Scheduled"),
        /// The minutes were issued to the attendees.
        Issued = ("issued", "Minutes issued"),
    }
}

/// One topic on the agenda, with what was said about it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgendaItem {
    pub title: String,
    #[serde(default)]
    pub notes: String,
}

/// Someone invited, and whether they came.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Attendee {
    pub person: Id<Person>,
    #[serde(default)]
    pub present: bool,
}

/// Something someone agreed to do.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeetingAction {
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<Id<Person>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
    #[serde(default)]
    pub done: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meeting {
    pub id: Id<Meeting>,
    pub project: Id<Project>,
    /// Sequential per project, starting at 1.
    pub number: u32,
    pub title: String,
    /// What kind of meeting: "Owner-architect-contractor", "Safety", "Coordination".
    #[serde(default)]
    pub category: String,
    pub date: Date,
    /// Free text: "9:00 AM".
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub status: MeetingStatus,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default)]
    pub agenda: Vec<AgendaItem>,
    #[serde(default)]
    pub actions: Vec<MeetingAction>,
    /// General minutes beyond the agenda items.
    #[serde(default)]
    pub minutes: String,
    #[serde(default)]
    pub attachments: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued: Option<Stamp>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Meeting, "meeting", trash, project);

impl Meeting {
    pub fn label(&self) -> String {
        format!("Meeting #{}", self.number)
    }
}
