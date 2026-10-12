//! Who may run which command.
//!
//! Every [`CommandSpec`] declares an [`Access`]: the tool it belongs to and the level it needs,
//! company-wide or on the project named by the `project` parameter. This module checks that before
//! any command runs. Item-level rules (a Standard user may edit only their own drafts, a private
//! RFI is hidden from people who aren't part of it) are checked by the commands themselves.

use buildkiosk_core::Id;
use buildkiosk_model::project::Project;
use buildkiosk_model::{Choice, CompanyState, Level, perm};
use serde_json::Value;

use crate::cmd::{Access, CommandSpec};
use crate::{Actor, EngineError, Params};

/// Checks that `actor` may run `spec` with `params`.
pub fn check(state: &CompanyState, actor: Actor, spec: &CommandSpec, params: &Value) -> Result<(), EngineError> {
    let person = match actor {
        Actor::System => return Ok(()),
        Actor::Person(p) => p,
    };
    let Some(who) = state.people.get(person) else {
        return Err(EngineError::forbidden("your account no longer exists"));
    };
    if !who.is_enabled() || !who.is_user {
        return Err(EngineError::forbidden("your account is disabled"));
    }
    match spec.access {
        Access::System => Err(EngineError::forbidden(format!("`{}` can only be run by an administrator from the command line", spec.id))),
        Access::Member => Ok(()),
        Access::Company(need) => {
            let have = perm::company_level(state, person, spec.tool);
            if have >= need { Ok(()) } else { Err(denied(spec, need, have)) }
        }
        Access::Project(need) => {
            let project = project_param(state, spec, params)?;
            let have = perm::project_level(state, person, project, spec.tool);
            if have >= need { Ok(()) } else { Err(denied(spec, need, have)) }
        }
    }
}

/// The `project` parameter of a project command: present, and an existing project that isn't in
/// the recycle bin.
pub fn project_param(state: &CompanyState, spec: &CommandSpec, params: &Value) -> Result<Id<Project>, EngineError> {
    let p = Params::new(params, spec.id);
    let id: Id<Project> = p.id("project")?;
    match state.projects.get(id) {
        Some(proj) if proj.deleted.is_none() => Ok(id),
        _ => Err(EngineError::not_found(format!("project {id} not found"))),
    }
}

fn denied(spec: &CommandSpec, need: Level, have: Level) -> EngineError {
    let tool = spec.tool.label();
    if have == Level::None {
        EngineError::forbidden(format!("you don't have access to {tool}"))
    } else {
        EngineError::forbidden(format!("{} needs {} access to {tool} (you have {})", spec.label, need.label(), have.label()))
    }
}
