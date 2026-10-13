//! Reports: summaries across a project's tools. Read-only; each tool's figures are only what the
//! caller may see in that tool.

use buildkiosk_core::Id;
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::Value;

use super::{Access, CommandSpec};
use crate::report;
use crate::view::json;
use crate::{Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::query(
        "report.summary",
        "Project summary",
        Tool::Reports,
        Access::Project(Level::Read),
        "project: project id (every tool you can read, with its total, open and overdue counts)",
        summary,
    ),
    CommandSpec::query(
        "report.waiting_on_me",
        "Waiting on me",
        Tool::Reports,
        Access::Project(Level::Read),
        "project: project id (RFIs, submittals, tasks, punch items, observations, incidents and meeting actions waiting on you, soonest due first)",
        waiting,
    ),
];

fn summary(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    json(&report::project_summary(v.state, v.actor, project, v.today()))
}

fn waiting(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let Some(me) = v.person() else { return json(&Vec::<report::Todo>::new()) };
    json(&report::waiting_on(v.state, v.actor, project, me))
}
