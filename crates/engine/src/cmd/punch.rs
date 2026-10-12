//! Punch list: small items of work to fix or finish before closeout.
//!
//! Workflow: someone with Standard access adds an item and assigns the people who must do the work →
//! an assignee marks the work done (ready for review) → the item's manager accepts and closes it,
//! or sends it back with a comment. Closed items can be reopened.
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - edit: the author or manager while the item isn't closed, admins always;
//! - mark work done: assignees, the manager, admins;
//! - send back, close, reopen: the manager or a punch list admin;
//! - comment, attach files: anyone with Standard access who can see the item (not once closed);
//! - delete: admins, or the author of a still-open item; restore: admins.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::punch::{PunchComment, PunchItem, PunchStatus};
use buildkiosk_model::{Level, Priority, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::submittal::file_ref;
use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "punch.create",
        "Add punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project: project id; title: text; description?, location?, kind?: text (kind: the trade or type of work); priority?: low|medium|high|urgent; \
         manager?: person id (default: you; accepts the work); assignees?: [person ids]; due?: YYYY-MM-DD",
        create,
    ),
    CommandSpec::mutate(
        "punch.update",
        "Edit punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; then any punch.create field",
        update,
    ),
    CommandSpec::mutate(
        "punch.complete",
        "Mark punch work done",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; comment?: text",
        complete,
    ),
    CommandSpec::mutate(
        "punch.send_back",
        "Send punch item back",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; comment: text (what still needs doing)",
        send_back,
    ),
    CommandSpec::mutate(
        "punch.close",
        "Close punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; comment?: text",
        close,
    ),
    CommandSpec::mutate(
        "punch.reopen",
        "Reopen punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; comment?: text",
        reopen,
    ),
    CommandSpec::mutate(
        "punch.comment",
        "Comment on punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; body: text",
        comment,
    ),
    CommandSpec::mutate(
        "punch.attach",
        "Attach file to punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text (of an uploaded file); name: text; size: integer (bytes); mime?: text",
        attach,
    ),
    CommandSpec::mutate(
        "punch.detach",
        "Remove attachment from punch item",
        Tool::PunchList,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text",
        detach,
    ),
    CommandSpec::mutate("punch.delete", "Delete punch item", Tool::PunchList, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("punch.restore", "Restore punch item", Tool::PunchList, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "punch.list",
        "List punch items",
        Tool::PunchList,
        Access::Project(Level::Read),
        "project: project id; status?: open|ready_for_review|closed; q?: text; mine?: bool (ball in court is me); overdue?: bool; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("punch.get", "Show punch item", Tool::PunchList, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_COMMENTS: usize = 500;
const MAX_ATTACHMENTS: usize = 50;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<PunchItem> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<PunchItem> = p.id("id")?;
    let item = in_project(&ctx.state.punch_items, project, id, "punch item")?;
    if !view::can_see_punch(ctx.state, ctx.actor, item) {
        return Err(EngineError::not_found(format!("punch item {id} not found in this project")));
    }
    Ok(item.clone())
}

fn is_admin(ctx: &Ctx<'_>, item: &PunchItem) -> bool {
    ctx.level(item.project, Tool::PunchList) == Level::Admin
}

fn is_manager(ctx: &Ctx<'_>, item: &PunchItem) -> bool {
    is_admin(ctx, item) || (ctx.person().is_some() && item.manager == ctx.person())
}

fn is_assignee(ctx: &Ctx<'_>, item: &PunchItem) -> bool {
    ctx.person().is_some_and(|m| item.assignees.contains(&m))
}

/// Whether the actor may edit the item's fields now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, item: &PunchItem) -> bool {
    let lvl = view::level(state, actor, item.project, Tool::PunchList);
    if lvl == Level::Admin {
        return true;
    }
    let me = actor.person();
    lvl >= Level::Standard && me.is_some() && item.status != PunchStatus::Closed && (item.created.by == me || item.manager == me)
}

fn link(item: &PunchItem) -> String {
    format!("/projects/{}/punch-list/{}", item.project, item.id)
}

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, item: &mut PunchItem, creating: bool) -> Result<()> {
    if creating || p.present("title") {
        item.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        item.description = p.text_or_empty("description", text::LONG)?;
    }
    for (key, field) in [("location", &mut item.location), ("kind", &mut item.kind)] {
        if p.present(key) {
            *field = p.line_or_empty(key, text::SHORT)?;
        }
    }
    if let Some(pr) = p.choice::<Priority>("priority")? {
        item.priority = pr;
    }
    if p.present("manager") {
        let m = p.opt_id::<Person>("manager")?;
        if let Some(m) = m {
            ctx.check_person(m)?;
        }
        item.manager = m;
    }
    if p.present("assignees") {
        let mut a = p.ids("assignees")?;
        ctx.check_people(&a)?;
        let mut seen = Vec::new();
        a.retain(|x| {
            let fresh = !seen.contains(x);
            seen.push(*x);
            fresh
        });
        item.assignees = a;
    }
    if p.present("due") {
        item.due = p.date("due")?;
    }
    item.recompute_ball_in_court();
    Ok(())
}

fn add_comment(ctx: &Ctx<'_>, item: &mut PunchItem, body: String) -> Result<()> {
    if body.is_empty() {
        return Ok(());
    }
    if item.comments.len() >= MAX_COMMENTS {
        return Err(EngineError::conflict(format!("{} has too many comments", item.label())));
    }
    let id = item.comments.iter().map(|c| c.id).max().unwrap_or(0).saturating_add(1);
    item.comments.push(PunchComment { id, by: ctx.person(), at: ctx.at, body });
    Ok(())
}

/// The next number for a new item of the project.
fn next_number(ctx: &Ctx<'_>, project: Id<Project>) -> u32 {
    ctx.state.punch_items.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

/// A new open item with nothing filled in yet, managed by the actor.
pub(crate) fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> PunchItem {
    let stamp = ctx.stamp();
    PunchItem {
        id: ctx.state.punch_items.peek_next(),
        project,
        number: next_number(ctx, project),
        title: String::new(),
        description: String::new(),
        location: String::new(),
        kind: String::new(),
        priority: Priority::default(),
        status: PunchStatus::Open,
        manager: ctx.person(),
        assignees: Vec::new(),
        ball_in_court: Vec::new(),
        due: None,
        work_done: None,
        closed: None,
        attachments: Vec::new(),
        comments: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    }
}

/// Stores a finished new item and returns its id.
pub(crate) fn insert(ctx: &mut Ctx<'_>, mut item: PunchItem) -> Id<PunchItem> {
    let id = ctx.state.punch_items.alloc();
    item.id = id;
    ctx.state.punch_items.put(item);
    id
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut item = blank(ctx, project);
    let number = item.number;
    apply_fields(ctx, p, &mut item, true)?;
    let id = ctx.state.punch_items.alloc();
    item.id = id;
    notify(ctx, &item.assignees, project, &format!("{} was assigned to you: {}", item.label(), item.title), &link(&item));
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &item) {
        return Err(EngineError::forbidden(match item.status {
            PunchStatus::Closed => format!("{} is closed; reopen it to edit", item.label()),
            _ => format!("only the author, the manager or a punch list admin can edit {}", item.label()),
        }));
    }
    let before = item.clone();
    apply_fields(ctx, p, &mut item, false)?;
    let id = item.id;
    if item != before {
        let newly: Vec<Id<Person>> = item.assignees.iter().copied().filter(|a| !before.assignees.contains(a)).collect();
        notify(ctx, &newly, item.project, &format!("{} was assigned to you: {}", item.label(), item.title), &link(&item));
        item.updated = ctx.stamp();
        ctx.state.punch_items.put(item);
    }
    Ok(json!({ "id": id }))
}

fn complete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !is_assignee(ctx, &item) && !is_manager(ctx, &item) {
        return Err(EngineError::forbidden(format!("only the people assigned to {} or its manager can mark the work done", item.label())));
    }
    if item.status != PunchStatus::Open {
        return Err(EngineError::conflict(format!("{} is {}; only open items can be marked done", item.label(), item.status)));
    }
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = PunchStatus::ReadyForReview;
    item.work_done = Some(ctx.stamp());
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id;
    notify(
        ctx,
        &item.manager.into_iter().collect::<Vec<_>>(),
        item.project,
        &format!("{} is ready for your review: {}", item.label(), item.title),
        &link(&item),
    );
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn send_back(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or a punch list admin can send it back"));
    }
    if item.status != PunchStatus::ReadyForReview {
        return Err(EngineError::conflict(format!("{} is {}; only items ready for review can be sent back", item.label(), item.status)));
    }
    add_comment(ctx, &mut item, p.req_text("comment", text::LONG)?)?;
    item.status = PunchStatus::Open;
    item.work_done = None;
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id;
    notify(ctx, &item.assignees, item.project, &format!("{} was sent back: {}", item.label(), item.title), &link(&item));
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn close(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or a punch list admin can close it"));
    }
    if item.status == PunchStatus::Closed {
        return Err(EngineError::conflict(format!("{} is already closed", item.label())));
    }
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = PunchStatus::Closed;
    item.closed = Some(ctx.stamp());
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id;
    let audience: Vec<Id<Person>> = item.assignees.iter().copied().chain(item.created.by).collect();
    notify(ctx, &audience, item.project, &format!("{} was closed: {}", item.label(), item.title), &link(&item));
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or a punch list admin can reopen it"));
    }
    if item.status != PunchStatus::Closed {
        return Err(EngineError::conflict(format!("{} is {}; only closed items can be reopened", item.label(), item.status)));
    }
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = PunchStatus::Open;
    item.closed = None;
    item.work_done = None;
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id;
    notify(ctx, &item.assignees, item.project, &format!("{} was reopened: {}", item.label(), item.title), &link(&item));
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn comment(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if item.status == PunchStatus::Closed {
        return Err(EngineError::conflict(format!("{} is closed; reopen it to comment", item.label())));
    }
    add_comment(ctx, &mut item, p.req_text("body", text::LONG)?)?;
    item.updated = ctx.stamp();
    let id = item.id;
    let waiting = item.ball_in_court.clone();
    notify(ctx, &waiting, item.project, &format!("New comment on {}: {}", item.label(), item.title), &link(&item));
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

/// Who may add or remove files: whoever may edit, and assignees while the item isn't closed.
fn can_attach(ctx: &Ctx<'_>, item: &PunchItem) -> bool {
    can_edit(ctx.state, ctx.actor, item) || (item.status != PunchStatus::Closed && is_assignee(ctx, item))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !can_attach(ctx, &item) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", item.label())));
    }
    let file = file_ref(p)?;
    if item.attachments.iter().any(|a| a.sha256 == file.sha256) {
        return Ok(json!({ "id": item.id }));
    }
    if item.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", item.label())));
    }
    item.attachments.push(file);
    item.updated = ctx.stamp();
    let id = item.id;
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item = load(ctx, p)?;
    if !can_attach(ctx, &item) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", item.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = item.attachments.len();
    item.attachments.retain(|a| a.sha256 != sha256);
    if item.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this punch item"));
    }
    item.updated = ctx.stamp();
    let id = item.id;
    ctx.state.punch_items.put(item);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let item = load(ctx, p)?;
    let own_open = item.status == PunchStatus::Open && item.created.by.is_some() && item.created.by == ctx.person();
    if !is_admin(ctx, &item) && !own_open {
        return Err(EngineError::forbidden("only punch list admins can delete items (authors can delete their own open items)"));
    }
    let stamp = ctx.stamp();
    ctx.state.punch_items.update(item.id, |i| i.deleted = Some(stamp));
    Ok(json!({ "id": item.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<PunchItem> = p.id("id")?;
    let item = in_project(&ctx.state.punch_items, project, id, "punch item")?;
    if item.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", item.label())));
    }
    ctx.state.punch_items.update(id, |i| i.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<PunchStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    let mine = p.flag_or("mine", false)?;
    let overdue = p.flag_or("overdue", false)?;
    let today: Date = v.today();
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::PunchList) < Level::Admin {
            return Err(EngineError::forbidden("only punch list admins can see the recycle bin"));
        }
        let bin: Vec<&PunchItem> = v.state.punch_items.iter().filter(|i| i.project == project && i.deleted.is_some()).collect();
        return json(&bin);
    }
    let me = v.person();
    let list: Vec<&PunchItem> = view::punch_for(v.state, v.actor, project)
        .into_iter()
        .filter(|i| status.is_none_or(|s| i.status == s))
        .filter(|i| !mine || me.is_some_and(|m| i.ball_in_court.contains(&m)))
        .filter(|i| !overdue || i.is_overdue(today))
        .filter(|i| {
            q.as_ref().is_none_or(|q| i.title.to_lowercase().contains(q) || i.location.to_lowercase().contains(q) || i.number.to_string() == *q)
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<PunchItem> = p.id("id")?;
    let item = in_project(&v.state.punch_items, project, id, "punch item")?;
    if !view::can_see_punch(v.state, v.actor, item) {
        return Err(EngineError::not_found(format!("punch item {id} not found in this project")));
    }
    json(item)
}
