//! The workflow punch items and observations share, written once.
//!
//! Someone with Standard access adds an item and assigns the people who must do the work → an
//! assignee marks the work done (ready for review) → the item's manager accepts and closes it, or
//! sends it back with a comment. Closed items can be reopened.
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - edit: the author or manager while the item isn't closed, admins always;
//! - mark work done: assignees, the manager, admins;
//! - send back, close, reopen: the manager or a tool admin;
//! - comment, attach files: anyone with Standard access who can see the item (not once closed);
//! - delete: admins, or the author of a still-open item; restore: admins.
//!
//! A tool opts in by implementing [`WorkItem`] for its record and registering its commands with
//! `work_commands!`.

use std::ops::DerefMut;

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::work::{WorkComment, WorkCore, WorkStatus};
use buildkiosk_model::{CompanyState, InProject, Level, Priority, Record, Table, Tool, Trash};
use serde::Serialize;
use serde_json::{Value, json};

use crate::cmd::notification::notify;
use crate::cmd::submittal::file_ref;
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Result, View};

/// A record that runs the shared workflow.
pub trait WorkItem: Record + InProject + Trash + Clone + PartialEq + Serialize + DerefMut<Target = WorkCore> + 'static {
    const TOOL: Tool;
    /// How messages name one: "punch item".
    const NOUN: &'static str;
    /// Its first letter capitalised, for labels: "Punch item".
    const LABEL: &'static str;
    /// Its path in the web app: "punch-list".
    const PATH: &'static str;
    /// The command that attaches an uploaded file: "punch.attach".
    const ATTACH_CMD: &'static str;
    /// Its table in the journal and snapshots (for the change history).
    const TABLE: &'static str;

    fn table(state: &CompanyState) -> &Table<Self>;
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self>;
    /// A new open item with nothing filled in yet, managed by the actor.
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self;
    fn set_id(&mut self, id: Id<Self>);
    /// Reads the fields only this kind of item has.
    fn apply_extra(&mut self, _p: &Params<'_>) -> Result<()> {
        Ok(())
    }
    /// Whether this person may see the item, beyond having access to the tool (tool admins always may).
    fn visible_to(&self, _state: &CompanyState, _actor: Actor) -> bool {
        true
    }
    /// Runs after the item is created (`before` is `None`) or changed, once it is saved.
    fn after_save(&self, _before: Option<&Self>, _ctx: &mut Ctx<'_>) -> Result<()> {
        Ok(())
    }
    /// Checks the finished item against the rest of the company (links to other records, and so on).
    fn validate(&self, _ctx: &Ctx<'_>) -> Result<()> {
        Ok(())
    }
    /// Whether the item may be closed now.
    fn before_close(&self, _ctx: &Ctx<'_>) -> Result<()> {
        Ok(())
    }
    /// Runs once the item is closed, before it is saved (it may add other records).
    fn after_close(&mut self, _ctx: &mut Ctx<'_>) -> Result<()> {
        Ok(())
    }
}

/// "Punch item #4".
pub fn label<T: WorkItem>(item: &T) -> String {
    format!("{} #{}", T::LABEL, item.number)
}

fn link<T: WorkItem>(item: &T) -> String {
    format!("/projects/{}/{}/{}", item.project(), T::PATH, item.id())
}

/// Whether the actor may see the item: Read access to the tool; admins also see deleted ones.
pub fn can_see<T: WorkItem>(state: &CompanyState, actor: Actor, item: &T) -> bool {
    let lvl = view::level(state, actor, item.project(), T::TOOL);
    lvl == Level::Admin || (item.deleted.is_none() && lvl >= Level::Read && item.visible_to(state, actor))
}

/// The items of a project the actor may see, in number order (deleted ones left out).
pub fn items_for<T: WorkItem>(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&T> {
    let mut list: Vec<&T> = T::table(state).iter().filter(|i| i.project() == project && i.deleted.is_none() && can_see(state, actor, *i)).collect();
    list.sort_by_key(|i| i.number);
    list
}

/// Whether the actor may edit the item's fields now.
pub fn can_edit<T: WorkItem>(state: &CompanyState, actor: Actor, item: &T) -> bool {
    let lvl = view::level(state, actor, item.project(), T::TOOL);
    if lvl == Level::Admin {
        return true;
    }
    let me = actor.person();
    lvl >= Level::Standard && me.is_some() && item.status != WorkStatus::Closed && (item.created.by == me || item.manager == me)
}

fn load<T: WorkItem>(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<T> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let item = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if !can_see(ctx.state, ctx.actor, item) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    Ok(item.clone())
}

fn is_admin<T: WorkItem>(ctx: &Ctx<'_>, item: &T) -> bool {
    ctx.level(item.project(), T::TOOL) == Level::Admin
}

fn is_manager<T: WorkItem>(ctx: &Ctx<'_>, item: &T) -> bool {
    is_admin(ctx, item) || (ctx.person().is_some() && item.manager == ctx.person())
}

fn is_assignee<T: WorkItem>(ctx: &Ctx<'_>, item: &T) -> bool {
    ctx.person().is_some_and(|m| item.assignees.contains(&m))
}

fn apply_fields<T: WorkItem>(ctx: &Ctx<'_>, p: &Params<'_>, item: &mut T, creating: bool) -> Result<()> {
    if creating || p.present("title") {
        item.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        item.description = p.text_or_empty("description", text::LONG)?;
    }
    for key in ["location", "kind", "drawing_number"] {
        if p.present(key) {
            let v = p.line_or_empty(key, text::SHORT)?;
            match key {
                "location" => item.location = v,
                "kind" => item.kind = v,
                _ => item.drawing_number = v,
            }
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
    item.apply_extra(p)?;
    item.validate(ctx)?;
    item.recompute_ball_in_court();
    Ok(())
}

fn add_comment<T: WorkItem>(ctx: &Ctx<'_>, item: &mut T, body: String) -> Result<()> {
    const MAX_COMMENTS: usize = 500;
    if body.is_empty() {
        return Ok(());
    }
    if item.comments.len() >= MAX_COMMENTS {
        return Err(EngineError::conflict(format!("{} has too many comments", label(item))));
    }
    let id = item.comments.iter().map(|c| c.id).max().unwrap_or(0).saturating_add(1);
    item.comments.push(WorkComment { id, by: ctx.person(), at: ctx.at, body });
    Ok(())
}

pub fn create<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut item = T::blank(ctx, project);
    let number = item.number;
    apply_fields(ctx, p, &mut item, true)?;
    let id = insert(ctx, item.clone());
    item.set_id(id);
    notify(ctx, &item.assignees, project, &format!("{} was assigned to you: {}", label(&item), item.title), &link(&item));
    item.after_save(None, ctx)?;
    Ok(json!({ "id": id, "number": number }))
}

/// Adds many items from CSV lines: title, location, kind, priority, due (YYYY-MM-DD), assignees
/// (email addresses separated by `;`). An optional header line starting with "title" is skipped.
/// Nothing is added unless every line is good.
pub fn import<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    const MAX_ROWS: usize = 500;
    let project: Id<Project> = p.id("project")?;
    let body = p.req_text("text", 500_000)?;
    let mut rows: Vec<Value> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells = crate::cmd::submittal::csv_cells(line);
        let get = |i: usize| cells.get(i).map(String::as_str).unwrap_or("");
        if rows.is_empty() && get(0).eq_ignore_ascii_case("title") {
            continue;
        }
        if rows.len() >= MAX_ROWS {
            return Err(p.bad("text", format!("has more than {MAX_ROWS} rows")));
        }
        let mut people = Vec::new();
        for email in get(5).split(';').map(str::trim).filter(|e| !e.is_empty()) {
            let found = ctx.state.people.iter().find(|x| x.is_enabled() && x.email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(email)));
            let found = found.ok_or_else(|| p.bad("text", format!("line {}: nobody in the directory has the email {email}", n + 1)))?;
            people.push(json!(found.id));
        }
        let mut row = serde_json::Map::new();
        row.insert("title".into(), json!(get(0)));
        row.insert("location".into(), json!(get(1)));
        row.insert("kind".into(), json!(get(2)));
        row.insert("assignees".into(), json!(people));
        if !get(3).is_empty() {
            row.insert("priority".into(), json!(get(3).to_ascii_lowercase()));
        }
        if !get(4).is_empty() {
            row.insert("due".into(), json!(get(4)));
        }
        let row = Value::Object(row);
        rows.push(row);
    }
    let mut created = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        let rp = Params::new(row, p.cmd());
        let mut item = T::blank(ctx, project);
        apply_fields(ctx, &rp, &mut item, true).map_err(|e| p.bad("text", format!("row {}: {e}", i + 1)))?;
        let id = insert(ctx, item.clone());
        notify(ctx, &item.assignees, project, &format!("{} was assigned to you: {}", label(&item), item.title), &link(&item));
        created.push(id);
    }
    Ok(json!({ "created": created.len(), "ids": created }))
}

/// Stores a finished new item and returns its id.
pub fn insert<T: WorkItem>(ctx: &mut Ctx<'_>, mut item: T) -> Id<T> {
    let id = T::table_mut(ctx.state).alloc();
    item.set_id(id);
    T::table_mut(ctx.state).put(item);
    id
}

pub fn update<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &item) {
        return Err(EngineError::forbidden(match item.status {
            WorkStatus::Closed => format!("{} is closed; reopen it to edit", label(&item)),
            _ => format!("only the author, the manager or an admin can edit {}", label(&item)),
        }));
    }
    let before = item.clone();
    apply_fields(ctx, p, &mut item, false)?;
    let id = item.id();
    if item != before {
        let newly: Vec<Id<Person>> = item.assignees.iter().copied().filter(|a| !before.assignees.contains(a)).collect();
        notify(ctx, &newly, item.project(), &format!("{} was assigned to you: {}", label(&item), item.title), &link(&item));
        item.updated = ctx.stamp();
        T::table_mut(ctx.state).put(item.clone());
        item.after_save(Some(&before), ctx)?;
    }
    Ok(json!({ "id": id }))
}

pub fn complete<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !is_assignee(ctx, &item) && !is_manager(ctx, &item) {
        return Err(EngineError::forbidden(format!("only the people assigned to {} or its manager can mark the work done", label(&item))));
    }
    if item.status != WorkStatus::Open {
        return Err(EngineError::conflict(format!("{} is {}; only open items can be marked done", label(&item), item.status)));
    }
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = WorkStatus::ReadyForReview;
    item.work_done = Some(ctx.stamp());
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id();
    notify(
        ctx,
        &item.manager.into_iter().collect::<Vec<_>>(),
        item.project(),
        &format!("{} is ready for your review: {}", label(&item), item.title),
        &link(&item),
    );
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn send_back<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or an admin can send it back"));
    }
    if item.status != WorkStatus::ReadyForReview {
        return Err(EngineError::conflict(format!("{} is {}; only items ready for review can be sent back", label(&item), item.status)));
    }
    add_comment(ctx, &mut item, p.req_text("comment", text::LONG)?)?;
    item.status = WorkStatus::Open;
    item.work_done = None;
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id();
    notify(ctx, &item.assignees, item.project(), &format!("{} was sent back: {}", label(&item), item.title), &link(&item));
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn close<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or an admin can close it"));
    }
    if item.status == WorkStatus::Closed {
        return Err(EngineError::conflict(format!("{} is already closed", label(&item))));
    }
    item.before_close(ctx)?;
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = WorkStatus::Closed;
    item.closed = Some(ctx.stamp());
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    item.after_close(ctx)?;
    let id = item.id();
    let audience: Vec<Id<Person>> = item.assignees.iter().copied().chain(item.created.by).collect();
    notify(ctx, &audience, item.project(), &format!("{} was closed: {}", label(&item), item.title), &link(&item));
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn reopen<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !is_manager(ctx, &item) {
        return Err(EngineError::forbidden("only the item's manager or an admin can reopen it"));
    }
    if item.status != WorkStatus::Closed {
        return Err(EngineError::conflict(format!("{} is {}; only closed items can be reopened", label(&item), item.status)));
    }
    add_comment(ctx, &mut item, p.text_or_empty("comment", text::LONG)?)?;
    item.status = WorkStatus::Open;
    item.closed = None;
    item.work_done = None;
    item.recompute_ball_in_court();
    item.updated = ctx.stamp();
    let id = item.id();
    notify(ctx, &item.assignees, item.project(), &format!("{} was reopened: {}", label(&item), item.title), &link(&item));
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn comment<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if item.status == WorkStatus::Closed {
        return Err(EngineError::conflict(format!("{} is closed; reopen it to comment", label(&item))));
    }
    add_comment(ctx, &mut item, p.req_text("body", text::LONG)?)?;
    item.updated = ctx.stamp();
    let id = item.id();
    let waiting = item.ball_in_court.clone();
    notify(ctx, &waiting, item.project(), &format!("New comment on {}: {}", label(&item), item.title), &link(&item));
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

/// Who may add or remove files: whoever may edit, and assignees while the item isn't closed.
fn can_attach<T: WorkItem>(ctx: &Ctx<'_>, item: &T) -> bool {
    can_edit(ctx.state, ctx.actor, item) || (item.status != WorkStatus::Closed && is_assignee(ctx, item))
}

pub fn attach<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    const MAX_ATTACHMENTS: usize = 50;
    let mut item: T = load(ctx, p)?;
    if !can_attach(ctx, &item) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", label(&item))));
    }
    let file: FileRef = file_ref(p)?;
    if item.attachments.iter().any(|a| a.sha256 == file.sha256) {
        return Ok(json!({ "id": item.id() }));
    }
    if item.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", label(&item))));
    }
    item.attachments.push(file);
    item.updated = ctx.stamp();
    let id = item.id();
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn detach<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut item: T = load(ctx, p)?;
    if !can_attach(ctx, &item) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", label(&item))));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = item.attachments.len();
    item.attachments.retain(|a| a.sha256 != sha256);
    if item.attachments.len() == before {
        return Err(EngineError::not_found(format!("that file is not attached to this {}", T::NOUN)));
    }
    item.updated = ctx.stamp();
    let id = item.id();
    T::table_mut(ctx.state).put(item);
    Ok(json!({ "id": id }))
}

pub fn delete<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let item: T = load(ctx, p)?;
    let own_open = item.status == WorkStatus::Open && item.created.by.is_some() && item.created.by == ctx.person();
    if !is_admin(ctx, &item) && !own_open {
        return Err(EngineError::forbidden(format!("only admins can delete a {} (authors can delete their own open ones)", T::NOUN)));
    }
    let stamp = ctx.stamp();
    T::table_mut(ctx.state).update(item.id(), |i| i.deleted = Some(stamp));
    Ok(json!({ "id": item.id() }))
}

pub fn restore<T: WorkItem>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let item = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if item.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", label(item))));
    }
    T::table_mut(ctx.state).update(id, |i| i.deleted = None);
    Ok(json!({ "id": id }))
}

pub fn list<T: WorkItem>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<WorkStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    let mine = p.flag_or("mine", false)?;
    let overdue = p.flag_or("overdue", false)?;
    let today: Date = v.today();
    if p.flag_or("deleted", false)? {
        if v.level(project, T::TOOL) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&T> = T::table(v.state).iter().filter(|i| i.project() == project && i.deleted.is_some()).collect();
        return json(&bin);
    }
    let me = v.person();
    let list: Vec<&T> = items_for::<T>(v.state, v.actor, project)
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

pub fn get<T: WorkItem>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let item = in_project(T::table(v.state), project, id, T::NOUN)?;
    if !can_see(v.state, v.actor, item) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    json(item)
}

/// The commands of one work item tool. `$extra` documents the create/update fields only this kind
/// has (it starts with `; ` when present).
#[macro_export]
macro_rules! work_commands {
    ($prefix:literal, $noun:literal, $tool:expr, $ty:ty, $extra:literal) => {
        &[
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".create"),
                concat!("Add ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                concat!(
                    "project: project id; title: text; description?, location?, drawing_number?, kind?: text (kind: the trade or category; drawing_number: the sheet it refers to); priority?: low|medium|high|urgent; \
                     manager?: person id (default: you; accepts the work); assignees?: [person ids]; due?: YYYY-MM-DD",
                    $extra
                ),
                $crate::workitem::create::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".import"),
                concat!("Import ", $noun, "s"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project: project id; text: CSV lines title, location, kind, priority, due (YYYY-MM-DD), assignees (emails separated by `;`); an optional header line is skipped",
                $crate::workitem::import::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".update"),
                concat!("Edit ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                concat!("project, id: ids; then any ", $prefix, ".create field"),
                $crate::workitem::update::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".complete"),
                concat!("Mark ", $noun, " work done"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; comment?: text",
                $crate::workitem::complete::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".send_back"),
                concat!("Send ", $noun, " back"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; comment: text (what still needs doing)",
                $crate::workitem::send_back::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".close"),
                concat!("Close ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; comment?: text",
                $crate::workitem::close::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".reopen"),
                concat!("Reopen ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; comment?: text",
                $crate::workitem::reopen::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".comment"),
                concat!("Comment on ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; body: text",
                $crate::workitem::comment::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".attach"),
                concat!("Attach file to ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; sha256: text (of an uploaded file); name: text; size: integer (bytes); mime?: text",
                $crate::workitem::attach::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".detach"),
                concat!("Remove attachment from ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; sha256: text",
                $crate::workitem::detach::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".delete"),
                concat!("Delete ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids",
                $crate::workitem::delete::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".restore"),
                concat!("Restore ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Admin),
                "project, id: ids",
                $crate::workitem::restore::<$ty>,
            ),
            $crate::cmd::CommandSpec::query(
                concat!($prefix, ".list"),
                concat!("List ", $noun, "s"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Read),
                "project: project id; status?: open|ready_for_review|closed; q?: text; mine?: bool (ball in court is me); overdue?: bool; deleted?: bool (recycle bin, admins)",
                $crate::workitem::list::<$ty>,
            ),
            $crate::cmd::CommandSpec::query(
                concat!($prefix, ".get"),
                concat!("Show ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Read),
                "project, id: ids",
                $crate::workitem::get::<$ty>,
            ),
        ]
    };
}
