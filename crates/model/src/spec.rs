//! Specification sections: the list of sections a project's specifications are divided into.
//! Users enter or import their own numbering; BuildKiosk ships no standard list.

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::project::Project;
use crate::record;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecSection {
    pub id: Id<SpecSection>,
    pub project: Id<Project>,
    /// The section number as written, e.g. "23 74 00".
    pub number: String,
    pub title: String,
    pub created: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(SpecSection, "spec_section", trash, project);

impl SpecSection {
    pub fn display(&self) -> String {
        if self.title.is_empty() { self.number.clone() } else { format!("{} {}", self.number, self.title) }
    }
}
