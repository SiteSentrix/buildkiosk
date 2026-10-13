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

choice! {
    /// What a markup draws on a sheet.
    pub enum MarkupKind {
        /// A point with a note.
        Pin = ("pin", "Pin"),
        /// A box around an area.
        Rectangle = ("rect", "Box"),
        /// An arrow from (x, y) to (x2, y2).
        Arrow = ("arrow", "Arrow"),
        /// Words placed at (x, y).
        Text = ("text", "Text"),
    }
}

/// Where and how far across a page things are placed: 0 to this, left to right and top to bottom.
pub const MARKUP_SCALE: u16 = 10_000;

/// A note drawn on one page of one revision. Positions are fractions of the page
/// (`0..=MARKUP_SCALE`), so they stay right at any zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Markup {
    /// Counts up within the revision.
    pub id: u32,
    /// The page, from 1.
    pub page: u32,
    pub kind: MarkupKind,
    pub x: u16,
    pub y: u16,
    /// The far corner of a box, or the arrow's head.
    #[serde(default)]
    pub x2: u16,
    #[serde(default)]
    pub y2: u16,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
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
    /// Notes drawn on its pages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markups: Vec<Markup>,
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
