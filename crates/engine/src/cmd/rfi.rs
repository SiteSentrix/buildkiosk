//! RFIs: questions that need a documented answer.
//!
//! Workflow: a Standard user drafts an RFI → its manager (or an RFI admin) opens it to the
//! assignees, which sets the due date and the ball in court → assignees respond → the manager marks
//! the answer that settles it as official and closes it. Closed RFIs can be reopened. Deleting
//! moves an RFI to the recycle bin.
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - edit: the author or manager while it's a draft, the manager while it's open, RFI admins always;
//! - open, close, reopen, mark official: the manager or an RFI admin;
//! - respond: assignees, the manager, RFI admins;
//! - delete: RFI admins, or the author of a draft; restore: RFI admins.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::{Person, Vendor};
use buildkiosk_model::project::Project;
use buildkiosk_model::rfi::{Rfi, RfiResponse, RfiStatus};
use buildkiosk_model::{Impact, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

const FIELDS: &str = "subject: text; question?: text; manager?: person id (default: you); assignees?: [person ids]; received_from?: person id; \
     responsible_vendor?: company id; distribution?: [person ids]; due?: YYYY-MM-DD; spec_section?, drawing_number?, location?: text; \
     cost_impact?, schedule_impact?: tbd|no|yes; cost_amount?: amount; schedule_days?: integer; private?: bool";

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "rfi.create",
        "Create RFI",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project: project id; then the RFI fields: subject: text; question?: text; manager?: person id (default: you); assignees?: [person ids]; \
         received_from?: person id; responsible_vendor?: company id; distribution?: [person ids]; due?: YYYY-MM-DD; spec_section?, drawing_number?, \
         location?: text; cost_impact?, schedule_impact?: tbd|no|yes; cost_amount?: amount; schedule_days?: integer; private?: bool; \
         open?: bool (send it to the assignees right away)",
        create,
    ),
    CommandSpec::mutate("rfi.update", "Edit RFI", Tool::Rfis, Access::Project(Level::Standard), "project, id: ids; then any RFI field", update),
    CommandSpec::mutate(
        "rfi.open",
        "Open RFI",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project, id: ids; due?: YYYY-MM-DD (default: today plus the project's RFI working days)",
        open,
    ),
    CommandSpec::mutate(
        "rfi.respond",
        "Respond to RFI",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project, id: ids; body: text; official?: bool (manager or RFI admin)",
        respond,
    ),
    CommandSpec::mutate(
        "rfi.mark_official",
        "Mark official response",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project, id: ids; response: response number within the RFI; official?: bool (default true)",
        mark_official,
    ),
    CommandSpec::mutate(
        "rfi.attach",
        "Attach file to RFI",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text (of an uploaded file); name: text; size: integer (bytes); mime?: text",
        attach,
    ),
    CommandSpec::mutate(
        "rfi.detach",
        "Remove attachment from RFI",
        Tool::Rfis,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text",
        detach,
    ),
    CommandSpec::mutate("rfi.close", "Close RFI", Tool::Rfis, Access::Project(Level::Standard), "project, id: ids", close),
    CommandSpec::mutate("rfi.reopen", "Reopen RFI", Tool::Rfis, Access::Project(Level::Standard), "project, id: ids", reopen),
    CommandSpec::mutate("rfi.delete", "Delete RFI", Tool::Rfis, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("rfi.restore", "Restore RFI", Tool::Rfis, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "rfi.list",
        "List RFIs",
        Tool::Rfis,
        Access::Project(Level::Read),
        "project: project id; status?: draft|open|closed; q?: text; mine?: bool (ball in court is me); overdue?: bool; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("rfi.get", "Show RFI", Tool::Rfis, Access::Project(Level::Read), "project, id: ids", get),
];

/// The RFI fields a person can set, for the forms in the UI.
pub const EDITABLE_FIELDS: &str = FIELDS;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<(Id<Project>, Rfi)> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Rfi> = p.id("id")?;
    let rfi = in_project(&ctx.state.rfis, project, id, "RFI")?;
    if !view::can_see_rfi(ctx.state, ctx.actor, rfi) {
        return Err(EngineError::not_found(format!("RFI {id} not found in this project")));
    }
    Ok((project, rfi.clone()))
}

fn is_admin(ctx: &Ctx<'_>, rfi: &Rfi) -> bool {
    ctx.level(rfi.project, Tool::Rfis) == Level::Admin
}

fn is_manager(ctx: &Ctx<'_>, rfi: &Rfi) -> bool {
    is_admin(ctx, rfi) || (ctx.person().is_some() && rfi.manager == ctx.person())
}

/// Whether the actor may edit the RFI's fields now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, rfi: &Rfi) -> bool {
    let lvl = view::level(state, actor, rfi.project, Tool::Rfis);
    if lvl == Level::Admin {
        return true;
    }
    let me = actor.person();
    if lvl < Level::Standard || me.is_none() {
        return false;
    }
    match rfi.status {
        RfiStatus::Draft => rfi.created.by == me || rfi.manager == me,
        RfiStatus::Open => rfi.manager == me,
        RfiStatus::Closed => false,
    }
}

/// The default due date: today plus the project's (else the company's) RFI working days.
fn default_due(ctx: &Ctx<'_>, project: Id<Project>) -> Option<Date> {
    let days = ctx.state.projects.get(project).and_then(|p| p.rfi_due_days).or_else(|| ctx.state.company().map(|c| c.rfi_due_days)).unwrap_or(7);
    ctx.today().add_business_days(days)
}

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, rfi: &mut Rfi, creating: bool) -> Result<()> {
    if creating || p.present("subject") {
        rfi.subject = p.req_line("subject", text::SHORT)?;
    }
    if p.present("question") {
        rfi.question = p.text_or_empty("question", text::LONG)?;
    }
    if p.present("manager") {
        let m = p.opt_id::<Person>("manager")?;
        if let Some(m) = m {
            ctx.check_person(m)?;
        }
        rfi.manager = m;
    }
    if p.present("assignees") {
        let a = p.ids("assignees")?;
        ctx.check_people(&a)?;
        rfi.assignees = a;
    }
    if p.present("received_from") {
        let r = p.opt_id::<Person>("received_from")?;
        if let Some(r) = r {
            ctx.check_person(r)?;
        }
        rfi.received_from = r;
    }
    if p.present("responsible_vendor") {
        let v = p.opt_id::<Vendor>("responsible_vendor")?;
        if let Some(v) = v {
            ctx.check_vendor(v)?;
        }
        rfi.responsible_vendor = v;
    }
    if p.present("distribution") {
        let d = p.ids("distribution")?;
        ctx.check_people(&d)?;
        rfi.distribution = d;
    }
    if p.present("due") {
        rfi.due = p.date("due")?;
    }
    for (key, field) in [("spec_section", &mut rfi.spec_section), ("drawing_number", &mut rfi.drawing_number), ("location", &mut rfi.location)] {
        if p.present(key) {
            *field = p.line_or_empty(key, text::SHORT)?;
        }
    }
    if let Some(i) = p.choice::<Impact>("cost_impact")? {
        rfi.cost_impact = i;
    }
    if p.present("cost_amount") {
        rfi.cost_amount = p.money("cost_amount")?;
    }
    if let Some(i) = p.choice::<Impact>("schedule_impact")? {
        rfi.schedule_impact = i;
    }
    if p.present("schedule_days") {
        rfi.schedule_days = p.int("schedule_days", -3650..=3650)?.and_then(|d| i32::try_from(d).ok());
    }
    if let Some(private) = p.flag("private")? {
        rfi.private = private;
    }
    rfi.recompute_ball_in_court();
    Ok(())
}

/// Moves a draft to open: checks it's complete, sets the opened stamp, due date and ball in court.
fn do_open(ctx: &Ctx<'_>, p: &Params<'_>, rfi: &mut Rfi) -> Result<()> {
    if rfi.status != RfiStatus::Draft {
        return Err(EngineError::conflict(format!("{} is already {}", rfi.label(), rfi.status)));
    }
    if rfi.question.is_empty() {
        return Err(EngineError::conflict("write the question before opening the RFI"));
    }
    if rfi.assignees.is_empty() {
        return Err(EngineError::conflict("assign someone to answer before opening the RFI"));
    }
    if rfi.manager.is_none() {
        rfi.manager = ctx.person();
    }
    if let Some(d) = p.date("due")? {
        rfi.due = Some(d);
    }
    if rfi.due.is_none() {
        rfi.due = default_due(ctx, rfi.project);
    }
    rfi.status = RfiStatus::Open;
    rfi.opened = Some(ctx.stamp());
    rfi.recompute_ball_in_court();
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.rfis.iter().filter(|r| r.project == project).map(|r| r.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut rfi = Rfi {
        id: ctx.state.rfis.peek_next(),
        project,
        number,
        subject: String::new(),
        question: String::new(),
        status: RfiStatus::Draft,
        manager: ctx.person(),
        assignees: Vec::new(),
        ball_in_court: Vec::new(),
        received_from: None,
        responsible_vendor: None,
        distribution: Vec::new(),
        due: None,
        opened: None,
        closed: None,
        spec_section: String::new(),
        drawing_number: String::new(),
        location: String::new(),
        cost_impact: Impact::default(),
        cost_amount: None,
        schedule_impact: Impact::default(),
        schedule_days: None,
        private: false,
        attachments: Vec::new(),
        responses: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(ctx, p, &mut rfi, true)?;
    if p.flag_or("open", false)? {
        if !is_manager(ctx, &rfi) {
            return Err(EngineError::forbidden("only the RFI manager or an RFI admin can open an RFI"));
        }
        do_open(ctx, p, &mut rfi)?;
    }
    let id = ctx.state.rfis.alloc();
    rfi.id = id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &rfi) {
        return Err(EngineError::forbidden(match rfi.status {
            RfiStatus::Closed => format!("{} is closed; reopen it to edit", rfi.label()),
            _ => format!("only the RFI manager or an RFI admin can edit {}", rfi.label()),
        }));
    }
    let before = rfi.clone();
    apply_fields(ctx, p, &mut rfi, false)?;
    let id = rfi.id;
    if rfi != before {
        rfi.updated = ctx.stamp();
        ctx.state.rfis.put(rfi);
    }
    Ok(json!({ "id": id }))
}

fn open(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !is_manager(ctx, &rfi) && rfi.created.by != ctx.person() {
        return Err(EngineError::forbidden("only the RFI's author, its manager or an RFI admin can open it"));
    }
    do_open(ctx, p, &mut rfi)?;
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

fn respond(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if rfi.status != RfiStatus::Open {
        return Err(EngineError::conflict(format!("{} is {}; only open RFIs take responses", rfi.label(), rfi.status)));
    }
    let me = ctx.person();
    let assignee = me.is_some_and(|m| rfi.assignees.contains(&m));
    if !assignee && !is_manager(ctx, &rfi) {
        return Err(EngineError::forbidden(format!("only assignees, the RFI manager and RFI admins can respond to {}", rfi.label())));
    }
    let official = p.flag_or("official", false)?;
    if official && !is_manager(ctx, &rfi) {
        return Err(EngineError::forbidden("only the RFI manager or an RFI admin can give the official response"));
    }
    if rfi.responses.len() >= 500 {
        return Err(EngineError::conflict("this RFI has too many responses"));
    }
    if official {
        for r in &mut rfi.responses {
            r.official = false;
        }
    }
    let rid = rfi.responses.iter().map(|r| r.id).max().unwrap_or(0).saturating_add(1);
    rfi.responses.push(RfiResponse { id: rid, by: me, at: ctx.at, body: p.req_text("body", text::LONG)?, official, attachments: Vec::new() });
    rfi.recompute_ball_in_court();
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id, "response": rid }))
}

fn mark_official(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !is_manager(ctx, &rfi) {
        return Err(EngineError::forbidden("only the RFI manager or an RFI admin can choose the official response"));
    }
    let rid = p.int("response", 1..=100_000)?.ok_or_else(|| p.bad("response", "is required"))?;
    let official = p.flag_or("official", true)?;
    if !rfi.responses.iter().any(|r| i64::from(r.id) == rid) {
        return Err(EngineError::not_found(format!("response {rid} not found on {}", rfi.label())));
    }
    for r in &mut rfi.responses {
        r.official = official && i64::from(r.id) == rid;
    }
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

/// Most files one RFI can carry.
const MAX_ATTACHMENTS: usize = 50;

/// Who may add or remove files: whoever may edit the RFI, and assignees while it is open.
fn can_attach(ctx: &Ctx<'_>, rfi: &Rfi) -> bool {
    can_edit(ctx.state, ctx.actor, rfi) || (rfi.status == RfiStatus::Open && ctx.person().is_some_and(|m| rfi.assignees.contains(&m)))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !can_attach(ctx, &rfi) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", rfi.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if rfi.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": rfi.id }));
    }
    if rfi.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", rfi.label())));
    }
    rfi.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !can_attach(ctx, &rfi) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", rfi.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = rfi.attachments.len();
    rfi.attachments.retain(|a| a.sha256 != sha256);
    if rfi.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this RFI"));
    }
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

fn close(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !is_manager(ctx, &rfi) {
        return Err(EngineError::forbidden("only the RFI manager or an RFI admin can close an RFI"));
    }
    if rfi.status != RfiStatus::Open {
        return Err(EngineError::conflict(format!("{} is {}; only open RFIs can be closed", rfi.label(), rfi.status)));
    }
    rfi.status = RfiStatus::Closed;
    rfi.closed = Some(ctx.stamp());
    rfi.recompute_ball_in_court();
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, mut rfi) = load(ctx, p)?;
    if !is_manager(ctx, &rfi) {
        return Err(EngineError::forbidden("only the RFI manager or an RFI admin can reopen an RFI"));
    }
    if rfi.status != RfiStatus::Closed {
        return Err(EngineError::conflict(format!("{} is {}; only closed RFIs can be reopened", rfi.label(), rfi.status)));
    }
    rfi.status = RfiStatus::Open;
    rfi.closed = None;
    rfi.recompute_ball_in_court();
    rfi.updated = ctx.stamp();
    let id = rfi.id;
    ctx.state.rfis.put(rfi);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (_, rfi) = load(ctx, p)?;
    let own_draft = rfi.status == RfiStatus::Draft && rfi.created.by.is_some() && rfi.created.by == ctx.person();
    if !is_admin(ctx, &rfi) && !own_draft {
        return Err(EngineError::forbidden("only RFI admins can delete RFIs (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    ctx.state.rfis.update(rfi.id, |r| r.deleted = Some(stamp));
    Ok(json!({ "id": rfi.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Rfi> = p.id("id")?;
    let rfi = in_project(&ctx.state.rfis, project, id, "RFI")?;
    if rfi.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", rfi.label())));
    }
    ctx.state.rfis.update(id, |r| r.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<RfiStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    let mine = p.flag_or("mine", false)?;
    let overdue = p.flag_or("overdue", false)?;
    let today = v.today();
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Rfis) < Level::Admin {
            return Err(EngineError::forbidden("only RFI admins can see the recycle bin"));
        }
        let bin: Vec<&Rfi> = v.state.rfis.iter().filter(|r| r.project == project && r.deleted.is_some()).collect();
        return json(&bin);
    }
    let me = v.person();
    let list: Vec<&Rfi> = view::rfis_for(v.state, v.actor, project)
        .into_iter()
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| !mine || me.is_some_and(|m| r.ball_in_court.contains(&m)))
        .filter(|r| !overdue || r.is_overdue(today))
        .filter(|r| {
            q.as_ref().is_none_or(|q| r.subject.to_lowercase().contains(q) || r.question.to_lowercase().contains(q) || r.number.to_string() == *q)
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Rfi> = p.id("id")?;
    let rfi = in_project(&v.state.rfis, project, id, "RFI")?;
    if !view::can_see_rfi(v.state, v.actor, rfi) {
        return Err(EngineError::not_found(format!("RFI {id} not found in this project")));
    }
    json(rfi)
}
