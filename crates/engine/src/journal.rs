//! Journal entries: the record of every change.

use buildkiosk_core::{Id, Timestamp};
use buildkiosk_model::directory::Person;
use buildkiosk_model::{ItemRef, TableChange};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One executed command: who ran what, when, with which parameters, and the rows it changed.
/// Replaying `changes` in sequence rebuilds the company state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct JournalEntry {
    pub seq: u64,
    pub at: Timestamp,
    /// `None` for the system actor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<Id<Person>>,
    pub cmd: String,
    pub params: Value,
    pub changes: Vec<TableChange>,
}

impl JournalEntry {
    /// Whether the entry changed the row `id` of `table`.
    pub fn touches(&self, table: &str, id: u64) -> bool {
        self.changes.iter().any(|c| c.table == table && c.rows.iter().any(|r| r.id == id))
    }

    /// The (table, row id) pairs the entry changed.
    pub fn touched(&self) -> impl Iterator<Item = (&str, u64)> {
        self.changes.iter().flat_map(|c| c.rows.iter().map(move |r| (c.table.as_str(), r.id)))
    }
}

/// Where journal entries go. [`crate::Engine::execute`] appends before a change takes effect; an
/// error aborts the command.
pub trait JournalSink {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), String>;
}

/// Keeps entries in memory (tests, previews, a browser build).
#[derive(Clone, Debug, Default)]
pub struct MemoryJournal {
    pub entries: Vec<JournalEntry>,
}

impl JournalSink for MemoryJournal {
    fn append(&mut self, entry: &JournalEntry) -> Result<(), String> {
        self.entries.push(entry.clone());
        Ok(())
    }
}

/// Discards entries (dry runs and benchmarks).
#[derive(Clone, Copy, Debug, Default)]
pub struct NoJournal;

impl JournalSink for NoJournal {
    fn append(&mut self, _entry: &JournalEntry) -> Result<(), String> {
        Ok(())
    }
}

/// The table a kind of item lives in, for history lookups.
pub fn table_of(item: ItemRef) -> &'static str {
    use buildkiosk_model::ItemKind::*;
    match item.kind {
        Project => "projects",
        Person => "people",
        Vendor => "vendors",
        Rfi => "rfis",
        Submittal => "submittals",
        PunchItem => "punch_items",
        Observation => "observations",
        Inspection => "inspections",
        DailyLog => "daily_logs",
        Meeting => "meetings",
        Task => "tasks",
        Document => "documents",
        Drawing => "drawings",
        Photo => "photos",
        ChangeEvent => "change_events",
        ChangeOrder => "change_orders",
        Commitment => "commitments",
        PrimeContract => "prime_contracts",
        Invoice => "invoices",
        DirectCost => "direct_costs",
        Transmittal => "transmittals",
        Incident => "incidents",
        Specification => "spec_sections",
        Correspondence => "correspondence",
        Timesheet => "timesheets",
        BidPackage => "bid_packages",
    }
}
