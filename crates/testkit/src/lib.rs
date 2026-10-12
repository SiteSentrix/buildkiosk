//! Test helpers (a dev-dependency only: `cargo xtask layers` refuses it as a normal dependency).
//!
//! [`World`] is a company with an admin, a fake clock that moves a minute per command, and an
//! in-memory journal, so a test reads like the workflow it checks:
//!
//! ```
//! use buildkiosk_testkit::World;
//! use serde_json::json;
//!
//! let mut w = World::new();
//! let project = w.project("Riverside Clinic");
//! let sub = w.member(project, "Sam", "Sub", "Subcontractor");
//! let rfi = w.ok(sub, "rfi.create", json!({"project": project, "subject": "Footing depth"}));
//! assert_eq!(rfi["number"], 1);
//! ```
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::{Id, Timestamp};
use buildkiosk_engine::cmd::company::template_named;
use buildkiosk_engine::{Actor, Engine, EngineError, JournalSink, MemoryJournal};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use serde_json::{Value, json};

/// 2026-10-12T09:00:00Z, a Monday.
pub const START: i64 = 1_791_795_600_000;

/// A company under test.
pub struct World {
    pub engine: Engine,
    pub journal: MemoryJournal,
    /// Milliseconds since the epoch; advances one minute per command.
    pub clock: i64,
    pub admin: Id<Person>,
}

impl Default for World {
    fn default() -> Self {
        World::new()
    }
}

impl World {
    /// A company "Summit Ridge Builders" with admin Ada Admin (ada@example.com).
    pub fn new() -> World {
        let mut w = World { engine: Engine::new(), journal: MemoryJournal::default(), clock: START, admin: Id::new(1) };
        let out = w.ok(
            Actor::System,
            "company.setup",
            json!({"name": "Summit Ridge Builders", "admin_first_name": "Ada", "admin_last_name": "Admin", "admin_email": "ada@example.com"}),
        );
        w.admin = Id::new(out["admin"].as_u64().unwrap());
        w
    }

    pub fn admin(&self) -> Actor {
        Actor::Person(self.admin)
    }

    pub fn now(&self) -> Timestamp {
        Timestamp::from_millis(self.clock)
    }

    /// Moves the clock forward.
    pub fn advance_days(&mut self, days: i64) {
        self.clock += days * 86_400_000;
    }

    /// Runs a command at the next minute.
    pub fn run(&mut self, actor: impl Into<Actor>, cmd: &str, params: Value) -> Result<Value, EngineError> {
        self.clock += 60_000;
        let at = self.now();
        self.engine.execute(&mut self.journal, actor.into(), at, cmd, &params)
    }

    /// Runs a command that must succeed.
    #[track_caller]
    pub fn ok(&mut self, actor: impl Into<Actor>, cmd: &str, params: Value) -> Value {
        match self.run(actor, cmd, params.clone()) {
            Ok(v) => v,
            Err(e) => panic!("{cmd} {params} failed: {e}"),
        }
    }

    /// Runs a command that must fail, and returns the error.
    #[track_caller]
    pub fn err(&mut self, actor: impl Into<Actor>, cmd: &str, params: Value) -> EngineError {
        match self.run(actor, cmd, params.clone()) {
            Ok(v) => panic!("{cmd} {params} should have failed but returned {v}"),
            Err(e) => e,
        }
    }

    /// A person who can sign in (no company permissions).
    pub fn user(&mut self, first: &str, last: &str) -> Id<Person> {
        let email = format!("{}.{}@example.com", first.to_lowercase(), last.to_lowercase());
        let out = self.ok(self.admin(), "person.create", json!({"first_name": first, "last_name": last, "email": email, "is_user": true}));
        Id::new(out["id"].as_u64().unwrap())
    }

    /// A project created by the admin.
    pub fn project(&mut self, name: &str) -> Id<Project> {
        let out = self.ok(self.admin(), "project.create", json!({"name": name}));
        Id::new(out["id"].as_u64().unwrap())
    }

    /// A new user added to `project` with the named permission template.
    pub fn member(&mut self, project: Id<Project>, first: &str, last: &str, template: &str) -> Id<Person> {
        let person = self.user(first, last);
        let t = template_named(self.engine.state(), template).unwrap_or_else(|| panic!("no template {template}"));
        self.ok(self.admin(), "member.add", json!({"project": project, "person": person, "template": t}));
        person
    }

    /// Asserts that replaying the journal from an empty engine rebuilds the current state.
    #[track_caller]
    pub fn assert_replay(&self) {
        let mut fresh = Engine::new();
        for e in &self.journal.entries {
            fresh.apply(e).unwrap_or_else(|err| panic!("replaying entry {} ({}): {err}", e.seq, e.cmd));
        }
        assert_eq!(fresh.seq(), self.engine.seq());
        assert!(fresh.state() == self.engine.state(), "replayed state differs from the live state");
    }
}

/// A journal that fails every append, for checking that failed writes change nothing.
pub struct FailingJournal;

impl JournalSink for FailingJournal {
    fn append(&mut self, _entry: &buildkiosk_engine::JournalEntry) -> Result<(), String> {
        Err("disk full".into())
    }
}

/// `Actor::Person` from an id.
pub fn person(id: Id<Person>) -> Actor {
    Actor::Person(id)
}
