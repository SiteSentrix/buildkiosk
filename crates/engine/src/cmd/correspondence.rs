//! Correspondence: a letter, notice or email written as a draft, sent to its recipients (who are
//! notified), answered with replies, and closed when the matter is settled.
//!
//! A draft is private to its author and admins. A sent item is visible to anyone with Read access,
//! unless marked private (then only the author, the recipients and admins). The author and the
//! recipients may reply while it is open; the author or an admin closes and reopens it.

use buildkiosk_core::{Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::correspondence::{Correspondence, CorrespondenceStatus, Reply};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "correspondence.create",
        "Create correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project: project id; subject: text; kind?: text (Letter, Notice, Email…); date?: YYYY-MM-DD (default today); to?: [person ids]; cc?: [person ids]; reply_by?: YYYY-MM-DD; body?: text; private?: bool",
        create,
    ),
    CommandSpec::mutate(
        "correspondence.update",
        "Edit correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids; then any correspondence.create field (drafts only, unless an admin)",
        update,
    ),
    CommandSpec::mutate(
        "correspondence.send",
        "Send correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids (needs a recipient)",
        send,
    ),
    CommandSpec::mutate(
        "correspondence.reply",
        "Reply to correspondence",
        Tool::Correspondence,
        Access::Project(Level::Read),
        "project, id: ids; body: text (while it is open: the author or a recipient)",
        reply,
    ),
    CommandSpec::mutate(
        "correspondence.close",
        "Close correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids (the author or an admin)",
        close,
    ),
    CommandSpec::mutate(
        "correspondence.reopen",
        "Reopen correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids (the author or an admin)",
        reopen,
    ),
    CommandSpec::mutate(
        "correspondence.attach",
        "Attach file to correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        attach,
    ),
    CommandSpec::mutate(
        "correspondence.detach",
        "Remove file from correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "correspondence.delete",
        "Delete correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project, id: ids (the author of a draft, or an admin)",
        delete,
    ),
    CommandSpec::mutate(
        "correspondence.restore",
        "Restore correspondence",
        Tool::Correspondence,
        Access::Project(Level::Admin),
        "project, id: ids",
        restore,
    ),
    CommandSpec::query(
        "correspondence.list",
        "List correspondence",
        Tool::Correspondence,
        Access::Project(Level::Read),
        "project: project id; status?: draft|open|closed; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("correspondence.get", "Show correspondence", Tool::Correspondence, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_PEOPLE: usize = 100;
const MAX_ATTACHMENTS: usize = 50;
const MAX_REPLIES: usize = 500;

pub fn can_see(state: &buildkiosk_model::CompanyState, actor: Actor, c: &Correspondence) -> bool {
    let lvl = view::level(state, actor, c.project, Tool::Correspondence);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || c.deleted.is_some() {
        return false;
    }
    let me = actor.person();
    let mine = me.is_some() && c.created.by == me;
    let addressed = me.is_some_and(|m| c.to.contains(&m) || c.cc.contains(&m));
    match c.status {
        CorrespondenceStatus::Draft => mine,
        _ => !c.private || mine || addressed,
    }
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Correspondence> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Correspondence> = p.id("id")?;
    let c = in_project(&ctx.state.correspondence, project, id, "correspondence")?;
    if c.deleted.is_some() || !can_see(ctx.state, ctx.actor, c) {
        return Err(EngineError::not_found(format!("correspondence {id} not found in this project")));
    }
    Ok(c.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Correspondence) == Level::Admin
}

fn is_author(ctx: &Ctx<'_>, c: &Correspondence) -> bool {
    ctx.person().is_some() && c.created.by == ctx.person()
}

/// A draft is edited by its author; anything sent only by an admin.
fn can_edit(ctx: &Ctx<'_>, c: &Correspondence) -> bool {
    is_admin(ctx, c.project) || (c.status == CorrespondenceStatus::Draft && is_author(ctx, c))
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

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, c: &mut Correspondence) -> Result<()> {
    if p.present("subject") {
        c.subject = p.req_line("subject", text::SHORT)?;
    }
    if p.present("kind") {
        c.kind = p.line_or_empty("kind", 50)?;
    }
    if p.present("date")
        && let Some(d) = p.date("date")?
    {
        c.date = d;
    }
    if p.present("to") {
        c.to = people(ctx, p, "to")?;
    }
    if p.present("cc") {
        c.cc = people(ctx, p, "cc")?;
    }
    if p.present("reply_by") {
        c.reply_by = p.date("reply_by")?;
    }
    if p.present("body") {
        c.body = p.text_or_empty("body", text::LONG)?;
    }
    if let Some(x) = p.flag("private")? {
        c.private = x;
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.correspondence.iter().filter(|c| c.project == project).map(|c| c.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut c = Correspondence {
        id: ctx.state.correspondence.peek_next(),
        project,
        number,
        subject: String::new(),
        kind: String::new(),
        status: CorrespondenceStatus::Draft,
        to: Vec::new(),
        cc: Vec::new(),
        date: ctx.today(),
        reply_by: None,
        body: String::new(),
        replies: Vec::new(),
        attachments: Vec::new(),
        private: false,
        sent: None,
        closed: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut c)?;
    if c.subject.is_empty() {
        return Err(p.bad("subject", "is required"));
    }
    let id = ctx.state.correspondence.alloc();
    c.id = id;
    ctx.state.correspondence.put(c);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !can_edit(ctx, &c) {
        return Err(EngineError::forbidden(format!("you can't edit {}", c.label())));
    }
    apply(ctx, p, &mut c)?;
    c.updated = ctx.stamp();
    let id = c.id;
    ctx.state.correspondence.put(c);
    Ok(json!({ "id": id }))
}

fn link(c: &Correspondence) -> String {
    format!("/projects/{}/correspondence/{}", c.project, c.id)
}

fn send(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !can_edit(ctx, &c) {
        return Err(EngineError::forbidden(format!("you can't send {}", c.label())));
    }
    if c.status != CorrespondenceStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already sent", c.label())));
    }
    if c.to.is_empty() {
        return Err(EngineError::conflict("choose at least one recipient"));
    }
    c.status = CorrespondenceStatus::Open;
    c.sent = Some(ctx.stamp());
    c.updated = ctx.stamp();
    let (to, cc, project, l) = (c.to.clone(), c.cc.clone(), c.project, link(&c));
    let (to_text, cc_text) =
        (format!("{} was sent to you: {}", c.label(), c.subject), format!("{} was sent, with a copy to you: {}", c.label(), c.subject));
    let id = c.id;
    ctx.state.correspondence.put(c);
    notify(ctx, &to, project, &to_text, &l);
    notify(ctx, &cc, project, &cc_text, &l);
    Ok(json!({ "id": id }))
}

fn reply(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if c.status != CorrespondenceStatus::Open {
        return Err(EngineError::conflict("only open correspondence can be replied to"));
    }
    let me = ctx.person();
    if !(is_admin(ctx, c.project) || is_author(ctx, &c) || me.is_some_and(|m| c.to.contains(&m) || c.cc.contains(&m))) {
        return Err(EngineError::forbidden("only the author and the people it was sent to can reply"));
    }
    if c.replies.len() >= MAX_REPLIES {
        return Err(EngineError::conflict(format!("{} already has {MAX_REPLIES} replies", c.label())));
    }
    let body = p.req_text("body", text::LONG)?;
    c.replies.push(Reply { by: me, at: ctx.at, body });
    c.updated = ctx.stamp();
    let mut audience: Vec<Id<Person>> = c.to.iter().chain(c.cc.iter()).copied().chain(c.created.by).collect();
    audience.sort();
    audience.dedup();
    let (project, l, text) = (c.project, link(&c), format!("New reply on {}: {}", c.label(), c.subject));
    let id = c.id;
    ctx.state.correspondence.put(c);
    notify(ctx, &audience, project, &text, &l);
    Ok(json!({ "id": id }))
}

fn set_status(ctx: &mut Ctx<'_>, p: &Params<'_>, from: CorrespondenceStatus, to: CorrespondenceStatus) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !(is_admin(ctx, c.project) || is_author(ctx, &c)) {
        return Err(EngineError::forbidden("only the author or an admin can do that"));
    }
    if c.status != from {
        return Err(EngineError::conflict(format!("{} is {}", c.label(), c.status)));
    }
    c.status = to;
    c.closed = if to == CorrespondenceStatus::Closed { Some(ctx.stamp()) } else { None };
    c.updated = ctx.stamp();
    let id = c.id;
    ctx.state.correspondence.put(c);
    Ok(json!({ "id": id }))
}

fn close(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    set_status(ctx, p, CorrespondenceStatus::Open, CorrespondenceStatus::Closed)
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    set_status(ctx, p, CorrespondenceStatus::Closed, CorrespondenceStatus::Open)
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !can_edit(ctx, &c) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", c.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if c.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": c.id }));
    }
    if c.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", c.label())));
    }
    c.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    c.updated = ctx.stamp();
    let id = c.id;
    ctx.state.correspondence.put(c);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !can_edit(ctx, &c) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", c.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = c.attachments.len();
    c.attachments.retain(|a| a.sha256 != sha256);
    if c.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached"));
    }
    c.updated = ctx.stamp();
    let id = c.id;
    ctx.state.correspondence.put(c);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c = load(ctx, p)?;
    if !can_edit(ctx, &c) {
        return Err(EngineError::forbidden("only the author of a draft or an admin can delete correspondence"));
    }
    let stamp = ctx.stamp();
    ctx.state.correspondence.update(c.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": c.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Correspondence> = p.id("id")?;
    let c = in_project(&ctx.state.correspondence, project, id, "correspondence")?;
    if c.deleted.is_none() {
        return Err(EngineError::conflict("it is not in the recycle bin"));
    }
    ctx.state.correspondence.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// What the actor may see, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Correspondence> {
    let mut v: Vec<&Correspondence> =
        state.correspondence.iter().filter(|c| c.project == project && c.deleted.is_none() && can_see(state, actor, c)).collect();
    v.sort_by_key(|c| std::cmp::Reverse(c.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Correspondence) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Correspondence> = v.state.correspondence.iter().filter(|c| c.project == project && c.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<CorrespondenceStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&Correspondence> = visible(v.state, v.actor, project)
        .into_iter()
        .filter(|c| status.is_none_or(|s| c.status == s))
        .filter(|c| {
            q.as_ref().is_none_or(|q| {
                c.subject.to_lowercase().contains(q.as_str())
                    || c.kind.to_lowercase().contains(q.as_str())
                    || c.body.to_lowercase().contains(q.as_str())
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Correspondence> = p.id("id")?;
    let c = in_project(&v.state.correspondence, project, id, "correspondence")?;
    if c.deleted.is_some() || !can_see(v.state, v.actor, c) {
        return Err(EngineError::not_found(format!("correspondence {id} not found in this project")));
    }
    json(c)
}
