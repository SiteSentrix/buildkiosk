//! Read helpers shared by queries and the UI.
//!
//! The UI reads the state directly (it's thin: it shows engine state and acts through commands);
//! these helpers keep the rules for what a person may see in one place, so a page and the query
//! behind the API answer the same.

use buildkiosk_core::Id;
use buildkiosk_model::directory::{Person, Vendor};
use buildkiosk_model::project::Project;
use buildkiosk_model::rfi::{Rfi, RfiStatus};
use buildkiosk_model::submittal::{Submittal, SubmittalStatus};
use buildkiosk_model::{CompanyState, Level, Tool, perm};
use serde::Serialize;
use serde_json::Value;

use crate::{Actor, EngineError};

/// Serializes a query result.
pub fn json<T: Serialize>(value: &T) -> Result<Value, EngineError> {
    serde_json::to_value(value).map_err(|e| EngineError::Internal { cmd: "view".into(), msg: e.to_string() })
}

pub fn person_name(state: &CompanyState, id: Id<Person>) -> String {
    state.people.get(id).map(Person::name).unwrap_or_else(|| format!("Unknown person {id}"))
}

pub fn people_names(state: &CompanyState, ids: &[Id<Person>]) -> String {
    ids.iter().map(|id| person_name(state, *id)).collect::<Vec<_>>().join(", ")
}

pub fn vendor_name(state: &CompanyState, id: Id<Vendor>) -> String {
    state.vendors.get(id).map(|v| v.name.clone()).unwrap_or_else(|| format!("Unknown company {id}"))
}

/// The company a person works for ("our" company when `vendor` is empty).
pub fn employer_name(state: &CompanyState, person: &Person) -> String {
    match person.vendor {
        Some(v) => vendor_name(state, v),
        None => state.company().map(|c| c.name.clone()).unwrap_or_default(),
    }
}

/// The actor's level in a project tool.
pub fn level(state: &CompanyState, actor: Actor, project: Id<Project>, tool: Tool) -> Level {
    match actor {
        Actor::System => Level::Admin,
        Actor::Person(p) => perm::project_level(state, p, project, tool),
    }
}

/// Projects the actor may open, newest first.
pub fn projects_for(state: &CompanyState, actor: Actor) -> Vec<&Project> {
    let mut list: Vec<&Project> = match actor {
        Actor::System => state.projects.iter().filter(|p| p.deleted.is_none()).collect(),
        Actor::Person(person) => perm::visible_projects(state, person).into_iter().filter_map(|id| state.projects.get(id)).collect(),
    };
    list.reverse();
    list
}

/// Whether the actor may see an RFI: Read access, and for drafts and private RFIs, being part of it
/// (or an RFI admin).
pub fn can_see_rfi(state: &CompanyState, actor: Actor, rfi: &Rfi) -> bool {
    let lvl = level(state, actor, rfi.project, Tool::Rfis);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || rfi.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match rfi.status {
        RfiStatus::Draft => rfi.created.by == Some(person) || rfi.manager == Some(person),
        _ => !rfi.private || rfi.involves(person),
    }
}

/// The RFIs of a project the actor may see, in number order.
pub fn rfis_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Rfi> {
    let mut list: Vec<&Rfi> = state.rfis.iter().filter(|r| r.project == project && can_see_rfi(state, actor, r)).collect();
    list.sort_by_key(|r| r.number);
    list
}

/// Whether the actor may see a submittal: Read access, and for drafts and private submittals,
/// being part of it (or a submittals admin).
pub fn can_see_submittal(state: &CompanyState, actor: Actor, s: &Submittal) -> bool {
    let lvl = level(state, actor, s.project, Tool::Submittals);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || s.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match s.status {
        SubmittalStatus::Draft => s.created.by == Some(person) || s.manager == Some(person),
        _ => !s.private || s.involves(person),
    }
}

/// The submittals of a project the actor may see, in number order.
pub fn submittals_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Submittal> {
    let mut list: Vec<&Submittal> = state.submittals.iter().filter(|s| s.project == project && can_see_submittal(state, actor, s)).collect();
    list.sort_by_key(|s| s.number);
    list
}
