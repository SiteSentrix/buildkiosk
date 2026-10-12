//! Commands about the engine itself and the signed-in person.

use buildkiosk_model::{Level, Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec, all_commands};
use crate::view::{self, json};
use crate::{Actor, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::query("command.list", "List commands", Tool::Portfolio, Access::Member, "tool?: tool key", command_list),
    CommandSpec::query("me.get", "Who am I", Tool::Portfolio, Access::Member, "(none)", me),
];

fn command_list(_v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let tool = p.choice::<Tool>("tool")?;
    let list: Vec<_> = all_commands().filter(|c| tool.is_none_or(|t| c.tool == t)).map(|c| c.info()).collect();
    json(&list)
}

fn me(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    let projects: Vec<Value> = view::projects_for(v.state, v.actor)
        .into_iter()
        .map(|p| {
            let levels: serde_json::Map<String, Value> = Tool::project_tools()
                .filter_map(|t| {
                    let l = view::level(v.state, v.actor, p.id, t);
                    (l > Level::None).then(|| (buildkiosk_model::Choice::key(t).to_string(), json!(l)))
                })
                .collect();
            json!({ "id": p.id, "name": p.display_name(), "levels": levels })
        })
        .collect();
    let person = match v.actor {
        Actor::System => Value::Null,
        Actor::Person(id) => json(&v.state.people.get(id))?,
    };
    let company_levels: serde_json::Map<String, Value> = match v.actor {
        Actor::System => serde_json::Map::new(),
        Actor::Person(id) => Tool::company_tools()
            .filter_map(|t| {
                let l = perm::company_level(v.state, id, t);
                (l > Level::None).then(|| (buildkiosk_model::Choice::key(t).to_string(), json!(l)))
            })
            .collect(),
    };
    Ok(json!({ "person": person, "company": v.state.company().map(|c| c.name.clone()), "company_levels": company_levels, "projects": projects }))
}
