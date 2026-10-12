//! Observations: safety, quality and housekeeping findings. The workflow is shared with the punch
//! list (see [`crate::workitem`]); observations add the day observed and a corrective action.

use buildkiosk_core::{Id, text};
use buildkiosk_model::observation::Observation;
use buildkiosk_model::project::Project;
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, Priority, Table, Tool};

use super::CommandSpec;
use crate::ctx::Read;
use crate::workitem::WorkItem;
use crate::{Ctx, Params, Result};

pub const COMMANDS: &[CommandSpec] = crate::work_commands!(
    "observation",
    "observation",
    Tool::Observations,
    Observation,
    "; corrective_action?: text; observed?: YYYY-MM-DD (default: today)"
);

fn next_number(state: &CompanyState, project: Id<Project>) -> u32 {
    state.observations.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

impl WorkItem for Observation {
    const TOOL: Tool = Tool::Observations;
    const NOUN: &'static str = "observation";
    const LABEL: &'static str = "Observation";
    const PATH: &'static str = "observations";
    const ATTACH_CMD: &'static str = "observation.attach";
    const TABLE: &'static str = "observations";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.observations
    }

    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.observations
    }

    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        Observation {
            id: ctx.state.observations.peek_next(),
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
            observed: Some(ctx.today()),
            corrective_action: String::new(),
        }
    }

    fn set_id(&mut self, id: Id<Self>) {
        self.id = id;
    }

    fn apply_extra(&mut self, p: &Params<'_>) -> Result<()> {
        if p.present("observed") {
            self.observed = p.date("observed")?;
        }
        if p.present("corrective_action") {
            self.corrective_action = p.text_or_empty("corrective_action", text::LONG)?;
        }
        Ok(())
    }
}
