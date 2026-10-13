//! Transmittals: written as a draft by their author, then sent to the recipients, who are
//! notified. Once sent, only admins change one (to fix a mistake); files can be added until then.
//!
//! Who sees what: a draft is private to its author and Transmittals admins; a sent transmittal is
//! visible to anyone with Read access, unless marked private, when only the author, the people it
//! was sent to and admins see it.

use buildkiosk_core::{Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::transmittal::{Purpose, Transmittal, TransmittalItem, TransmittalStatus};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "transmittal.create",
        "Create transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project: project id; subject: text; purpose?: for_information|for_review|for_approval|for_construction|for_record|as_requested; to?: [person ids]; cc?: [person ids]; due?: YYYY-MM-DD; message?: text; private?: bool; items?: [{description: text, copies?: number}]",
        create,
    ),
    CommandSpec::mutate(
        "transmittal.update",
        "Edit transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project, id: ids; then any transmittal.create field",
        update,
    ),
    CommandSpec::mutate(
        "transmittal.send",
        "Send transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project, id: ids (needs at least one recipient and one item or file)",
        send,
    ),
    CommandSpec::mutate(
        "transmittal.attach",
        "Attach file to transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        attach,
    ),
    CommandSpec::mutate(
        "transmittal.detach",
        "Remove file from transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "transmittal.delete",
        "Delete transmittal",
        Tool::Transmittals,
        Access::Project(Level::Standard),
        "project, id: ids (the author of a draft, or an admin)",
        delete,
    ),
    CommandSpec::mutate("transmittal.restore", "Restore transmittal", Tool::Transmittals, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "transmittal.list",
        "List transmittals",
        Tool::Transmittals,
        Access::Project(Level::Read),
        "project: project id; status?: draft|sent; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("transmittal.get", "Show transmittal", Tool::Transmittals, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_ITEMS: usize = 100;
const MAX_PEOPLE: usize = 100;
const MAX_ATTACHMENTS: usize = 50;

/// Whether the actor may open the transmittal.
pub fn can_see(state: &buildkiosk_model::CompanyState, actor: Actor, t: &Transmittal) -> bool {
    let lvl = view::level(state, actor, t.project, Tool::Transmittals);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || t.deleted.is_some() {
        return false;
    }
    let me = actor.person();
    let mine = me.is_some() && t.created.by == me;
    let addressed = me.is_some_and(|m| t.to.contains(&m) || t.cc.contains(&m));
    match t.status {
        TransmittalStatus::Draft => mine,
        TransmittalStatus::Sent => !t.private || mine || addressed,
    }
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Transmittal> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Transmittal> = p.id("id")?;
    let t = in_project(&ctx.state.transmittals, project, id, "transmittal")?;
    if t.deleted.is_some() || !can_see(ctx.state, ctx.actor, t) {
        return Err(EngineError::not_found(format!("transmittal {id} not found in this project")));
    }
    Ok(t.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Transmittals) == Level::Admin
}

fn is_author(ctx: &Ctx<'_>, t: &Transmittal) -> bool {
    ctx.person().is_some() && t.created.by == ctx.person()
}

/// Draft: its author or an admin. Sent: admins only.
fn can_edit(ctx: &Ctx<'_>, t: &Transmittal) -> bool {
    is_admin(ctx, t.project) || (t.status == TransmittalStatus::Draft && is_author(ctx, t))
}

fn people(ctx: &Ctx<'_>, p: &Params<'_>, key: &str) -> Result<Vec<Id<Person>>> {
    let mut ids = p.ids::<Person>(key)?;
    ids.sort();
    ids.dedup();
    if ids.len() > MAX_PEOPLE {
        return Err(p.bad(key, format!("lists more than {MAX_PEOPLE} people")));
    }
    for id in &ids {
        ctx.check_person(*id)?;
    }
    Ok(ids)
}

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, t: &mut Transmittal) -> Result<()> {
    if p.present("subject") {
        t.subject = p.req_line("subject", text::SHORT)?;
    }
    if let Some(x) = p.choice::<Purpose>("purpose")? {
        t.purpose = x;
    }
    if p.present("to") {
        t.to = people(ctx, p, "to")?;
    }
    if p.present("cc") {
        t.cc = people(ctx, p, "cc")?;
    }
    if p.present("due") {
        t.due = p.date("due")?;
    }
    if p.present("message") {
        t.message = p.text_or_empty("message", text::LONG)?;
    }
    if let Some(x) = p.flag("private")? {
        t.private = x;
    }
    if p.present("items") {
        let rows = p.items("items")?;
        if rows.len() > MAX_ITEMS {
            return Err(p.bad("items", format!("holds more than {MAX_ITEMS} items")));
        }
        let mut out = Vec::new();
        for r in rows {
            let description = r.line_or_empty("description", text::SHORT)?;
            if description.is_empty() {
                continue; // blank spare rows
            }
            let copies = r.int("copies", 0..=100_000)?.unwrap_or(1);
            out.push(TransmittalItem { description, copies: u32::try_from(copies).unwrap_or(1) });
        }
        t.items = out;
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.transmittals.iter().filter(|t| t.project == project).map(|t| t.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let id = ctx.state.transmittals.peek_next();
    let mut t = Transmittal {
        id,
        project,
        number,
        subject: String::new(),
        status: TransmittalStatus::Draft,
        purpose: Purpose::default(),
        to: Vec::new(),
        cc: Vec::new(),
        due: None,
        message: String::new(),
        items: Vec::new(),
        attachments: Vec::new(),
        private: false,
        sent: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut t)?;
    if t.subject.is_empty() {
        return Err(p.bad("subject", "is required"));
    }
    let id = ctx.state.transmittals.alloc();
    t.id = id;
    ctx.state.transmittals.put(t);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut t = load(ctx, p)?;
    if !can_edit(ctx, &t) {
        return Err(EngineError::forbidden(format!("you can't edit {}", t.label())));
    }
    apply(ctx, p, &mut t)?;
    t.updated = ctx.stamp();
    let id = t.id;
    ctx.state.transmittals.put(t);
    Ok(json!({ "id": id }))
}

fn link(t: &Transmittal) -> String {
    format!("/projects/{}/transmittals/{}", t.project, t.id)
}

fn send(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut t = load(ctx, p)?;
    if !can_edit(ctx, &t) {
        return Err(EngineError::forbidden(format!("you can't send {}", t.label())));
    }
    if t.status != TransmittalStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already sent", t.label())));
    }
    if t.to.is_empty() {
        return Err(EngineError::conflict("choose at least one recipient"));
    }
    if t.items.is_empty() && t.attachments.is_empty() {
        return Err(EngineError::conflict("list at least one item or attach a file"));
    }
    t.status = TransmittalStatus::Sent;
    t.sent = Some(ctx.stamp());
    t.updated = ctx.stamp();
    let (to, cc, project, l) = (t.to.clone(), t.cc.clone(), t.project, link(&t));
    let (to_text, cc_text) =
        (format!("{} was sent to you: {}", t.label(), t.subject), format!("{} was sent, with a copy to you: {}", t.label(), t.subject));
    let id = t.id;
    ctx.state.transmittals.put(t);
    notify(ctx, &to, project, &to_text, &l);
    notify(ctx, &cc, project, &cc_text, &l);
    Ok(json!({ "id": id }))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut t = load(ctx, p)?;
    if !can_edit(ctx, &t) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", t.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if t.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": t.id }));
    }
    if t.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", t.label())));
    }
    t.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    t.updated = ctx.stamp();
    let id = t.id;
    ctx.state.transmittals.put(t);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut t = load(ctx, p)?;
    if !can_edit(ctx, &t) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", t.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = t.attachments.len();
    t.attachments.retain(|a| a.sha256 != sha256);
    if t.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this transmittal"));
    }
    t.updated = ctx.stamp();
    let id = t.id;
    ctx.state.transmittals.put(t);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let t = load(ctx, p)?;
    if !(is_admin(ctx, t.project) || (t.status == TransmittalStatus::Draft && is_author(ctx, &t))) {
        return Err(EngineError::forbidden("only the author of a draft or an admin can delete a transmittal"));
    }
    let stamp = ctx.stamp();
    ctx.state.transmittals.update(t.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": t.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Transmittal> = p.id("id")?;
    let t = in_project(&ctx.state.transmittals, project, id, "transmittal")?;
    if t.deleted.is_none() {
        return Err(EngineError::conflict("the transmittal is not in the recycle bin"));
    }
    ctx.state.transmittals.update(id, |t| t.deleted = None);
    Ok(json!({ "id": id }))
}

pub fn visible(state: &buildkiosk_model::CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Transmittal> {
    let mut v: Vec<&Transmittal> =
        state.transmittals.iter().filter(|t| t.project == project && t.deleted.is_none() && can_see(state, actor, t)).collect();
    v.sort_by_key(|t| std::cmp::Reverse(t.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Transmittals) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Transmittal> = v.state.transmittals.iter().filter(|t| t.project == project && t.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<TransmittalStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&Transmittal> = visible(v.state, v.actor, project)
        .into_iter()
        .filter(|t| status.is_none_or(|s| t.status == s))
        .filter(|t| {
            q.as_ref().is_none_or(|q| {
                t.subject.to_lowercase().contains(q.as_str()) || t.items.iter().any(|i| i.description.to_lowercase().contains(q.as_str()))
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Transmittal> = p.id("id")?;
    let t = in_project(&v.state.transmittals, project, id, "transmittal")?;
    if t.deleted.is_some() || !can_see(v.state, v.actor, t) {
        return Err(EngineError::not_found(format!("transmittal {id} not found in this project")));
    }
    json(t)
}
