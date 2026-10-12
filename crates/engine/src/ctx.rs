//! What a command sees while it runs.

use buildkiosk_core::{Date, Id, Timestamp};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, InProject, Level, Stamp, Table, Tool, perm};

use crate::{Actor, EngineError};

/// Read access shared by mutating commands ([`Ctx`]) and queries ([`View`]).
pub trait Read {
    fn state(&self) -> &CompanyState;
    fn actor(&self) -> Actor;
    fn at(&self) -> Timestamp;
    fn cmd(&self) -> &'static str;

    fn person(&self) -> Option<Id<Person>> {
        self.actor().person()
    }

    /// Today in the company's time zone.
    fn today(&self) -> Date {
        self.at().date_at(self.state().utc_offset())
    }

    /// The actor's level in a project tool (Admin for the system).
    fn level(&self, project: Id<Project>, tool: Tool) -> Level {
        match self.actor() {
            Actor::System => Level::Admin,
            Actor::Person(p) => perm::project_level(self.state(), p, project, tool),
        }
    }

    /// The actor's level in a company tool (Admin for the system).
    fn company_level(&self, tool: Tool) -> Level {
        match self.actor() {
            Actor::System => Level::Admin,
            Actor::Person(p) => perm::company_level(self.state(), p, tool),
        }
    }

    fn bad(&self, msg: impl Into<String>) -> EngineError {
        EngineError::bad(self.cmd(), msg)
    }

    /// A person who exists and isn't deleted (for assignees, managers…).
    fn check_person(&self, id: Id<Person>) -> Result<Id<Person>, EngineError> {
        match self.state().people.get(id) {
            Some(p) if p.deleted.is_none() => Ok(id),
            _ => Err(self.bad(format!("person {id} not found"))),
        }
    }

    fn check_people(&self, ids: &[Id<Person>]) -> Result<(), EngineError> {
        ids.iter().try_for_each(|id| self.check_person(*id).map(|_| ()))
    }

    fn check_vendor(&self, id: Id<buildkiosk_model::directory::Vendor>) -> Result<(), EngineError> {
        match self.state().vendors.get(id) {
            Some(v) if v.deleted.is_none() => Ok(()),
            _ => Err(self.bad(format!("company {id} not found in the directory"))),
        }
    }
}

/// A row of `table` that belongs to `project`; NotFound otherwise (also for ids of other projects,
/// so that one project's access can't reach another's items).
pub fn in_project<'t, T: InProject>(table: &'t Table<T>, project: Id<Project>, id: Id<T>, what: &str) -> Result<&'t T, EngineError> {
    table.get(id).filter(|r| r.project() == project).ok_or_else(|| EngineError::not_found(format!("{what} {id} not found in this project")))
}

/// A mutating command's context: the copy of the state it edits.
pub struct Ctx<'a> {
    pub state: &'a mut CompanyState,
    pub actor: Actor,
    pub at: Timestamp,
    pub cmd: &'static str,
}

impl Ctx<'_> {
    /// Now, by the actor: for `created`, `updated` and similar fields.
    pub fn stamp(&self) -> Stamp {
        Stamp { by: self.actor.person(), at: self.at }
    }
}

impl Read for Ctx<'_> {
    fn state(&self) -> &CompanyState {
        self.state
    }
    fn actor(&self) -> Actor {
        self.actor
    }
    fn at(&self) -> Timestamp {
        self.at
    }
    fn cmd(&self) -> &'static str {
        self.cmd
    }
}

/// A query's context: read-only.
pub struct View<'a> {
    pub state: &'a CompanyState,
    pub actor: Actor,
    pub at: Timestamp,
    pub cmd: &'static str,
}

impl Read for View<'_> {
    fn state(&self) -> &CompanyState {
        self.state
    }
    fn actor(&self) -> Actor {
        self.actor
    }
    fn at(&self) -> Timestamp {
        self.at
    }
    fn cmd(&self) -> &'static str {
        self.cmd
    }
}
