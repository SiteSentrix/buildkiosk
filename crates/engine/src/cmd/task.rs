//! Tasks: general to-do items. The workflow is shared with the punch list and observations (see
//! [`crate::workitem`]); a task adds subtasks, repeating, links to the RFI or meeting it came from,
//! and a list of a person's tasks across projects.

use buildkiosk_core::{Date, Id};
use buildkiosk_model::meeting::Meeting;
use buildkiosk_model::project::Project;
use buildkiosk_model::rfi::Rfi;
use buildkiosk_model::task::{Repeat, Task};
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, Level, Priority, Table, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::workitem::WorkItem;
use crate::{Ctx, EngineError, Params, Result, View, view};

pub const COMMANDS: &[CommandSpec] = crate::work_commands!(
    "task",
    "task",
    Tool::Tasks,
    Task,
    "; parent?: task id (makes this a subtask; one level only, and a subtask can't repeat); \
     repeat?: never|daily|weekly|biweekly|monthly (closing the task makes the next one, due a step later; needs a due date); \
     repeat_until?: YYYY-MM-DD (no new one is made after this day); rfi?: RFI id and meeting?: meeting id (where the task came from)"
);

/// The cross-project query, registered beside the shared commands.
pub const MINE: &[CommandSpec] = &[CommandSpec::query(
    "task.mine",
    "List my tasks",
    Tool::Tasks,
    Access::Member,
    "include_closed?: bool (default false); all?: bool (every task you can see, not only those assigned to you or managed by you)",
    mine,
)];

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
            parent: None,
            repeat: Repeat::Never,
            repeat_until: None,
            next: None,
            rfi: None,
            meeting: None,
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

    fn apply_extra(&mut self, p: &Params<'_>) -> Result<()> {
        if p.present("parent") {
            self.parent = p.opt_id::<Task>("parent")?;
        }
        if let Some(r) = p.choice::<Repeat>("repeat")? {
            self.repeat = r;
        }
        if p.present("repeat_until") {
            self.repeat_until = p.date("repeat_until")?;
        }
        if p.present("rfi") {
            self.rfi = p.opt_id::<Rfi>("rfi")?;
        }
        if p.present("meeting") {
            self.meeting = p.opt_id::<Meeting>("meeting")?;
        }
        Ok(())
    }

    fn validate(&self, ctx: &Ctx<'_>) -> Result<()> {
        let bad = |what: &str| EngineError::conflict(what.to_string());
        if self.repeat != Repeat::Never && self.due.is_none() {
            return Err(bad("a repeating task needs a due date"));
        }
        if self.repeat_until.is_some() && self.repeat == Repeat::Never {
            return Err(bad("repeat_until only means something for a repeating task"));
        }
        if self.repeat_until.zip(self.due).is_some_and(|(until, due)| until < due) {
            return Err(bad("repeat_until can't be before the due date"));
        }
        if let Some(parent) = self.parent {
            if parent == self.id {
                return Err(bad("a task can't be its own subtask"));
            }
            let Some(p) = in_project(&ctx.state.tasks, self.project, parent, "task").ok().filter(|t| t.deleted.is_none()) else {
                return Err(EngineError::not_found(format!("task {parent} not found in this project")));
            };
            if p.parent.is_some() {
                return Err(bad("subtasks go one level deep: that task is itself a subtask"));
            }
            if self.repeat != Repeat::Never {
                return Err(bad("a subtask can't repeat; repeat its parent instead"));
            }
            if ctx.state.tasks.iter().any(|t| t.parent == Some(self.id) && t.deleted.is_none()) {
                return Err(bad("a task with subtasks can't become a subtask"));
            }
            if ctx.state.tasks.iter().filter(|t| t.parent == Some(parent) && t.deleted.is_none() && t.id != self.id).count() >= MAX_SUBTASKS {
                return Err(bad("a task can have at most 100 subtasks"));
            }
        }
        if let Some(r) = self.rfi {
            in_project(&ctx.state.rfis, self.project, r, "RFI")?;
        }
        if let Some(m) = self.meeting {
            in_project(&ctx.state.meetings, self.project, m, "meeting")?;
        }
        Ok(())
    }

    fn before_close(&self, ctx: &Ctx<'_>) -> Result<()> {
        let open = ctx.state.tasks.iter().filter(|t| t.parent == Some(self.id) && t.deleted.is_none() && t.status != WorkStatus::Closed).count();
        if open > 0 {
            return Err(EngineError::conflict(format!(
                "{} has {open} open subtask{}; close {} first",
                self.label(),
                if open == 1 { "" } else { "s" },
                if open == 1 { "it" } else { "them" }
            )));
        }
        Ok(())
    }

    /// A repeating task makes the next one: the same work, due a step later (the first due date that is
    /// still ahead, so a task closed late doesn't start out overdue), unless it is past `repeat_until`.
    fn after_close(&mut self, ctx: &mut Ctx<'_>) -> Result<()> {
        // A task made from a meeting action finishes the action, in the meeting it came from and in any later
        // one the action was carried forward to.
        if self.meeting.is_some() {
            let me = self.id;
            let holders: Vec<_> =
                ctx.state.meetings.iter().filter(|m| m.actions.iter().any(|a| a.task == Some(me) && !a.done)).map(|m| m.id).collect();
            for mid in holders {
                ctx.state.meetings.update(mid, |m| {
                    for a in m.actions.iter_mut().filter(|a| a.task == Some(me)) {
                        a.done = true;
                    }
                });
            }
        }
        let (Some(due), true, None) = (self.due, self.repeat != Repeat::Never, self.next) else { return Ok(()) };
        let today = ctx.today();
        let mut next = self.repeat.step(due);
        for _ in 0..MAX_CATCH_UP {
            match next {
                Some(d) if d < today => next = self.repeat.step(d),
                _ => break,
            }
        }
        let Some(next_due) = next.filter(|d| self.repeat_until.is_none_or(|u| *d <= u)) else { return Ok(()) };
        let mut again = Task::blank(ctx, self.project);
        again.title = self.title.clone();
        again.description = self.description.clone();
        again.location = self.location.clone();
        again.drawing_number = self.drawing_number.clone();
        again.kind = self.kind.clone();
        again.priority = self.priority;
        again.manager = self.manager;
        again.assignees = self.assignees.clone();
        again.due = Some(next_due);
        again.repeat = self.repeat;
        again.repeat_until = self.repeat_until;
        again.rfi = self.rfi;
        again.meeting = self.meeting;
        again.recompute_ball_in_court();
        let assignees = again.assignees.clone();
        let id = crate::workitem::insert(ctx, again);
        self.next = Some(id);
        crate::cmd::notification::notify(
            ctx,
            &assignees,
            self.project,
            &format!("The next {} is due {}: {}", self.label().to_lowercase(), next_due.pretty(), self.title),
            &format!("/projects/{}/tasks/{id}", self.project),
        );
        Ok(())
    }
}

const MAX_SUBTASKS: usize = 100;
/// How many steps a repeating task is moved on to reach a due date that hasn't passed.
const MAX_CATCH_UP: usize = 1_000;

/// The tasks a person has across projects: those assigned to or managed by them (with `all`, every task they
/// can see), open ones unless `include_closed`, in the projects where they can read tasks. Soonest due first,
/// undated last.
pub fn tasks_for(state: &CompanyState, actor: crate::Actor, include_closed: bool, all: bool) -> Vec<&Task> {
    let me = actor.person();
    let mut found: Vec<&Task> = state
        .tasks
        .iter()
        .filter(|t| t.deleted.is_none() && (include_closed || t.status != WorkStatus::Closed))
        .filter(|t| all || me.is_some_and(|me| t.assignees.contains(&me) || t.manager == Some(me)))
        .filter(|t| view::level(state, actor, t.project, Tool::Tasks) >= Level::Read)
        .collect();
    found.sort_by_key(|t| (t.due.is_none(), t.due, t.project, t.number));
    found
}

/// A person's own tasks across projects (see [`tasks_for`]).
fn mine(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    if v.actor.person().is_none() {
        return Err(EngineError::forbidden("task.mine lists a person's tasks; run it as a person"));
    }
    let (closed, all) = (p.flag_or("include_closed", false)?, p.flag_or("all", false)?);
    let today: Date = v.today();
    let rows: Vec<Value> = tasks_for(v.state, v.actor, closed, all)
        .into_iter()
        .take(1_000)
        .map(|t| {
            json!({
                "id": t.id, "project": t.project, "project_name": v.state.projects.get(t.project).map(|x| x.name.clone()).unwrap_or_default(),
                "number": t.number, "title": t.title, "status": t.status, "priority": t.priority, "due": t.due,
                "overdue": t.is_overdue(today), "parent": t.parent, "repeat": t.repeat, "assignees": t.assignees, "manager": t.manager,
            })
        })
        .collect();
    view::json(&rows)
}
