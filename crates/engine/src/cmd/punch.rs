//! Punch list: small items of work to fix or finish before closeout. The workflow is shared with
//! observations (see [`crate::workitem`]).

use buildkiosk_core::Id;
use buildkiosk_model::project::Project;
use buildkiosk_model::punch::PunchItem;
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, Priority, Table, Tool};

use super::CommandSpec;
use crate::Ctx;
use crate::ctx::Read;
use crate::workitem::WorkItem;

pub const COMMANDS: &[CommandSpec] = crate::work_commands!("punch", "punch item", Tool::PunchList, PunchItem, "");

/// The next number for a new item of the project.
fn next_number(state: &CompanyState, project: Id<Project>) -> u32 {
    state.punch_items.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

impl WorkItem for PunchItem {
    const TOOL: Tool = Tool::PunchList;
    const NOUN: &'static str = "punch item";
    const LABEL: &'static str = "Punch item";
    const PATH: &'static str = "punch-list";
    const ATTACH_CMD: &'static str = "punch.attach";
    const TABLE: &'static str = "punch_items";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.punch_items
    }

    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.punch_items
    }

    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        PunchItem {
            id: ctx.state.punch_items.peek_next(),
            project,
            core: WorkCore {
                number: next_number(ctx.state, project),
                title: String::new(),
                description: String::new(),
                location: String::new(),
                drawing_number: String::new(),
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
        }
    }

    fn set_id(&mut self, id: Id<Self>) {
        self.id = id;
    }
}
