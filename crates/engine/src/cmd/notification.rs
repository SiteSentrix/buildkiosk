//! In-app notifications: what changed that needs a person's attention.
//!
//! Other commands call [`notify`] when work lands on someone; people read and clear their own
//! notifications here. Nobody sees another person's notifications.

use buildkiosk_core::Id;
use buildkiosk_model::directory::Person;
use buildkiosk_model::notification::Notification;
use buildkiosk_model::project::Project;
use buildkiosk_model::{Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::Read;
use crate::view::json;
use crate::{Actor, Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::query("notification.list", "List my notifications", Tool::CompanyAdmin, Access::Member, "unread?: bool", list),
    CommandSpec::mutate(
        "notification.read",
        "Mark notifications as read",
        Tool::CompanyAdmin,
        Access::Member,
        "id?: notification id (default: all of mine)",
        read,
    ),
];

/// Most notifications one action creates.
const MAX_RECIPIENTS: usize = 50;

/// Tells `people` something, except the person doing it, anyone who can't see the project and
/// anyone repeated. `link` is a path inside the app.
pub(crate) fn notify(ctx: &mut Ctx<'_>, people: &[Id<Person>], project: Id<Project>, text: &str, link: &str) {
    let me = ctx.person();
    let mut sent: Vec<Id<Person>> = Vec::new();
    for person in people {
        if Some(*person) == me || sent.contains(person) || sent.len() >= MAX_RECIPIENTS {
            continue;
        }
        let enabled = ctx.state.people.get(*person).is_some_and(|p| p.is_enabled() && p.is_user);
        if !enabled || perm::project_level(ctx.state, *person, project, Tool::ProjectHome) == buildkiosk_model::Level::None {
            continue;
        }
        sent.push(*person);
        let id = ctx.state.notifications.alloc();
        let stamp = ctx.stamp();
        ctx.state.notifications.put(Notification {
            id,
            person: *person,
            project: Some(project),
            text: text.to_string(),
            link: link.to_string(),
            created: stamp,
            read: false,
        });
    }
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let Actor::Person(me) = v.actor else { return json(&Vec::<&Notification>::new()) };
    let unread = p.flag_or("unread", false)?;
    let mut mine: Vec<&Notification> = v.state.notifications.iter().filter(|n| n.person == me && (!unread || !n.read)).collect();
    mine.reverse();
    mine.truncate(200);
    json(&mine)
}

fn read(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let Some(me) = ctx.person() else { return Err(EngineError::forbidden("notifications belong to a person")) };
    let one = p.opt_id::<Notification>("id")?;
    let ids: Vec<Id<Notification>> =
        ctx.state.notifications.iter().filter(|n| n.person == me && !n.read && one.is_none_or(|i| i == n.id)).map(|n| n.id).collect();
    if let Some(i) = one
        && !ctx.state.notifications.get(i).is_some_and(|n| n.person == me)
    {
        return Err(EngineError::not_found(format!("notification {i} not found")));
    }
    for id in &ids {
        ctx.state.notifications.update(*id, |n| n.read = true);
    }
    Ok(json!({ "marked": ids.len() }))
}
