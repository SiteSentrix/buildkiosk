//! The company state: every table of one company.

use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::directory::{Company, DistributionGroup, Person, Vendor};
use crate::notification::Notification;
use crate::perm::{PermissionTemplate, ProjectMember};
use crate::project::{CostCode, Project};
use crate::rfi::Rfi;
use crate::table::{Table, TableChange, TableData};
// -- project management
use crate::action_plan::{ActionPlan, PlanTemplate};
use crate::bidding::BidPackage;
use crate::correspondence::Correspondence;
use crate::document::{Document, Folder};
use crate::drawing::Drawing;
use crate::estimate::{Assembly, CatalogItem, Estimate};
use crate::form::{FormRecord, FormTemplate};
use crate::incident::Incident;
use crate::meeting::Meeting;
use crate::photo::Photo;
use crate::prequal::{PrequalForm, Prequalification};
use crate::saved_report::SavedReport;
use crate::schedule::ScheduleTask;
use crate::spec::SpecSection;
use crate::submittal::Submittal;
use crate::task::Task;
use crate::timesheet::TimeEntry;
use crate::transmittal::Transmittal;

// -- quality and safety
use crate::inspection::{Inspection, InspectionTemplate};
use crate::observation::Observation;
use crate::punch::PunchItem;

// -- field
use crate::daily_log::DailyLog;

// -- financials
use crate::finance::{
    BudgetChange, BudgetLine, BudgetSnapshot, BudgetView, ChangeEvent, Commitment, CommitmentChange, CommitmentInvoice, DirectCost, OwnerInvoice,
    PrimeChange, PrimeContract,
};

// -- documents

// -- collaboration

/// Version of the stored data. Bump it when a change would make older builds misread new data
/// (a renamed field, a new meaning); older builds refuse to open newer data.
pub const SCHEMA: u32 = 1;

/// Declares [`CompanyState`] from its tables. The field name is the table's stable name in journals
/// and snapshots: add tables freely, never rename one.
macro_rules! tables {
    ($($(#[$doc:meta])* $field:ident : $ty:ty),* $(,)?) => {
        /// Everything one company has: the document every command edits. Cloning is cheap (see
        /// [`Table`]).
        #[derive(Clone, Debug, Default, PartialEq)]
        pub struct CompanyState {
            $($(#[$doc])* pub $field: Table<$ty>,)*
        }

        impl CompanyState {
            /// Table names as they appear in journals and snapshots.
            pub const TABLES: &'static [&'static str] = &[$(stringify!($field)),*];

            /// The changes that turn `self` into `newer`, one entry per changed table.
            pub fn diff(&self, newer: &Self) -> Result<Vec<TableChange>, ModelError> {
                let mut out = Vec::new();
                $(
                    if let Some(ch) = self.$field.diff(stringify!($field), &newer.$field)? {
                        out.push(ch);
                    }
                )*
                Ok(out)
            }

            /// Applies one recorded change (journal replay).
            pub fn apply(&mut self, change: &TableChange) -> Result<(), ModelError> {
                match change.table.as_str() {
                    $(stringify!($field) => self.$field.apply(change),)*
                    other => Err(ModelError::UnknownTable(other.to_string())),
                }
            }

            /// The state as a snapshot after journal entry `seq`.
            pub fn snapshot(&self, seq: u64) -> SnapshotOut<'_> {
                SnapshotOut { schema: SCHEMA, seq, $($field: self.$field.to_data(),)* }
            }

            /// Rows per table, for statistics.
            pub fn row_counts(&self) -> Vec<(&'static str, usize)> {
                vec![$((stringify!($field), self.$field.len()),)*]
            }
        }

        /// A snapshot being written: borrows the rows.
        #[derive(Serialize)]
        pub struct SnapshotOut<'a> {
            pub schema: u32,
            pub seq: u64,
            $(pub $field: TableData<&'a $ty>,)*
        }

        /// A snapshot being read. Unknown tables are refused: they come from a newer build.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct SnapshotIn {
            pub schema: u32,
            pub seq: u64,
            $(#[serde(default)] pub $field: TableData<$ty>,)*
        }

        impl SnapshotIn {
            /// The state and the journal sequence number it was taken after.
            pub fn into_state(self) -> Result<(CompanyState, u64), ModelError> {
                if self.schema > SCHEMA {
                    return Err(ModelError::NewerSchema { found: self.schema, supported: SCHEMA });
                }
                Ok((CompanyState { $($field: Table::from_data(stringify!($field), self.$field)?,)* }, self.seq))
            }
        }
    };
}

tables! {
    /// The company itself (one row).
    company: Company,
    people: Person,
    vendors: Vendor,
    prequalifications: Prequalification,
    prequal_forms: PrequalForm,
    saved_reports: SavedReport,
    groups: DistributionGroup,
    templates: PermissionTemplate,
    projects: Project,
    members: ProjectMember,
    cost_codes: CostCode,
    rfis: Rfi,
    // -- project management
    submittals: Submittal,
    transmittals: Transmittal,
    correspondence: Correspondence,
    meetings: Meeting,
    form_templates: FormTemplate,
    form_records: FormRecord,
    tasks: Task,
    spec_sections: SpecSection,

    // -- quality and safety
    punch_items: PunchItem,
    inspection_templates: InspectionTemplate,
    inspections: Inspection,
    observations: Observation,
    action_plans: ActionPlan,
    plan_templates: PlanTemplate,
    incidents: Incident,

    // -- field
    daily_logs: DailyLog,
    time_entries: TimeEntry,
    schedule_tasks: ScheduleTask,

    // -- financials
    budget_lines: BudgetLine,
    commitments: Commitment,
    direct_costs: DirectCost,
    change_events: ChangeEvent,
    commitment_changes: CommitmentChange,
    budget_changes: BudgetChange,
    budget_snapshots: BudgetSnapshot,
    budget_views: BudgetView,
    prime_contracts: PrimeContract,
    prime_changes: PrimeChange,
    owner_invoices: OwnerInvoice,
    commitment_invoices: CommitmentInvoice,
    bid_packages: BidPackage,
    estimates: Estimate,
    catalog_items: CatalogItem,
    assemblies: Assembly,

    // -- documents
    folders: Folder,
    documents: Document,
    drawings: Drawing,
    photos: Photo,

    // -- collaboration
    notifications: Notification,
}

impl CompanyState {
    /// The company row, if the company has been set up.
    pub fn company(&self) -> Option<&Company> {
        self.company.get(Company::ID)
    }

    /// Minutes east of UTC for the company's "today".
    pub fn utc_offset(&self) -> i32 {
        self.company().map(|c| c.utc_offset_minutes).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Stamp;
    use buildkiosk_core::{Id, Timestamp};

    fn stamp() -> Stamp {
        Stamp { by: None, at: Timestamp::from_millis(0) }
    }

    fn vendor(id: u64, name: &str) -> Vendor {
        Vendor {
            id: Id::new(id),
            name: name.into(),
            kind: Default::default(),
            trade: String::new(),
            email: String::new(),
            phone: String::new(),
            website: String::new(),
            address: Default::default(),
            license_number: String::new(),
            notes: String::new(),
            created: stamp(),
            updated: stamp(),
            deleted: None,
        }
    }

    #[test]
    fn diff_apply_and_snapshot_round_trip() {
        let a = CompanyState::default();
        let mut b = a.clone();
        b.vendors.put(vendor(1, "Acme Electric"));
        b.vendors.put(vendor(2, "Bolt Steel"));
        let changes = a.diff(&b).unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].table, "vendors");

        let mut replayed = a.clone();
        for ch in &changes {
            replayed.apply(ch).unwrap();
        }
        assert_eq!(replayed, b);

        let json = serde_json::to_string(&b.snapshot(7)).unwrap();
        let back: SnapshotIn = serde_json::from_str(&json).unwrap();
        let (state, seq) = back.into_state().unwrap();
        assert_eq!(seq, 7);
        assert_eq!(state, b);
    }

    #[test]
    fn refuses_unknown_tables_and_newer_schemas() {
        let ch = TableChange { table: "mystery".into(), next: None, rows: vec![] };
        assert!(CompanyState::default().apply(&ch).is_err());
        assert!(serde_json::from_str::<SnapshotIn>(r#"{"schema":1,"seq":0,"mystery":{"next":1,"rows":[]}}"#).is_err());
        let newer: SnapshotIn = serde_json::from_str(r#"{"schema":99,"seq":0}"#).unwrap();
        assert!(newer.into_state().is_err());
    }
}
