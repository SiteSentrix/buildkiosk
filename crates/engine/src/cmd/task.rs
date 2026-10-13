//! Tasks: general to-do items. The workflow is shared with the punch list and observations (see
//! [`crate::workitem`]); a task adds nothing of its own.

use buildkiosk_core::Id;
use buildkiosk_model::project::Project;
use buildkiosk_model::task::Task;
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, Priority, Table, Tool};

use super::CommandSpec;
use crate::Ctx;
use crate::ctx::Read;
use crate::workitem::WorkItem;

pub const COMMANDS: &[CommandSpec] = crate::work_commands!("task", "task", Tool::Tasks, Task, "");

fn next_number(state: &CompanyState, project: Id<Project>) -> u32 {
    state.tasks.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

impl WorkItem for Task {
    const TOOL: Tool = Tool::Tasks;
    const NOUN: &'static str = "task";
    const LABEL: &'static str = "Task";
    const PATH: &'static str = "tasks";
    const ATTACH_CMD: &'static str = "task.attach";
    const TABLE: &'static str = "tasks";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.tasks
    }

    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.tasks
    }

    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        Task {
            id: ctx.state.tasks.peek_next(),
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
