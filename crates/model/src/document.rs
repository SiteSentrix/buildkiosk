//! Documents: a project's folders and the files in them, with a version history.

use buildkiosk_core::{Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::record;

/// A folder. A private folder (and everything under it) is hidden from people who are neither
/// listed in `access` nor Documents admins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Id<Folder>,
    pub project: Id<Project>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id<Folder>>,
    pub name: String,
    #[serde(default)]
    pub private: bool,
    /// Who may open a private folder besides the Documents admins.
    #[serde(default)]
    pub access: Vec<Id<Person>>,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Folder, "folder", trash, project);

/// One uploaded revision of a document's file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DocVersion {
    /// 1 for the first upload, then counting up.
    pub number: u32,
    pub file: FileRef,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    #[serde(default)]
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub id: Id<Document>,
    pub project: Id<Project>,
    /// The folder it sits in; none is the project's top level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Id<Folder>>,
    pub title: String,
    #[serde(default)]
    pub description: String,
    /// Oldest first; never empty.
    pub versions: Vec<DocVersion>,
    /// A restricted document is opened only by its uploader, the people in `access` and Documents admins.
    #[serde(default)]
    pub restricted: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub access: Vec<Id<Person>>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Document, "document", trash, project);

impl Document {
    /// The newest version.
    pub fn current(&self) -> Option<&DocVersion> {
        self.versions.last()
    }
}
