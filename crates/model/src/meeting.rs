//! Meetings: agenda, attendance, minutes and action items.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::task::Task;
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
    /// Brought forward to a later meeting, which now carries it: it no longer counts as open here.
    #[serde(default)]
    pub carried: bool,
    /// The task made from this action (closing the task marks the action done).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Id<Task>>,
    /// The earlier meeting this action was carried forward from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_meeting: Option<Id<Meeting>>,
}

impl MeetingAction {
    /// Still to do, and not handed on to a later meeting.
    pub fn is_open(&self) -> bool {
        !self.done && !self.carried
    }
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

/// A meeting set up once and used again: the type, time, place, people and agenda topics of a recurring kind of meeting.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeetingTemplate {
    pub id: Id<MeetingTemplate>,
    pub project: Id<Project>,
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub time: String,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub invitees: Vec<Id<Person>>,
    /// The agenda topics, in order.
    #[serde(default)]
    pub agenda: Vec<String>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(MeetingTemplate, "meeting_template", trash, project);
