//! Incidents: injuries, near misses and damage. The workflow is shared with the punch list and
//! observations (see [`crate::workitem`]); an incident adds when it happened, how serious it was,
//! whether it is recordable, who was involved, the root cause and the corrective action.
//!
//! Incidents are sensitive: the Incidents tool is hidden from the subcontractor and owner
//! templates, and only people with access to the tool see them.

use buildkiosk_core::{Id, text};
use buildkiosk_model::incident::{Incident, Severity};
use buildkiosk_model::project::Project;
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, Priority, Table, Tool};

use super::CommandSpec;
use crate::ctx::Read;
use crate::workitem::WorkItem;
use crate::{Ctx, Params, Result};

pub const COMMANDS: &[CommandSpec] = crate::work_commands!(
    "incident",
    "incident",
    Tool::Incidents,
    Incident,
    "; occurred?: YYYY-MM-DD (default: today); severity?: near_miss|property_damage|first_aid|medical_treatment|lost_time|fatality; recordable?: bool; people_involved?: text; root_cause?: text; corrective_action?: text"
);

fn next_number(state: &CompanyState, project: Id<Project>) -> u32 {
    state.incidents.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

impl WorkItem for Incident {
    const TOOL: Tool = Tool::Incidents;
    const NOUN: &'static str = "incident";
    const LABEL: &'static str = "Incident";
    const PATH: &'static str = "incidents";
    const ATTACH_CMD: &'static str = "incident.attach";
    const TABLE: &'static str = "incidents";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.incidents
    }

    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.incidents
    }

    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        Incident {
            id: ctx.state.incidents.peek_next(),
            project,
            core: WorkCore {
                number: next_number(ctx.state, project),
                title: String::new(),
                description: String::new(),
                location: String::new(),
                kind: String::new(),
                priority: Priority::default(),
                status: WorkStatus::Open,
                manager: ctx.person(),
                assignees: Vec::new(),
                ball_in_court: Vec::new(),
                due: None,
                work_done: None,
                closed: None,
                attachments: Vec::new(),
                comments: Vec::new(),
                created: stamp,
                updated: stamp,
                deleted: None,
            },
            occurred: Some(ctx.today()),
            severity: Severity::default(),
            recordable: false,
            people_involved: String::new(),
            root_cause: String::new(),
            corrective_action: String::new(),
        }
    }

    fn set_id(&mut self, id: Id<Self>) {
        self.id = id;
    }

    fn apply_extra(&mut self, p: &Params<'_>) -> Result<()> {
        if p.present("occurred") {
            self.occurred = p.date("occurred")?;
        }
        if let Some(s) = p.choice::<Severity>("severity")? {
            self.severity = s;
        }
        if let Some(r) = p.flag("recordable")? {
            self.recordable = r;
        }
        for (key, field) in
            [("people_involved", &mut self.people_involved), ("root_cause", &mut self.root_cause), ("corrective_action", &mut self.corrective_action)]
        {
            if p.present(key) {
                *field = p.text_or_empty(key, text::LONG)?;
            }
        }
        Ok(())
    }
}
