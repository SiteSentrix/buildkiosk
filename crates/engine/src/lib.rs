//! The BuildKiosk engine (layer 5).
//!
//! Every user-visible action is a command with a stable id (`rfi.create`, `project.update`,
//! `member.add`…), a permission it needs, and JSON parameters. The web UI, the HTTP API, the CLI and
//! the MCP server all go through [`Engine::execute`]; nothing else changes the company state.
//!
//! A command runs on a copy of the state. If it succeeds, the engine diffs the copy against the
//! current state, appends a [`JournalEntry`] (the command, who ran it, when, and every row it
//! changed) to the journal, and only then makes the copy current. If it fails — or panics — the
//! copy is dropped and nothing happened. Replaying the journal's row changes rebuilds the state
//! exactly, and the same entries are each item's change history and the audit log.
#![forbid(unsafe_code)]

pub mod access;
pub mod cmd;
pub mod costdoc;
pub mod ctx;
pub mod guard;
pub mod invoicedoc;
pub mod journal;
pub mod params;
pub mod view;
pub mod workitem;

use buildkiosk_core::{Id, Timestamp};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, ModelError};
use serde_json::Value;

pub use cmd::{Access, CommandSpec, Run, all_commands, find_command};
pub use ctx::{Ctx, Read, View};
pub use journal::{JournalEntry, JournalSink, MemoryJournal};
pub use params::Params;

pub use buildkiosk_core as core;
pub use buildkiosk_model as model;

/// Who runs a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    /// Maintenance, imports, first-run setup and seeding: not subject to permissions. Never used
    /// for requests from people.
    System,
    /// A signed-in person.
    Person(Id<Person>),
}

impl From<Id<Person>> for Actor {
    fn from(p: Id<Person>) -> Actor {
        Actor::Person(p)
    }
}

impl Actor {
    pub fn person(self) -> Option<Id<Person>> {
        match self {
            Actor::System => None,
            Actor::Person(p) => Some(p),
        }
    }
}

/// Why a command didn't run.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command `{0}`")]
    UnknownCommand(String),
    /// The person lacks the permission (HTTP 403).
    #[error("{0}")]
    Forbidden(String),
    #[error("invalid parameters for `{cmd}`: {msg}")]
    BadParams { cmd: String, msg: String },
    /// HTTP 404.
    #[error("{0}")]
    NotFound(String),
    /// The item's state doesn't allow it (closing a draft, editing a closed item…). HTTP 409.
    #[error("{0}")]
    Conflict(String),
    /// The journal couldn't be written; nothing changed. HTTP 500.
    #[error("the change could not be saved: {0}")]
    Storage(String),
    /// A bug: the command panicked or produced unstorable data. Nothing changed. HTTP 500.
    #[error("internal error in `{cmd}`: {msg} (nothing was changed; please report this bug)")]
    Internal { cmd: String, msg: String },
}

impl EngineError {
    /// The HTTP status that fits.
    pub fn http_status(&self) -> u16 {
        match self {
            EngineError::UnknownCommand(_) | EngineError::NotFound(_) => 404,
            EngineError::Forbidden(_) => 403,
            EngineError::BadParams { .. } => 400,
            EngineError::Conflict(_) => 409,
            EngineError::Storage(_) | EngineError::Internal { .. } => 500,
        }
    }

    /// A short machine-readable kind for API clients.
    pub fn kind(&self) -> &'static str {
        match self {
            EngineError::UnknownCommand(_) => "unknown_command",
            EngineError::Forbidden(_) => "forbidden",
            EngineError::BadParams { .. } => "bad_params",
            EngineError::NotFound(_) => "not_found",
            EngineError::Conflict(_) => "conflict",
            EngineError::Storage(_) => "storage",
            EngineError::Internal { .. } => "internal",
        }
    }

    pub fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
        EngineError::BadParams { cmd: cmd.to_string(), msg: msg.into() }
    }

    pub fn conflict(msg: impl Into<String>) -> EngineError {
        EngineError::Conflict(msg.into())
    }

    pub fn not_found(msg: impl Into<String>) -> EngineError {
        EngineError::NotFound(msg.into())
    }

    pub fn forbidden(msg: impl Into<String>) -> EngineError {
        EngineError::Forbidden(msg.into())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// One company's state and the sequence number of the last journal entry applied to it.
#[derive(Clone, Debug, Default)]
pub struct Engine {
    state: CompanyState,
    seq: u64,
}

impl Engine {
    pub fn new() -> Engine {
        Engine::default()
    }

    /// An engine over a state loaded from a snapshot taken after journal entry `seq`.
    pub fn from_state(state: CompanyState, seq: u64) -> Engine {
        Engine { state, seq }
    }

    pub fn state(&self) -> &CompanyState {
        &self.state
    }

    /// The sequence number of the last journal entry in the state (0 for a new company).
    pub fn seq(&self) -> u64 {
        self.seq
    }

    /// Runs command `id` as `actor` at time `at`. Queries return data and change nothing. Other
    /// commands are journaled through `journal` before they take effect; a command that changes
    /// nothing isn't journaled.
    pub fn execute(&mut self, journal: &mut dyn JournalSink, actor: Actor, at: Timestamp, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        let empty = Value::Object(serde_json::Map::new());
        let params = match params {
            Value::Null => &empty,
            Value::Object(_) => params,
            _ => return Err(EngineError::bad(spec.id, "parameters must be a JSON object")),
        };
        access::check(&self.state, actor, spec, params)?;
        let p = Params::new(params, spec.id);
        match spec.run {
            Run::Query(q) => {
                let view = View { state: &self.state, actor, at, cmd: spec.id };
                guard::catch_panic(|| q(&view, &p)).map_err(|msg| EngineError::Internal { cmd: spec.id.to_string(), msg })?
            }
            Run::Mutate(m) => {
                let mut next = self.state.clone();
                let value = {
                    let mut ctx = Ctx { state: &mut next, actor, at, cmd: spec.id };
                    guard::catch_panic(|| m(&mut ctx, &p)).map_err(|msg| EngineError::Internal { cmd: spec.id.to_string(), msg })??
                };
                let changes = self.state.diff(&next).map_err(|e| EngineError::Internal { cmd: spec.id.to_string(), msg: e.to_string() })?;
                if changes.is_empty() {
                    return Ok(value);
                }
                let entry = JournalEntry {
                    seq: self.seq.saturating_add(1),
                    at,
                    actor: actor.person(),
                    cmd: spec.id.to_string(),
                    params: params.clone(),
                    changes,
                };
                journal.append(&entry).map_err(EngineError::Storage)?;
                self.state = next;
                self.seq = entry.seq;
                Ok(value)
            }
        }
    }

    /// Replays one journal entry (loading a company). Entries must come in order; a bad entry
    /// leaves the state as it was.
    pub fn apply(&mut self, entry: &JournalEntry) -> std::result::Result<(), ModelError> {
        if entry.seq != self.seq.saturating_add(1) {
            return Err(ModelError::Decode { table: "journal".into(), msg: format!("entry {} follows entry {}", entry.seq, self.seq) });
        }
        let mut next = self.state.clone();
        for change in &entry.changes {
            next.apply(change)?;
        }
        self.state = next;
        self.seq = entry.seq;
        Ok(())
    }

    /// Whether `actor` may run `id` (on `project`, for project commands): for showing or hiding
    /// buttons. The command can still refuse for item-level reasons.
    pub fn allowed(&self, actor: Actor, id: &str, project: Option<Id<Project>>) -> bool {
        let Some(spec) = find_command(id) else { return false };
        let mut params = serde_json::Map::new();
        if let Some(p) = project {
            params.insert("project".into(), Value::from(p.raw()));
        }
        access::check(&self.state, actor, spec, &Value::Object(params)).is_ok()
    }
}
