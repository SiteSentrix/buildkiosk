//! Permission templates.

use std::collections::BTreeMap;

use buildkiosk_core::{Id, text};
use buildkiosk_model::perm::PermissionTemplate;
use buildkiosk_model::{Choice, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::view::json;
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "template.create",
        "Add permission template",
        Tool::CompanyAdmin,
        Access::Company(Level::Admin),
        "name: text; description?: text; levels?: {tool key: none|read|standard|admin}",
        create,
    ),
    CommandSpec::mutate(
        "template.update",
        "Edit permission template",
        Tool::CompanyAdmin,
        Access::Company(Level::Admin),
        "id: template id; name?: text; description?: text; levels?: {tool key: level} (only the tools given change)",
        update,
    ),
    CommandSpec::mutate(
        "template.delete",
        "Delete permission template",
        Tool::CompanyAdmin,
        Access::Company(Level::Admin),
        "id: template id",
        delete,
    ),
    CommandSpec::query("template.list", "List permission templates", Tool::CompanyAdmin, Access::Member, "(none)", list),
];

/// Reads a `{tool: level}` object into `levels`, replacing the levels of the tools it names
/// (`none` removes a tool).
fn read_levels(p: &Params<'_>, levels: &mut BTreeMap<Tool, Level>) -> Result<()> {
    let Some(map) = p.object("levels") else { return Ok(()) };
    if map.len() > Tool::ALL.len() * 2 {
        return Err(p.bad("levels", "has too many entries"));
    }
    for (k, v) in map {
        let tool = Tool::parse(k).ok_or_else(|| p.bad("levels", format!("`{k}` is not a tool")))?;
        let level = v.as_str().and_then(Level::parse).ok_or_else(|| p.bad("levels", format!("`{v}` is not a level for {k}")))?;
        if level == Level::None {
            levels.remove(&tool);
        } else {
            levels.insert(tool, level);
        }
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let name = p.req_line("name", text::SHORT)?;
    if ctx.state.templates.iter().any(|t| t.deleted.is_none() && t.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", format!("a template called {name} exists")));
    }
    let mut levels = BTreeMap::new();
    read_levels(p, &mut levels)?;
    let stamp = ctx.stamp();
    let id = ctx.state.templates.alloc();
    ctx.state.templates.put(PermissionTemplate {
        id,
        name,
        description: p.text_or_empty("description", 2000)?,
        levels,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<PermissionTemplate> = p.id("id")?;
    let before = ctx
        .state
        .templates
        .get(id)
        .filter(|t| t.deleted.is_none())
        .cloned()
        .ok_or_else(|| EngineError::not_found(format!("template {id} not found")))?;
    let mut t = before.clone();
    if p.present("name") {
        let name = p.req_line("name", text::SHORT)?;
        if ctx.state.templates.iter().any(|o| o.id != id && o.deleted.is_none() && o.name.eq_ignore_ascii_case(&name)) {
            return Err(p.bad("name", format!("a template called {name} exists")));
        }
        t.name = name;
    }
    if p.present("description") {
        t.description = p.text_or_empty("description", 2000)?;
    }
    read_levels(p, &mut t.levels)?;
    if t != before {
        t.updated = ctx.stamp();
        ctx.state.templates.put(t);
    }
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<PermissionTemplate> = p.id("id")?;
    let in_use = ctx.state.members.iter().filter(|m| m.removed.is_none() && m.template == Some(id)).count()
        + ctx.state.people.iter().filter(|x| x.deleted.is_none() && x.company_template == Some(id)).count();
    if in_use > 0 {
        return Err(EngineError::conflict(format!("the template is in use by {in_use} people; give them another template first")));
    }
    let stamp = ctx.stamp();
    ctx.state.templates.update(id, |t| t.deleted = Some(stamp)).ok_or_else(|| EngineError::not_found(format!("template {id} not found")))?;
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    let list: Vec<&PermissionTemplate> = v.state.templates.iter().filter(|t| t.deleted.is_none()).collect();
    json(&list)
}
