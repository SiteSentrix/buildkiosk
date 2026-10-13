//! Commitments: subcontracts and purchase orders, each with a schedule of values by cost code.
//!
//! A commitment starts as a draft that its author edits; a commitments admin approves it, which
//! makes its amounts count as committed cost in the budget. Approved commitments can be marked
//! complete, and any commitment can be voided (its amounts stop counting).
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - edit: the author while it's a draft, admins always (until void);
//! - approve, complete, void, restore: commitments admins;
//! - delete: admins, or the author of a draft.

use buildkiosk_core::{Id, text};
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::finance::{Commitment, CommitmentKind, CommitmentLine, CommitmentStatus, InvoiceStatus};
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::usable_code;
use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "commitment.create",
        "Create commitment",
        Tool::Commitments,
        Access::Project(Level::Standard),
        "project: project id; title: text; kind?: subcontract|purchase_order; vendor?: company id; description?: text; retainage?: percent (such as 10, withheld on each invoice); \
         lines?: [{cost_code: cost code id, description?: text, amount: amount}]",
        create,
    ),
    CommandSpec::mutate(
        "commitment.update",
        "Edit commitment",
        Tool::Commitments,
        Access::Project(Level::Standard),
        "project, id: ids; then any commitment.create field (a lines list replaces the old one)",
        update,
    ),
    CommandSpec::mutate(
        "commitment.approve",
        "Approve commitment",
        Tool::Commitments,
        Access::Project(Level::Admin),
        "project, id: ids (the commitment needs at least one line)",
        approve,
    ),
    CommandSpec::mutate("commitment.complete", "Complete commitment", Tool::Commitments, Access::Project(Level::Admin), "project, id: ids", complete),
    CommandSpec::mutate("commitment.void", "Void commitment", Tool::Commitments, Access::Project(Level::Admin), "project, id: ids", void),
    CommandSpec::mutate("commitment.delete", "Delete commitment", Tool::Commitments, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("commitment.restore", "Restore commitment", Tool::Commitments, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "commitment.list",
        "List commitments",
        Tool::Commitments,
        Access::Project(Level::Read),
        "project: project id; status?: draft|approved|complete|void; q?: text; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("commitment.get", "Show commitment", Tool::Commitments, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_LINES: usize = 200;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Commitment> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Commitment> = p.id("id")?;
    let c = in_project(&ctx.state.commitments, project, id, "commitment")?;
    if !view::can_see_commitment(ctx.state, ctx.actor, c) {
        return Err(EngineError::not_found(format!("commitment {id} not found in this project")));
    }
    Ok(c.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Commitments) == Level::Admin
}

/// Whether the actor may edit the commitment now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, c: &Commitment) -> bool {
    let lvl = view::level(state, actor, c.project, Tool::Commitments);
    if c.status == CommitmentStatus::Void {
        return false;
    }
    lvl == Level::Admin
        || (lvl >= Level::Standard && c.status == CommitmentStatus::Draft && actor.person().is_some() && c.created.by == actor.person())
}

fn read_lines(ctx: &Ctx<'_>, p: &Params<'_>, project: Id<Project>) -> Result<Vec<CommitmentLine>> {
    let rows = p.items("lines")?;
    if rows.len() > MAX_LINES {
        return Err(p.bad("lines", format!("holds more than {MAX_LINES} lines")));
    }
    let mut out = Vec::new();
    for row in rows {
        // Rows left blank on a form are skipped.
        let code: Option<Id<CostCode>> = row.opt_id("cost_code")?;
        if code.is_none() && row.line_or_empty("amount", 40)?.is_empty() && row.line_or_empty("description", text::SHORT)?.is_empty() {
            continue;
        }
        let code = code.ok_or_else(|| row.bad("cost_code", "is required"))?;
        usable_code(ctx.state, project, code)?;
        let amount = row.money("amount")?.ok_or_else(|| row.bad("amount", "is required"))?;
        out.push(CommitmentLine { cost_code: code, description: row.line_or_empty("description", text::SHORT)?, amount });
    }
    Ok(out)
}

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, c: &mut Commitment, creating: bool) -> Result<()> {
    if creating || p.present("title") {
        c.title = p.req_line("title", text::SHORT)?;
    }
    if let Some(k) = p.choice::<CommitmentKind>("kind")? {
        c.kind = k;
    }
    if p.present("vendor") {
        let v = p.opt_id::<Vendor>("vendor")?;
        if let Some(v) = v {
            ctx.check_vendor(v)?;
        }
        c.vendor = v;
    }
    if p.present("description") {
        c.description = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("retainage") {
        c.retainage = super::invoice::retainage(p)?;
    }
    if p.present("lines") {
        if !creating && ctx.state.commitment_invoices.iter().any(|i| i.commitment == c.id && i.deleted.is_none() && i.status != InvoiceStatus::Void) {
            return Err(EngineError::conflict("invoices exist for this commitment, so its lines can't change (use a change order)"));
        }
        c.lines = read_lines(ctx, p, c.project)?;
        if c.total().is_none() {
            return Err(p.bad("lines", "the amounts are too large to add up"));
        }
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.commitments.iter().filter(|c| c.project == project).map(|c| c.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut c = Commitment {
        id: ctx.state.commitments.peek_next(),
        project,
        number,
        title: String::new(),
        kind: CommitmentKind::default(),
        status: CommitmentStatus::Draft,
        vendor: None,
        description: String::new(),
        lines: Vec::new(),
        retainage: None,
        approved: None,
        attachments: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(ctx, p, &mut c, true)?;
    let id = ctx.state.commitments.alloc();
    c.id = id;
    ctx.state.commitments.put(c);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut c = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &c) {
        return Err(EngineError::forbidden(match c.status {
            CommitmentStatus::Void => format!("{} is void", c.label()),
            _ => format!("only the author (while it is a draft) or a commitments admin can edit {}", c.label()),
        }));
    }
    let before = c.clone();
    apply_fields(ctx, p, &mut c, false)?;
    let id = c.id;
    if c != before {
        c.updated = ctx.stamp();
        ctx.state.commitments.put(c);
    }
    Ok(json!({ "id": id }))
}

fn set_status(ctx: &mut Ctx<'_>, mut c: Commitment, status: CommitmentStatus) -> Value {
    c.status = status;
    if status == CommitmentStatus::Approved && c.approved.is_none() {
        c.approved = Some(ctx.stamp());
    }
    c.updated = ctx.stamp();
    let id = c.id;
    ctx.state.commitments.put(c);
    json!({ "id": id })
}

fn approve(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c = load(ctx, p)?;
    if c.status != CommitmentStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}; only drafts can be approved", c.label(), c.status)));
    }
    if c.lines.is_empty() {
        return Err(EngineError::conflict(format!("add at least one line to {} before approving it", c.label())));
    }
    Ok(set_status(ctx, c, CommitmentStatus::Approved))
}

fn complete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c = load(ctx, p)?;
    if c.status != CommitmentStatus::Approved {
        return Err(EngineError::conflict(format!("{} is {}; only approved commitments can be completed", c.label(), c.status)));
    }
    Ok(set_status(ctx, c, CommitmentStatus::Complete))
}

fn void(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c = load(ctx, p)?;
    if c.status == CommitmentStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", c.label())));
    }
    if ctx.state.commitment_invoices.iter().any(|i| i.commitment == c.id && i.deleted.is_none() && i.status != InvoiceStatus::Void) {
        return Err(EngineError::conflict("invoices exist for this commitment; void them first"));
    }
    Ok(set_status(ctx, c, CommitmentStatus::Void))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c = load(ctx, p)?;
    let own_draft = c.status == CommitmentStatus::Draft && c.created.by.is_some() && c.created.by == ctx.person();
    if !is_admin(ctx, c.project) && !own_draft {
        return Err(EngineError::forbidden("only commitments admins can delete commitments (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    ctx.state.commitments.update(c.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": c.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Commitment> = p.id("id")?;
    let c = in_project(&ctx.state.commitments, project, id, "commitment")?;
    if c.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", c.label())));
    }
    ctx.state.commitments.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn with_total(c: &Commitment) -> Value {
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    if let (Some(map), Some(total)) = (v.as_object_mut(), c.total()) {
        map.insert("total".into(), json!(total));
    }
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<CommitmentStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Commitments) < Level::Admin {
            return Err(EngineError::forbidden("only commitments admins can see the recycle bin"));
        }
        let bin: Vec<Value> = v.state.commitments.iter().filter(|c| c.project == project && c.deleted.is_some()).map(with_total).collect();
        return json(&bin);
    }
    let list: Vec<Value> = view::commitments_for(v.state, v.actor, project)
        .into_iter()
        .filter(|c| status.is_none_or(|s| c.status == s))
        .filter(|c| q.as_ref().is_none_or(|q| c.title.to_lowercase().contains(q) || c.number.to_string() == *q))
        .map(with_total)
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Commitment> = p.id("id")?;
    let c = in_project(&v.state.commitments, project, id, "commitment")?;
    if !view::can_see_commitment(v.state, v.actor, c) {
        return Err(EngineError::not_found(format!("commitment {id} not found in this project")));
    }
    Ok(with_total(c))
}
