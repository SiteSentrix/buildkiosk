//! Permission templates, project membership and how a person's level in a tool is worked out.

use std::collections::BTreeMap;

use buildkiosk_core::Id;
use serde::{Deserialize, Serialize};

use crate::common::Stamp;
use crate::directory::Person;
use crate::project::Project;
use crate::record;
use crate::state::CompanyState;
use crate::tool::{Level, Scope, Tool};

/// A named set of levels per tool ("Superintendent", "Subcontractor"…), given to people on the
/// company (for company tools) and on each project they join (for project tools).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionTemplate {
    pub id: Id<PermissionTemplate>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub levels: BTreeMap<Tool, Level>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(PermissionTemplate, "permission_template", trash);

impl PermissionTemplate {
    pub fn level(&self, tool: Tool) -> Level {
        self.levels.get(&tool).copied().unwrap_or_default()
    }
}

/// A person on a project's team, with their template and any per-tool overrides.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectMember {
    pub id: Id<ProjectMember>,
    pub project: Id<Project>,
    pub person: Id<Person>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template: Option<Id<PermissionTemplate>>,
    /// Levels that replace the template's for single tools.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub overrides: BTreeMap<Tool, Level>,
    /// Admin on every tool of this project, whatever the template says.
    #[serde(default)]
    pub project_admin: bool,
    /// Their role on this project ("Project manager", "Superintendent").
    #[serde(default)]
    pub role: String,
    pub added: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed: Option<Stamp>,
}
record!(ProjectMember, "project_member", project);

/// The current (not removed) membership of `person` on `project`.
pub fn membership(state: &CompanyState, project: Id<Project>, person: Id<Person>) -> Option<&ProjectMember> {
    state.members.iter().find(|m| m.project == project && m.person == person && m.removed.is_none())
}

/// A person's level in a company tool. Disabled people have none; company admins have Admin.
pub fn company_level(state: &CompanyState, person: Id<Person>, tool: Tool) -> Level {
    let Some(p) = state.people.get(person).filter(|p| p.is_enabled() && p.is_user) else {
        return Level::None;
    };
    if p.company_admin {
        return Level::Admin;
    }
    if tool.scope() != Scope::Company {
        return Level::None;
    }
    p.company_template.and_then(|t| state.templates.get(t)).filter(|t| t.deleted.is_none()).map(|t| t.level(tool)).unwrap_or_default()
}

/// A person's level in a project tool: Admin for company admins and project admins; otherwise the
/// member's override for the tool, else their template's level (at least Read on the project home);
/// nothing for people who aren't on the project.
pub fn project_level(state: &CompanyState, person: Id<Person>, project: Id<Project>, tool: Tool) -> Level {
    let Some(p) = state.people.get(person).filter(|p| p.is_enabled() && p.is_user) else {
        return Level::None;
    };
    if p.company_admin {
        return Level::Admin;
    }
    let Some(m) = membership(state, project, person) else {
        return Level::None;
    };
    if m.project_admin {
        return Level::Admin;
    }
    let level = match m.overrides.get(&tool) {
        Some(l) => *l,
        None => m.template.and_then(|t| state.templates.get(t)).filter(|t| t.deleted.is_none()).map(|t| t.level(tool)).unwrap_or_default(),
    };
    // Every member can at least open the project's home page.
    if tool == Tool::ProjectHome { level.max(Level::Read) } else { level }
}

/// Projects the person may open (any project tool above None).
pub fn visible_projects(state: &CompanyState, person: Id<Person>) -> Vec<Id<Project>> {
    state
        .projects
        .iter()
        .filter(|p| p.deleted.is_none())
        .filter(|p| project_level(state, person, p.id, Tool::ProjectHome) > Level::None)
        .map(|p| p.id)
        .collect()
}
