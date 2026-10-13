//! Drawings: the project's sheet log, each sheet with a history of revisions.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    /// The design discipline a sheet belongs to.
    #[derive(Default)]
    pub enum Discipline {
        #[default]
        General = ("general", "General"),
        Civil = ("civil", "Civil"),
        Landscape = ("landscape", "Landscape"),
        Architectural = ("architectural", "Architectural"),
        Interior = ("interior", "Interior"),
        Structural = ("structural", "Structural"),
        Mechanical = ("mechanical", "Mechanical"),
        Plumbing = ("plumbing", "Plumbing"),
        FireProtection = ("fire_protection", "Fire protection"),
        Electrical = ("electrical", "Electrical"),
        Other = ("other", "Other"),
    }
}

/// One issue of a sheet. The newest revision is the current one; older ones stay as history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingRevision {
    /// What the design team calls it: "A", "2", "Addendum 3".
    pub label: String,
    /// The issue date printed on the sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    /// The set or package it arrived in: "Permit set", "Bulletin 4".
    #[serde(default)]
    pub set: String,
    #[serde(default)]
    pub note: String,
    /// The sheet file; none for a revision logged before the file arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: Id<Drawing>,
    pub project: Id<Project>,
    /// The sheet number as written on the drawing, unique in the project: "A-101".
    pub number: String,
    pub title: String,
    #[serde(default)]
    pub discipline: Discipline,
    /// The building, floor or zone it covers.
    #[serde(default)]
    pub area: String,
    /// Oldest first; empty for a sheet that is only on the log so far.
    #[serde(default)]
    pub revisions: Vec<DrawingRevision>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Drawing, "drawing", trash, project);

impl Drawing {
    pub fn current(&self) -> Option<&DrawingRevision> {
        self.revisions.last()
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.number, self.title).trim().to_string()
    }
}
