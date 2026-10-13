//! Correspondence: a letter, notice or email written as a draft, sent to its recipients (who are
//! notified), answered with replies, and closed when the matter is settled.
//!
//! A draft is private to its author and admins. A sent item is visible to anyone with Read access,
//! unless marked private (then only the author, the recipients and admins). The author and the
//! recipients may reply while it is open; the author or an admin closes and reopens it.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::correspondence::{Correspondence, CorrespondenceStatus, CorrespondenceType, Reply, Sender};
use buildkiosk_model::directory::Person;
use buildkiosk_model::document::Document;
use buildkiosk_model::project::Project;
use buildkiosk_model::rfi::Rfi;
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
        "project: project id; subject: text; kind?: text (Letter, Notice, Email…); date?: YYYY-MM-DD (default today); to?: [person ids]; cc?: [person ids]; reply_by?: YYYY-MM-DD; body?: text; private?: bool; rfis?: [RFI ids]; documents?: [document ids] (links). When the project has configured types, kind must be one of them and the item gets that type's next reference (LTR-003)",
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
        "correspondence.file_incoming",
        "File incoming correspondence",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project: project id; subject: text; from_name: who sent it (outside the company); from_email?: text; to: [person ids who should see it]; then any correspondence.create field (it is filed as open, with the people in `to` notified)",
        file_incoming,
    ),
    CommandSpec::mutate(
        "correspondence.import_email",
        "File an email",
        Tool::Correspondence,
        Access::Project(Level::Standard),
        "project: project id; raw: the email as saved by a mail program (.eml text: headers, blank line, body; plain text or quoted-printable); kind?: text; to?: [person ids] (default: the project people whose address is in To or Cc)",
        import_email,
    ),
    CommandSpec::mutate(
        "correspondence_type.save",
        "Save correspondence type",
        Tool::Correspondence,
        Access::Project(Level::Admin),
        "project: project id; id?: type id (changes it); name: text (Letter, Notice…); prefix: 1 to 8 letters or digits that start references (LTR)",
        type_save,
    ),
    CommandSpec::mutate(
        "correspondence_type.delete",
        "Delete correspondence type",
        Tool::Correspondence,
        Access::Project(Level::Admin),
        "project, id: ids (items already filed keep their reference)",
        type_delete,
    ),
    CommandSpec::query(
        "correspondence_type.list",
        "List correspondence types",
        Tool::Correspondence,
        Access::Project(Level::Read),
        "project: project id",
        type_list,
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
const MAX_LINKS: usize = 50;

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
    if p.present("rfis") {
        let mut ids = p.ids::<Rfi>("rfis")?;
        ids.sort();
        ids.dedup();
        if ids.len() > MAX_LINKS {
            return Err(p.bad("rfis", format!("lists more than {MAX_LINKS} RFIs")));
        }
        for id in &ids {
            let ok = in_project(&ctx.state.rfis, c.project, *id, "RFI")
                .ok()
                .is_some_and(|r| r.deleted.is_none() && view::can_see_rfi(ctx.state, ctx.actor, r));
            if !ok {
                return Err(p.bad("rfis", format!("RFI {id} not found in this project")));
            }
        }
        c.rfis = ids;
    }
    if p.present("documents") {
        let mut ids = p.ids::<Document>("documents")?;
        ids.sort();
        ids.dedup();
        if ids.len() > MAX_LINKS {
            return Err(p.bad("documents", format!("lists more than {MAX_LINKS} documents")));
        }
        for id in &ids {
            let ok = in_project(&ctx.state.documents, c.project, *id, "document")
                .ok()
                .is_some_and(|d| d.deleted.is_none() && view::can_see_document(ctx.state, ctx.actor, d));
            if !ok {
                return Err(p.bad("documents", format!("document {id} not found in this project")));
            }
        }
        c.documents = ids;
    }
    Ok(())
}

/// The project's configured types, by name.
pub fn types(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&CorrespondenceType> {
    let mut v: Vec<&CorrespondenceType> = state.correspondence_types.iter().filter(|t| t.project == project && t.deleted.is_none()).collect();
    v.sort_by_key(|t| t.name.to_lowercase());
    v
}

/// Files the item under the configured type its `kind` names and gives it that type's next reference. A project
/// with no types keeps the free-text kind and the plain number.
fn assign_type(ctx: &Ctx<'_>, p: &Params<'_>, c: &mut Correspondence) -> Result<()> {
    let list = types(ctx.state, c.project);
    if list.is_empty() {
        return Ok(());
    }
    let Some(t) = list.iter().find(|t| t.name.eq_ignore_ascii_case(c.kind.trim())) else {
        let names: Vec<&str> = list.iter().map(|t| t.name.as_str()).collect();
        return Err(p.bad("kind", format!("choose one of the project's types: {}", names.join(", "))));
    };
    if c.type_id != Some(t.id) {
        let n = ctx
            .state
            .correspondence
            .iter()
            .filter(|x| x.project == c.project && x.type_id == Some(t.id) && x.id != c.id)
            .map(|x| x.type_number)
            .max()
            .unwrap_or(0)
            .saturating_add(1);
        c.type_id = Some(t.id);
        c.type_number = n;
        c.reference = format!("{}-{n:03}", t.prefix);
    }
    c.kind = t.name.clone();
    Ok(())
}

fn type_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", 50)?;
    let prefix = p.req_line("prefix", 8)?.to_ascii_uppercase();
    if !prefix.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Err(p.bad("prefix", "may hold only letters and digits"));
    }
    let existing = p.opt_id::<CorrespondenceType>("id")?;
    if let Some(id) = existing {
        let t = in_project(&ctx.state.correspondence_types, project, id, "correspondence type")?;
        if t.deleted.is_some() {
            return Err(EngineError::not_found(format!("correspondence type {id} not found in this project")));
        }
    }
    let others = types(ctx.state, project);
    if others.iter().any(|t| Some(t.id) != existing && t.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already a type of this project"));
    }
    if others.iter().any(|t| Some(t.id) != existing && t.prefix == prefix) {
        return Err(p.bad("prefix", "is already used by another type"));
    }
    if existing.is_none() && others.len() >= 30 {
        return Err(EngineError::conflict("a project keeps at most 30 correspondence types"));
    }
    let stamp = ctx.stamp();
    let id = match existing {
        Some(id) => {
            ctx.state.correspondence_types.update(id, |t| (t.name, t.prefix) = (name, prefix));
            id
        }
        None => {
            let id = ctx.state.correspondence_types.alloc();
            ctx.state.correspondence_types.put(CorrespondenceType { id, project, name, prefix, created: stamp, deleted: None });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn type_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<CorrespondenceType> = p.id("id")?;
    if in_project(&ctx.state.correspondence_types, project, id, "correspondence type")?.deleted.is_some() {
        return Err(EngineError::not_found(format!("correspondence type {id} not found in this project")));
    }
    let stamp = ctx.stamp();
    ctx.state.correspondence_types.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn type_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    json(&types(v.state, p.id("project")?))
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut c = blank(ctx, project);
    let number = c.number;
    apply(ctx, p, &mut c)?;
    if c.subject.is_empty() {
        return Err(p.bad("subject", "is required"));
    }
    assign_type(ctx, p, &mut c)?;
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
    let old_kind = c.kind.clone();
    apply(ctx, p, &mut c)?;
    if p.present("kind") && (c.kind != old_kind || c.type_id.is_none()) {
        assign_type(ctx, p, &mut c)?;
    }
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

// -- incoming

fn file_new(ctx: &mut Ctx<'_>, p: &Params<'_>, mut c: Correspondence, notify_to: bool) -> Result<Value> {
    assign_type(ctx, p, &mut c)?;
    let stamp = ctx.stamp();
    c.status = CorrespondenceStatus::Open;
    c.sent = Some(stamp);
    let id = ctx.state.correspondence.alloc();
    c.id = id;
    let (to, cc, project, l, number) = (c.to.clone(), c.cc.clone(), c.project, link(&c), c.number);
    let text = format!("{} came in from {}: {}", c.label(), c.incoming.as_ref().map(|s| s.name.as_str()).unwrap_or("outside"), c.subject);
    ctx.state.correspondence.put(c);
    if notify_to {
        notify(ctx, &to, project, &text, &l);
        notify(ctx, &cc, project, &text, &l);
    }
    Ok(json!({ "id": id, "number": number }))
}

fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Correspondence {
    let number = ctx.state.correspondence.iter().filter(|c| c.project == project).map(|c| c.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    Correspondence {
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
        type_id: None,
        type_number: 0,
        reference: String::new(),
        rfis: Vec::new(),
        documents: Vec::new(),
        incoming: None,
        sent: None,
        closed: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    }
}

fn file_incoming(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut c = blank(ctx, project);
    apply(ctx, p, &mut c)?;
    if c.subject.is_empty() {
        return Err(p.bad("subject", "is required"));
    }
    if c.to.is_empty() {
        return Err(p.bad("to", "name at least one person who should see it"));
    }
    c.incoming = Some(Sender { name: p.req_line("from_name", 200)?, email: p.line_or_empty("from_email", 200)? });
    file_new(ctx, p, c, true)
}

/// The parts of an email the importer reads.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedEmail {
    pub from_name: String,
    pub from_email: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub date: Option<Date>,
    pub body: String,
}

fn headers_of(block: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in block.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some(last) = out.last_mut() {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
        } else if let Some((k, v)) = line.split_once(':') {
            out.push((k.trim().to_ascii_lowercase(), v.trim().to_string()));
        }
    }
    out
}

fn header<'a>(h: &'a [(String, String)], key: &str) -> &'a str {
    h.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str()).unwrap_or("")
}

/// "Jo Smith <jo@example.com>" → ("Jo Smith", "jo@example.com"); a bare address → ("", address).
fn address(raw: &str) -> (String, String) {
    let raw = raw.trim();
    match (raw.rfind('<'), raw.rfind('>')) {
        (Some(a), Some(b)) if a < b => {
            let name = raw.get(..a).unwrap_or("").trim().trim_matches('"').trim().to_string();
            let email = raw.get(a + 1..b).unwrap_or("").trim().to_string();
            (name, email)
        }
        _ => (String::new(), raw.to_string()),
    }
}

fn address_list(raw: &str) -> Vec<String> {
    raw.split(',').map(|a| address(a).1.to_lowercase()).filter(|a| a.contains('@')).collect()
}

fn decode_quoted_printable(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'=' {
            if bytes.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if bytes.get(i + 1) == Some(&b'\r') && bytes.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            let hex = bytes.get(i + 1..i + 3).and_then(|h| std::str::from_utf8(h).ok()).and_then(|h| u8::from_str_radix(h, 16).ok());
            if let Some(v) = hex {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// "Tue, 13 Oct 2026 09:15:00 -0500" → 2026-10-13.
fn email_date(raw: &str) -> Option<Date> {
    let toks: Vec<&str> = raw.split_whitespace().map(|t| t.trim_end_matches(',')).collect();
    let months = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    for w in toks.windows(3) {
        let (Some(&d), Some(&m), Some(&y)) = (w.first(), w.get(1), w.get(2)) else { continue };
        let (Ok(day), Ok(year)) = (d.parse::<u32>(), y.parse::<i32>()) else { continue };
        let Some(month) = months.iter().position(|x| m.len() >= 3 && m.get(..3).is_some_and(|p| p.eq_ignore_ascii_case(x))) else { continue };
        return Date::from_ymd(year, u32::try_from(month).unwrap_or(0) + 1, day);
    }
    None
}

/// Reads a saved email: the headers, and the plain-text body (the first text part of a multipart message).
pub fn parse_email(raw: &str) -> ParsedEmail {
    let norm = raw.replace("\r\n", "\n");
    let (head, body) = norm.split_once("\n\n").unwrap_or((norm.as_str(), ""));
    let h = headers_of(head);
    let (from_name, from_email) = address(header(&h, "from"));
    let ctype = header(&h, "content-type").to_string();
    let mut text_body = body.to_string();
    let mut encoding = header(&h, "content-transfer-encoding").to_ascii_lowercase();
    if ctype.to_ascii_lowercase().contains("multipart")
        && let Some(b) = ctype.split("boundary=").nth(1)
    {
        let boundary = b.split(';').next().unwrap_or("").trim().trim_matches('"').to_string();
        if !boundary.is_empty() {
            let marker = format!("--{boundary}");
            for part in body.split(marker.as_str()).skip(1) {
                let (ph, pb) = part.trim_start_matches('\n').split_once("\n\n").unwrap_or(("", part));
                let ph = headers_of(ph);
                let ct = header(&ph, "content-type").to_ascii_lowercase();
                if ct.is_empty() || ct.starts_with("text/plain") {
                    text_body = pb.to_string();
                    encoding = header(&ph, "content-transfer-encoding").to_ascii_lowercase();
                    break;
                }
            }
        }
    }
    let body = if encoding == "quoted-printable" {
        decode_quoted_printable(&text_body)
    } else if encoding == "base64" {
        "(The message text is encoded in a way BuildKiosk does not read; open the original email.)".to_string()
    } else {
        text_body
    };
    ParsedEmail {
        from_name,
        from_email,
        to: address_list(header(&h, "to")),
        cc: address_list(header(&h, "cc")),
        subject: header(&h, "subject").to_string(),
        date: email_date(header(&h, "date")),
        body: body.trim().to_string(),
    }
}

fn import_email(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let raw = p.req_text("raw", 500_000)?;
    let e = parse_email(&raw);
    if e.subject.is_empty() && e.body.is_empty() {
        return Err(p.bad("raw", "does not look like an email (no Subject line or text)"));
    }
    let find = |addr: &str| {
        ctx.state.people.iter().find(|x| x.deleted.is_none() && x.email.as_deref().is_some_and(|m| m.eq_ignore_ascii_case(addr))).map(|x| x.id)
    };
    let mut c = blank(ctx, project);
    if p.present("to") {
        c.to = people(ctx, p, "to")?;
    } else {
        c.to = e.to.iter().filter_map(|a| find(a)).collect();
    }
    c.cc = e.cc.iter().filter_map(|a| find(a)).filter(|x| !c.to.contains(x)).collect();
    c.to.sort();
    c.to.dedup();
    if c.to.is_empty() && c.cc.is_empty() {
        return Err(p.bad("to", "none of the addresses in To or Cc belong to people on this company; name who should see it with `to`"));
    }
    if c.to.is_empty() {
        c.to = std::mem::take(&mut c.cc);
    }
    c.subject = text::line(&e.subject, text::SHORT).map_err(|er| p.bad("raw", format!("subject: {er}")))?;
    if c.subject.is_empty() {
        c.subject = "(no subject)".into();
    }
    c.kind = if p.present("kind") { p.line_or_empty("kind", 50)? } else { "Email".into() };
    if let Some(d) = e.date {
        c.date = d;
    }
    c.body = text::multiline(&e.body, text::LONG).map_err(|er| p.bad("raw", format!("text: {er}")))?;
    let name = if e.from_name.is_empty() { e.from_email.clone() } else { e.from_name.clone() };
    c.incoming = Some(Sender { name: text::line(&name, 200).unwrap_or_default(), email: text::line(&e.from_email, 200).unwrap_or_default() });
    // The kind falls back to the first configured type when the project has types but none named Email.
    if !p.present("kind") {
        let list = types(ctx.state, project);
        if !list.iter().any(|t| t.name.eq_ignore_ascii_case(&c.kind))
            && let Some(t) = list.first()
        {
            c.kind = t.name.clone();
        }
    }
    file_new(ctx, p, c, true)
}
