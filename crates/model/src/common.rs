//! Types shared by many records.

use buildkiosk_core::{Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::choice;
use crate::directory::Person;
use crate::project::Project;
use crate::table::Record;

/// Who did something, and when. `by` is `None` for the system (imports, maintenance).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
}

/// A file in the blob store, addressed by the SHA-256 of its content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRef {
    pub sha256: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
}

/// A postal address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Address {
    #[serde(default)]
    pub street: String,
    #[serde(default)]
    pub city: String,
    #[serde(default)]
    pub region: String,
    #[serde(default)]
    pub postal_code: String,
    #[serde(default)]
    pub country: String,
}

impl Address {
    pub fn is_empty(&self) -> bool {
        [&self.street, &self.city, &self.region, &self.postal_code, &self.country].iter().all(|s| s.is_empty())
    }

    /// "12 Main St, Springfield, OR 97477".
    pub fn one_line(&self) -> String {
        let region_zip = [self.region.as_str(), self.postal_code.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
        [self.street.as_str(), self.city.as_str(), region_zip.as_str(), self.country.as_str()]
            .iter()
            .filter(|s| !s.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(", ")
    }
}

choice! {
    /// How urgent an item is.
    #[derive(Default)]
    pub enum Priority {
        Low = ("low", "Low"),
        #[default]
        Medium = ("medium", "Medium"),
        High = ("high", "High"),
        Urgent = ("urgent", "Urgent"),
    }
}

choice! {
    /// Whether an item affects cost or schedule.
    #[derive(Default)]
    pub enum Impact {
        #[default]
        Tbd = ("tbd", "To be determined"),
        No = ("no", "No"),
        Yes = ("yes", "Yes"),
    }
}

choice! {
    /// The kind of a record, for references between items (related items, history, links).
    pub enum ItemKind {
        Project = ("project", "Project"),
        Person = ("person", "Person"),
        Vendor = ("vendor", "Company"),
        Rfi = ("rfi", "RFI"),
        Submittal = ("submittal", "Submittal"),
        PunchItem = ("punch_item", "Punch item"),
        Observation = ("observation", "Observation"),
        Inspection = ("inspection", "Inspection"),
        DailyLog = ("daily_log", "Daily log"),
        Meeting = ("meeting", "Meeting"),
        Task = ("task", "Schedule task"),
        Document = ("document", "Document"),
        Drawing = ("drawing", "Drawing"),
        Photo = ("photo", "Photo"),
        ChangeEvent = ("change_event", "Change event"),
        ChangeOrder = ("change_order", "Change order"),
        Commitment = ("commitment", "Commitment"),
        PrimeContract = ("prime_contract", "Prime contract"),
        Invoice = ("invoice", "Invoice"),
        DirectCost = ("direct_cost", "Direct cost"),
        Transmittal = ("transmittal", "Transmittal"),
        Incident = ("incident", "Incident"),
        Specification = ("specification", "Specification section"),
        Correspondence = ("correspondence", "Correspondence"),
        Timesheet = ("timesheet", "Timesheet"),
        BidPackage = ("bid_package", "Bid package"),
    }
}

/// A reference to any record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ItemRef {
    pub kind: ItemKind,
    pub id: u64,
}

/// A record that belongs to one project. Commands check it so that an id from another project
/// can't be reached through a project the person has access to.
pub trait InProject: Record {
    fn project(&self) -> Id<Project>;
}

/// A record that goes to the recycle bin when deleted and can be restored from it.
pub trait Trash: Record {
    fn deleted(&self) -> Option<Stamp>;
    fn set_deleted(&mut self, stamp: Option<Stamp>);
    fn is_deleted(&self) -> bool {
        self.deleted().is_some()
    }
}

/// Implements [`Record`], and [`Trash`] when the record has a `deleted` field, and [`InProject`]
/// when it has a `project` field.
#[macro_export]
macro_rules! record {
    ($ty:ty, $kind:literal) => {
        impl $crate::Record for $ty {
            const KIND: &'static str = $kind;
            fn id(&self) -> ::buildkiosk_core::Id<Self> {
                self.id
            }
        }
    };
    ($ty:ty, $kind:literal, trash) => {
        $crate::record!($ty, $kind);
        impl $crate::Trash for $ty {
            fn deleted(&self) -> Option<$crate::Stamp> {
                self.deleted
            }
            fn set_deleted(&mut self, stamp: Option<$crate::Stamp>) {
                self.deleted = stamp;
            }
        }
    };
    ($ty:ty, $kind:literal, trash, project) => {
        $crate::record!($ty, $kind, trash);
        impl $crate::InProject for $ty {
            fn project(&self) -> ::buildkiosk_core::Id<$crate::project::Project> {
                self.project
            }
        }
    };
    ($ty:ty, $kind:literal, project) => {
        $crate::record!($ty, $kind);
        impl $crate::InProject for $ty {
            fn project(&self) -> ::buildkiosk_core::Id<$crate::project::Project> {
                self.project
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_one_line() {
        let a = Address {
            street: "12 Main St".into(),
            city: "Springfield".into(),
            region: "OR".into(),
            postal_code: "97477".into(),
            country: String::new(),
        };
        assert_eq!(a.one_line(), "12 Main St, Springfield, OR 97477");
        assert_eq!(Address::default().one_line(), "");
        assert!(Address::default().is_empty());
    }
}
