//! Specification sections: the project's spec book, one row per section.
//!
//! Numbers and titles are whatever the project's specifications use: BuildKiosk ships no
//! classification list (AGENTS.md › Clean-room), and people type or import their own.

use buildkiosk_core::{Date, Id};
use serde::{Deserialize, Serialize};

use crate::common::{FileRef, Stamp};
use crate::project::Project;
use crate::record;

/// One section of a project's specifications.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpecSection {
    pub id: Id<SpecSection>,
    pub project: Id<Project>,
    /// The section number as the project writes it ("08 71 00", "16400", "B-12"). Unique in the
    /// project among sections that aren't in the recycle bin, ignoring case and repeated spaces.
    pub number: String,
    pub title: String,
    /// The division or chapter the section belongs to, as the project names it.
    #[serde(default)]
    pub division: String,
    /// The revision label of the current issue ("0", "B", "Addendum 2").
    #[serde(default)]
    pub revision: String,
    /// When the current revision was issued.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issued: Option<Date>,
    /// The section's document in the blob store.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(SpecSection, "specification", trash, project);

impl SpecSection {
    /// "08 71 00 Door hardware".
    pub fn label(&self) -> String {
        if self.title.is_empty() { self.number.clone() } else { format!("{} {}", self.number, self.title) }
    }

    /// Whether two section numbers are the same number: case and runs of spaces don't count.
    pub fn same_number(a: &str, b: &str) -> bool {
        let mut x = a.split_whitespace();
        let mut y = b.split_whitespace();
        loop {
            match (x.next(), y.next()) {
                (None, None) => return true,
                (Some(p), Some(q)) if p.to_lowercase() == q.to_lowercase() => {}
                _ => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_without_case_or_extra_spaces() {
        assert!(SpecSection::same_number("08 71 00", " 08  71 00 "));
        assert!(SpecSection::same_number("b-12a", "B-12A"));
        assert!(!SpecSection::same_number("08 71 00", "087100"));
        assert!(!SpecSection::same_number("08 71", "08 71 00"));
        assert!(SpecSection::same_number("", "  "));
    }
}
