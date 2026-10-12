//! The command registry. Ids are `tool.verb` (`rfi.create`, `member.add`).
//!
//! Adding a command: write it in the tool's module, add its [`CommandSpec`] to the module's
//! `COMMANDS`, and test it (`crates/engine/tests/`). The web UI, HTTP API, CLI and MCP pick it up
//! from here.

pub mod company;
pub mod directory;
pub mod meta;
pub mod notification;
pub mod perm;
pub mod project;
pub mod rfi;
// -- project management
pub mod spec;
pub mod submittal;

// -- quality and safety
pub mod inspection;
pub mod observation;
pub mod punch;

// -- field
pub mod daily_log;

// -- financials
pub mod budget;
pub mod change;
pub mod commitment;
pub mod direct_cost;
pub mod invoice;
pub mod prime;

// -- documents

// -- collaboration

use std::collections::HashMap;
use std::sync::OnceLock;

use buildkiosk_model::{Level, Tool};
use serde::Serialize;
use serde_json::Value;

use crate::{Ctx, Params, Result, View};

/// A command that changes the state.
pub type Mutate = fn(&mut Ctx<'_>, &Params<'_>) -> Result<Value>;
/// A command that only reads.
pub type Query = fn(&View<'_>, &Params<'_>) -> Result<Value>;

#[derive(Clone, Copy)]
pub enum Run {
    Mutate(Mutate),
    Query(Query),
}

/// The permission a command needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "scope", content = "level", rename_all = "snake_case")]
pub enum Access {
    /// Any signed-in person of the company.
    Member,
    /// Only the system actor (command line maintenance, imports, first-run setup).
    System,
    /// This level in the command's tool, company-wide.
    Company(Level),
    /// This level in the command's tool on the project named by the `project` parameter.
    Project(Level),
}

/// Metadata and implementation of one command.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub tool: Tool,
    pub access: Access,
    /// The parameters, for people and agents: `name: type` pairs, `?` marks optional ones.
    pub params: &'static str,
    pub run: Run,
}

impl CommandSpec {
    pub const fn mutate(id: &'static str, label: &'static str, tool: Tool, access: Access, params: &'static str, run: Mutate) -> CommandSpec {
        CommandSpec { id, label, tool, access, params, run: Run::Mutate(run) }
    }

    pub const fn query(id: &'static str, label: &'static str, tool: Tool, access: Access, params: &'static str, run: Query) -> CommandSpec {
        CommandSpec { id, label, tool, access, params, run: Run::Query(run) }
    }

    pub fn is_query(&self) -> bool {
        matches!(self.run, Run::Query(_))
    }

    pub fn info(&self) -> CommandInfo {
        CommandInfo { id: self.id, label: self.label, tool: self.tool, access: self.access, params: self.params, query: self.is_query() }
    }
}

/// Serializable command metadata (`command.list`, MCP `tools/list`, `buildkiosk commands`).
#[derive(Clone, Debug, Serialize)]
pub struct CommandInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub tool: Tool,
    pub access: Access,
    pub params: &'static str,
    pub query: bool,
}

/// Every module's commands.
const MODULES: &[&[CommandSpec]] = &[
    company::COMMANDS,
    directory::COMMANDS,
    perm::COMMANDS,
    project::COMMANDS,
    rfi::COMMANDS,
    meta::COMMANDS,
    notification::COMMANDS,
    // -- project management
    spec::COMMANDS,
    submittal::COMMANDS,
    // -- quality and safety
    inspection::COMMANDS,
    observation::COMMANDS,
    punch::COMMANDS,
    // -- field
    daily_log::COMMANDS,
    // -- financials
    budget::COMMANDS,
    change::COMMANDS,
    commitment::COMMANDS,
    direct_cost::COMMANDS,
    invoice::COMMANDS,
    prime::COMMANDS,
    // -- documents

    // -- collaboration
];

/// All registered commands, grouped by module.
pub fn all_commands() -> impl Iterator<Item = &'static CommandSpec> {
    MODULES.iter().flat_map(|m| m.iter())
}

/// The command with this id.
pub fn find_command(id: &str) -> Option<&'static CommandSpec> {
    static INDEX: OnceLock<HashMap<&'static str, &'static CommandSpec>> = OnceLock::new();
    INDEX.get_or_init(|| all_commands().map(|c| (c.id, c)).collect()).get(id).copied()
}

/// Whether a tool has any command yet (the UI only shows tools that do).
pub fn tool_available(tool: Tool) -> bool {
    all_commands().any(|c| c.tool == tool)
}
