//! The BuildKiosk data model (layer 1).
//!
//! One [`CompanyState`] holds everything a company has: its directory, permission templates,
//! projects and every project tool's records, each in a copy-on-write [`Table`]. The engine edits
//! it only through commands; [`CompanyState::diff`] turns each command's effect into row-level
//! changes for the journal, and [`CompanyState::apply`] replays them. Pure data: no I/O, no clock,
//! builds for wasm32.
#![forbid(unsafe_code)]

pub mod choice;
pub mod common;
pub mod daily_log;
pub mod directory;
pub mod notification;
pub mod observation;
pub mod perm;
pub mod project;
pub mod punch;
pub mod rfi;
pub mod spec;
pub mod state;
pub mod submittal;
pub mod table;
pub mod tool;
// Tool modules, one region per area (keeps parallel work in separate places).
// -- project management (submittals, transmittals, specifications, correspondence)

// -- quality and safety (punch list, observations, inspections, incidents)

// -- field (daily log, meetings, schedule, timesheets)

// -- financials (budget, change events and orders, commitments, prime contract, invoicing, direct costs)

// -- documents (documents, drawings, photos)

// -- collaboration (comments, related items)

pub use choice::Choice;
pub use common::{Address, FileRef, Impact, InProject, ItemKind, ItemRef, Priority, Stamp, Trash};
pub use state::{CompanyState, SCHEMA, SnapshotIn, SnapshotOut};
pub use table::{Record, RowChange, Table, TableChange, TableData};
pub use tool::{Level, Scope, Tool, ToolGroup};

#[doc(hidden)]
pub use serde as __serde;

/// Why stored data couldn't be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("unknown table `{0}` (the data was written by a newer BuildKiosk)")]
    UnknownTable(String),
    #[error("table `{table}`: {msg}")]
    Decode { table: String, msg: String },
    #[error("table `{table}`: cannot encode a row: {msg}")]
    Encode { table: String, msg: String },
    #[error("the data uses schema {found}, but this build reads up to {supported}: upgrade BuildKiosk")]
    NewerSchema { found: u32, supported: u32 },
}
