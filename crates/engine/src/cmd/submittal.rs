//! Submittals: documents sent for review before the work they cover goes ahead.
//!
//! Workflow: a Standard user drafts a submittal → its manager (or a submittals admin) sends it for
//! review, which sets the due date and the ball in court → each reviewer answers with a decision →
//! when everyone has answered the ball returns to the manager, who closes it with the final
//! decision, or resubmits it as a new revision for another round.
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - edit: the author or manager while it's a draft, the manager while it's in review, admins always;
//! - send for review, close, resubmit: the manager or a submittals admin;
//! - review: the listed reviewers (once per revision) and admins;
//! - delete: admins, or the author of a draft; restore: admins.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::submittal::{Decision, Submittal, SubmittalReview, SubmittalStatus};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "submittal.create",
        "Create submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project: project id; then the submittal fields: title: text; description?: text; spec_section?, kind?: text; manager?: person id (default: you); \
         submitter?: person id; reviewers?: [person ids]; distribution?: [person ids]; due?: YYYY-MM-DD; private?: bool; open?: bool (send for review now)",
        create,
    ),
    CommandSpec::mutate(
        "submittal.update",
        "Edit submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; then any submittal field",
        update,
    ),
    CommandSpec::mutate(
        "submittal.open",
        "Send submittal for review",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; due?: YYYY-MM-DD (default: today plus the company's review days)",
        open,
    ),
    CommandSpec::mutate(
        "submittal.review",
        "Review submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; decision: approved|approved_as_noted|revise_resubmit|rejected; comment?: text",
        review,
    ),
    CommandSpec::mutate(
        "submittal.close",
        "Close submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; decision: approved|approved_as_noted|revise_resubmit|rejected (the final answer)",
        close,
    ),
    CommandSpec::mutate(
        "submittal.resubmit",
        "Resubmit submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; due?: YYYY-MM-DD (starts a new revision and sends it for review again)",
        resubmit,
    ),
    CommandSpec::mutate(
        "submittal.attach",
        "Attach file to submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text (of an uploaded file); name: text; size: integer (bytes); mime?: text",
        attach,
    ),
    CommandSpec::mutate(
        "submittal.detach",
        "Remove attachment from submittal",
        Tool::Submittals,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text",
        detach,
    ),
    CommandSpec::mutate("submittal.delete", "Delete submittal", Tool::Submittals, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("submittal.restore", "Restore submittal", Tool::Submittals, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "submittal.list",
        "List submittals",
        Tool::Submittals,
        Access::Project(Level::Read),
        "project: project id; status?: draft|in_review|closed; q?: text; mine?: bool (ball in court is me); overdue?: bool; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("submittal.get", "Show submittal", Tool::Submittals, Access::Project(Level::Read), "project, id: ids", get),
];

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Submittal> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Submittal> = p.id("id")?;
    let s = in_project(&ctx.state.submittals, project, id, "submittal")?;
    if !view::can_see_submittal(ctx.state, ctx.actor, s) {
        return Err(EngineError::not_found(format!("submittal {id} not found in this project")));
    }
    Ok(s.clone())
}

fn is_admin(ctx: &Ctx<'_>, s: &Submittal) -> bool {
    ctx.level(s.project, Tool::Submittals) == Level::Admin
}

fn is_manager(ctx: &Ctx<'_>, s: &Submittal) -> bool {
    is_admin(ctx, s) || (ctx.person().is_some() && s.manager == ctx.person())
}

/// Whether the actor may edit the submittal's fields now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, s: &Submittal) -> bool {
    let lvl = view::level(state, actor, s.project, Tool::Submittals);
    if lvl == Level::Admin {
        return true;
    }
    let me = actor.person();
    if lvl < Level::Standard || me.is_none() {
        return false;
    }
    match s.status {
        SubmittalStatus::Draft => s.created.by == me || s.manager == me,
        SubmittalStatus::InReview => s.manager == me,
        SubmittalStatus::Closed => false,
    }
}

fn default_due(ctx: &Ctx<'_>) -> Option<Date> {
    let days = ctx.state.company().map(|c| c.submittal_review_days).unwrap_or(10);
    ctx.today().add_business_days(days)
}

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, s: &mut Submittal, creating: bool) -> Result<()> {
    if creating || p.present("title") {
        s.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        s.description = p.text_or_empty("description", text::LONG)?;
    }
    for (key, field) in [("spec_section", &mut s.spec_section), ("kind", &mut s.kind)] {
        if p.present(key) {
            *field = p.line_or_empty(key, text::SHORT)?;
        }
    }
    for (key, field) in [("manager", &mut s.manager), ("submitter", &mut s.submitter)] {
        if p.present(key) {
            let v = p.opt_id::<Person>(key)?;
            if let Some(v) = v {
                ctx.check_person(v)?;
            }
            *field = v;
        }
    }
    for (key, field) in [("reviewers", &mut s.reviewers), ("distribution", &mut s.distribution)] {
        if p.present(key) {
            let v = p.ids(key)?;
            ctx.check_people(&v)?;
            *field = v;
        }
    }
    if p.present("due") {
        s.due = p.date("due")?;
    }
    if let Some(private) = p.flag("private")? {
        s.private = private;
    }
    s.recompute_ball_in_court();
    Ok(())
}

fn do_open(ctx: &Ctx<'_>, p: &Params<'_>, s: &mut Submittal) -> Result<()> {
    if s.status != SubmittalStatus::Draft {
        return Err(EngineError::conflict(format!("{} is already {}", s.label(), s.status)));
    }
    if s.reviewers.is_empty() {
        return Err(EngineError::conflict("choose at least one reviewer before sending the submittal for review"));
    }
    if s.manager.is_none() {
        s.manager = ctx.person();
    }
    if let Some(d) = p.date("due")? {
        s.due = Some(d);
    }
    if s.due.is_none() {
        s.due = default_due(ctx);
    }
    s.status = SubmittalStatus::InReview;
    s.opened = Some(ctx.stamp());
    s.recompute_ball_in_court();
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.submittals.iter().filter(|s| s.project == project).map(|s| s.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut s = Submittal {
        id: ctx.state.submittals.peek_next(),
        project,
        number,
        revision: 0,
        title: String::new(),
        description: String::new(),
        spec_section: String::new(),
        kind: String::new(),
        status: SubmittalStatus::Draft,
        manager: ctx.person(),
        submitter: None,
        reviewers: Vec::new(),
        distribution: Vec::new(),
        ball_in_court: Vec::new(),
        due: None,
        opened: None,
        closed: None,
        final_decision: None,
        private: false,
        attachments: Vec::new(),
        reviews: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(ctx, p, &mut s, true)?;
    if p.flag_or("open", false)? {
        if !is_manager(ctx, &s) {
            return Err(EngineError::forbidden("only the submittal manager or a submittals admin can send a submittal for review"));
        }
        do_open(ctx, p, &mut s)?;
    }
    let id = ctx.state.submittals.alloc();
    s.id = id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &s) {
        return Err(EngineError::forbidden(match s.status {
            SubmittalStatus::Closed => format!("{} is closed; resubmit it to change it", s.label()),
            _ => format!("only the submittal manager or a submittals admin can edit {}", s.label()),
        }));
    }
    let before = s.clone();
    apply_fields(ctx, p, &mut s, false)?;
    let id = s.id;
    if s != before {
        s.updated = ctx.stamp();
        ctx.state.submittals.put(s);
    }
    Ok(json!({ "id": id }))
}

fn open(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !is_manager(ctx, &s) && s.created.by != ctx.person() {
        return Err(EngineError::forbidden("only the submittal's author, its manager or a submittals admin can send it for review"));
    }
    do_open(ctx, p, &mut s)?;
    s.updated = ctx.stamp();
    let id = s.id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id }))
}

fn review(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if s.status != SubmittalStatus::InReview {
        return Err(EngineError::conflict(format!("{} is {}; only submittals in review take reviews", s.label(), s.status)));
    }
    let me = ctx.person();
    let reviewer = me.is_some_and(|m| s.reviewers.contains(&m));
    if !reviewer && !is_admin(ctx, &s) {
        return Err(EngineError::forbidden(format!("only the reviewers of {} can review it", s.label())));
    }
    if me.is_some_and(|m| s.has_reviewed(m)) {
        return Err(EngineError::conflict("you have already reviewed this revision"));
    }
    let decision = p.choice::<Decision>("decision")?.ok_or_else(|| p.bad("decision", "is required"))?;
    let id = u32::try_from(s.reviews.len()).unwrap_or(u32::MAX).saturating_add(1);
    s.reviews.push(SubmittalReview { id, revision: s.revision, by: me, at: ctx.at, decision, comment: p.text_or_empty("comment", text::LONG)? });
    s.recompute_ball_in_court();
    s.updated = ctx.stamp();
    let sid = s.id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": sid, "review": id }))
}

fn close(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !is_manager(ctx, &s) {
        return Err(EngineError::forbidden("only the submittal manager or a submittals admin can close a submittal"));
    }
    if s.status != SubmittalStatus::InReview {
        return Err(EngineError::conflict(format!("{} is {}; only submittals in review can be closed", s.label(), s.status)));
    }
    s.final_decision = Some(p.choice::<Decision>("decision")?.ok_or_else(|| p.bad("decision", "is required"))?);
    s.status = SubmittalStatus::Closed;
    s.closed = Some(ctx.stamp());
    s.recompute_ball_in_court();
    s.updated = ctx.stamp();
    let id = s.id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id }))
}

fn resubmit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !is_manager(ctx, &s) {
        return Err(EngineError::forbidden("only the submittal manager or a submittals admin can resubmit a submittal"));
    }
    if s.status == SubmittalStatus::Draft {
        return Err(EngineError::conflict(format!("{} has not been sent for review yet", s.label())));
    }
    s.revision = s.revision.saturating_add(1);
    s.status = SubmittalStatus::InReview;
    s.final_decision = None;
    s.closed = None;
    s.due = match p.date("due")? {
        Some(d) => Some(d),
        None => default_due(ctx),
    };
    s.recompute_ball_in_court();
    s.updated = ctx.stamp();
    let id = s.id;
    let revision = s.revision;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id, "revision": revision }))
}

const MAX_ATTACHMENTS: usize = 50;

/// Who may add or remove files: whoever may edit, and reviewers while it is in review.
fn can_attach(ctx: &Ctx<'_>, s: &Submittal) -> bool {
    can_edit(ctx.state, ctx.actor, s) || (s.status == SubmittalStatus::InReview && ctx.person().is_some_and(|m| s.reviewers.contains(&m)))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !can_attach(ctx, &s) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", s.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if s.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": s.id }));
    }
    if s.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", s.label())));
    }
    s.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    s.updated = ctx.stamp();
    let id = s.id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut s = load(ctx, p)?;
    if !can_attach(ctx, &s) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", s.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = s.attachments.len();
    s.attachments.retain(|a| a.sha256 != sha256);
    if s.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this submittal"));
    }
    s.updated = ctx.stamp();
    let id = s.id;
    ctx.state.submittals.put(s);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let s = load(ctx, p)?;
    let own_draft = s.status == SubmittalStatus::Draft && s.created.by.is_some() && s.created.by == ctx.person();
    if !is_admin(ctx, &s) && !own_draft {
        return Err(EngineError::forbidden("only submittals admins can delete submittals (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    ctx.state.submittals.update(s.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": s.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Submittal> = p.id("id")?;
    let s = in_project(&ctx.state.submittals, project, id, "submittal")?;
    if s.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", s.label())));
    }
    ctx.state.submittals.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<SubmittalStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    let mine = p.flag_or("mine", false)?;
    let overdue = p.flag_or("overdue", false)?;
    let today = v.today();
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Submittals) < Level::Admin {
            return Err(EngineError::forbidden("only submittals admins can see the recycle bin"));
        }
        let bin: Vec<&Submittal> = v.state.submittals.iter().filter(|s| s.project == project && s.deleted.is_some()).collect();
        return json(&bin);
    }
    let me = v.person();
    let list: Vec<&Submittal> = view::submittals_for(v.state, v.actor, project)
        .into_iter()
        .filter(|s| status.is_none_or(|x| s.status == x))
        .filter(|s| !mine || me.is_some_and(|m| s.ball_in_court.contains(&m)))
        .filter(|s| !overdue || s.is_overdue(today))
        .filter(|s| {
            q.as_ref().is_none_or(|q| s.title.to_lowercase().contains(q) || s.spec_section.to_lowercase().contains(q) || s.number.to_string() == *q)
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Submittal> = p.id("id")?;
    let s = in_project(&v.state.submittals, project, id, "submittal")?;
    if !view::can_see_submittal(v.state, v.actor, s) {
        return Err(EngineError::not_found(format!("submittal {id} not found in this project")));
    }
    json(s)
}
