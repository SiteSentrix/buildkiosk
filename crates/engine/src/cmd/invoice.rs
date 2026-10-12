//! Owner invoices: payment applications that bill the prime contract's schedule of values.
//!
//! Each invoice enters the work completed this period on the contract's billable rows. The engine
//! works out the rest (billed before, to date, balance to finish, retainage withheld, the amount
//! due; see [`crate::view::invoice_summary`]) and refuses to bill a row beyond its scheduled value.
//!
//! Flow: draft (its author edits) → submitted to the owner (an admin can send it back to draft) →
//! approved by an admin → paid (an admin records the date). An admin can void any invoice that
//! is not paid. Only submitted, approved and paid invoices count as billed.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::finance::{ChangeStatus, InvoiceLine, InvoiceStatus, OwnerInvoice, PrimeChange, PrimeContract};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "owner_invoice.create",
        "Start owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Standard),
        "project: project id; contract: prime contract id (must be approved); period_end?: YYYY-MM-DD (default: today); notes?: text; \
         lines?: [{row: row key like c.0 (contract line 0) or p.12.0 (line 0 of change order 12), amount: work completed this period}]",
        create,
    ),
    CommandSpec::mutate(
        "owner_invoice.update",
        "Edit owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Standard),
        "project, id: ids; period_end?, notes?, lines? (a lines list replaces the old one)",
        update,
    ),
    CommandSpec::mutate(
        "owner_invoice.submit",
        "Submit owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Standard),
        "project, id: ids",
        submit,
    ),
    CommandSpec::mutate(
        "owner_invoice.reopen",
        "Send owner invoice back to draft",
        Tool::Invoicing,
        Access::Project(Level::Admin),
        "project, id: ids (a submitted invoice only)",
        reopen,
    ),
    CommandSpec::mutate(
        "owner_invoice.approve",
        "Approve owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Admin),
        "project, id: ids",
        approve,
    ),
    CommandSpec::mutate(
        "owner_invoice.pay",
        "Record owner payment",
        Tool::Invoicing,
        Access::Project(Level::Admin),
        "project, id: ids; paid_on?: YYYY-MM-DD (default: today)",
        pay,
    ),
    CommandSpec::mutate("owner_invoice.void", "Void owner invoice", Tool::Invoicing, Access::Project(Level::Admin), "project, id: ids", void),
    CommandSpec::mutate(
        "owner_invoice.delete",
        "Delete owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Standard),
        "project, id: ids (drafts only)",
        delete,
    ),
    CommandSpec::mutate(
        "owner_invoice.restore",
        "Restore owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Admin),
        "project, id: ids",
        restore,
    ),
    CommandSpec::query(
        "owner_invoice.list",
        "List owner invoices",
        Tool::Invoicing,
        Access::Project(Level::Read),
        "project: project id; contract?: prime contract id; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query(
        "owner_invoice.get",
        "Show owner invoice",
        Tool::Invoicing,
        Access::Project(Level::Read),
        "project, id: ids (with the worked-out rows and totals)",
        get,
    ),
];

fn label(i: &OwnerInvoice) -> String {
    format!("Invoice #{}", i.number)
}

/// Drafts are private to their author and admins; submitted ones are visible to everyone with Read.
pub fn can_see(state: &buildkiosk_model::CompanyState, actor: crate::Actor, i: &OwnerInvoice) -> bool {
    let lvl = view::level(state, actor, i.project, Tool::Invoicing);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || i.deleted.is_some() {
        return false;
    }
    i.status != InvoiceStatus::Draft || (actor.person().is_some() && i.created.by == actor.person())
}

/// The invoices of a project the actor may see, in number order.
pub fn invoices_for(state: &buildkiosk_model::CompanyState, actor: crate::Actor, project: Id<Project>) -> Vec<&OwnerInvoice> {
    let mut list: Vec<&OwnerInvoice> =
        state.owner_invoices.iter().filter(|i| i.project == project && i.deleted.is_none() && can_see(state, actor, i)).collect();
    list.sort_by_key(|i| i.number);
    list
}

/// Whether the actor may edit the invoice now.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, i: &OwnerInvoice) -> bool {
    let lvl = view::level(state, actor, i.project, Tool::Invoicing);
    if matches!(i.status, InvoiceStatus::Void | InvoiceStatus::Paid) {
        return false;
    }
    lvl == Level::Admin && i.status != InvoiceStatus::Approved
        || (lvl >= Level::Standard && i.status == InvoiceStatus::Draft && actor.person().is_some() && i.created.by == actor.person())
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<OwnerInvoice> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<OwnerInvoice> = p.id("id")?;
    let i = in_project(&ctx.state.owner_invoices, project, id, "invoice")?;
    if !can_see(ctx.state, ctx.actor, i) {
        return Err(EngineError::not_found(format!("invoice {id} not found in this project")));
    }
    Ok(i.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Invoicing) == Level::Admin
}

/// Reads `lines` (row key and amount); blank amounts are skipped.
fn read_lines(p: &Params<'_>) -> Result<Vec<InvoiceLine>> {
    let rows = p.items("lines")?;
    if rows.len() > 1_000 {
        return Err(p.bad("lines", "holds more than 1,000 lines"));
    }
    let mut out: Vec<InvoiceLine> = Vec::new();
    for row in rows {
        let amount_text = row.line_or_empty("amount", 40)?;
        if amount_text.is_empty() {
            continue;
        }
        let amount = row.money("amount")?.unwrap_or(Money::ZERO);
        if amount.is_zero() {
            continue;
        }
        let key = row.req_line("row", 40)?;
        let parts: Vec<&str> = key.split('.').collect();
        let line = match parts.as_slice() {
            ["c", n] => InvoiceLine { change: None, line: n.parse().map_err(|_| row.bad("row", "is not a row key"))?, this_period: amount },
            ["p", c, n] => InvoiceLine {
                change: Some(Id::<PrimeChange>::parse(c).ok_or_else(|| row.bad("row", "is not a row key"))?),
                line: n.parse().map_err(|_| row.bad("row", "is not a row key"))?,
                this_period: amount,
            },
            _ => return Err(row.bad("row", format!("`{key}` is not a row key such as c.0 or p.12.0"))),
        };
        if out.iter().any(|l| l.change == line.change && l.line == line.line) {
            return Err(row.bad("row", format!("row {key} is listed twice")));
        }
        out.push(line);
    }
    Ok(out)
}

/// Checks the invoice's lines against the contract: every row exists, and no row is billed below
/// nothing or beyond its scheduled value.
fn check(ctx: &Ctx<'_>, p: &Params<'_>, inv: &OwnerInvoice) -> Result<()> {
    let summary = view::invoice_summary(ctx.state, inv)?;
    for l in &inv.lines {
        let Some(r) = summary.rows.iter().find(|r| r.row.change == l.change && r.row.line == l.line) else {
            return Err(p.bad("lines", "a row is not part of the contract (is its change order approved?)"));
        };
        let to_date = r.to_date();
        if to_date.is_negative()
            || (!r.row.scheduled.is_negative() && to_date.checked_sub(r.row.scheduled).is_some_and(|over| !over.is_negative() && !over.is_zero()))
        {
            return Err(p.bad(
                "lines",
                format!("{} would be billed {} to date against a scheduled value of {}", r.row.description, to_date, r.row.scheduled),
            ));
        }
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let contract_id: Id<PrimeContract> = p.id("contract")?;
    let contract = in_project(&ctx.state.prime_contracts, project, contract_id, "prime contract")?;
    if contract.deleted.is_some() || contract.status != ChangeStatus::Approved {
        return Err(EngineError::conflict("only an approved prime contract can be invoiced"));
    }
    let number = ctx.state.owner_invoices.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let inv = OwnerInvoice {
        id: ctx.state.owner_invoices.peek_next(),
        project,
        number,
        contract: contract_id,
        period_end: p.date("period_end")?.unwrap_or_else(|| ctx.today()),
        status: InvoiceStatus::Draft,
        notes: p.text_or_empty("notes", text::LONG)?,
        lines: read_lines(p)?,
        submitted: None,
        approved: None,
        paid_on: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    check(ctx, p, &inv)?;
    let id = ctx.state.owner_invoices.alloc();
    let mut inv = inv;
    inv.id = id;
    ctx.state.owner_invoices.put(inv);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("{} is {}; only drafts can be edited (by their author or an admin)", label(&i), i.status)));
    }
    let before = i.clone();
    if let Some(d) = p.date("period_end")? {
        i.period_end = d;
    }
    if p.present("notes") {
        i.notes = p.text_or_empty("notes", text::LONG)?;
    }
    if p.present("lines") {
        i.lines = read_lines(p)?;
    }
    check(ctx, p, &i)?;
    let id = i.id;
    if i != before {
        i.updated = ctx.stamp();
        ctx.state.owner_invoices.put(i);
    }
    Ok(json!({ "id": id }))
}

fn set_status(ctx: &mut Ctx<'_>, i: &OwnerInvoice, status: InvoiceStatus, apply: impl FnOnce(&mut OwnerInvoice)) -> Value {
    let stamp = ctx.stamp();
    ctx.state.owner_invoices.update(i.id, |x| {
        x.status = status;
        x.updated = stamp;
        apply(x);
    });
    json!({ "id": i.id })
}

fn submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("only the author or an admin can submit {}", label(&i))));
    }
    if i.status != InvoiceStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}", label(&i), i.status)));
    }
    if i.lines.is_empty() {
        return Err(EngineError::conflict(format!("enter some work completed on {} before submitting it", label(&i))));
    }
    let stamp = ctx.stamp();
    Ok(set_status(ctx, &i, InvoiceStatus::Submitted, |x| x.submitted = Some(stamp)))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    if i.status != InvoiceStatus::Submitted {
        return Err(EngineError::conflict(format!("{} is {}; only submitted invoices can go back to draft", label(&i), i.status)));
    }
    Ok(set_status(ctx, &i, InvoiceStatus::Draft, |x| x.submitted = None))
}

fn approve(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    if i.status != InvoiceStatus::Submitted {
        return Err(EngineError::conflict(format!("{} is {}; only submitted invoices can be approved", label(&i), i.status)));
    }
    let stamp = ctx.stamp();
    Ok(set_status(ctx, &i, InvoiceStatus::Approved, |x| x.approved = Some(stamp)))
}

fn pay(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    if i.status != InvoiceStatus::Approved {
        return Err(EngineError::conflict(format!("{} is {}; only approved invoices can be paid", label(&i), i.status)));
    }
    let on: Date = p.date("paid_on")?.unwrap_or_else(|| ctx.today());
    Ok(set_status(ctx, &i, InvoiceStatus::Paid, |x| x.paid_on = Some(on)))
}

fn void(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    if matches!(i.status, InvoiceStatus::Paid | InvoiceStatus::Void) {
        return Err(EngineError::conflict(format!("{} is {}; it can't be voided", label(&i), i.status)));
    }
    // Later invoices were worked out on top of this one; keep the numbering honest.
    let later = ctx.state.owner_invoices.iter().any(|o| o.contract == i.contract && o.deleted.is_none() && o.number > i.number && o.status.billed());
    if i.status.billed() && later {
        return Err(EngineError::conflict("later invoices build on this one; void them first"));
    }
    Ok(set_status(ctx, &i, InvoiceStatus::Void, |_| {}))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    let own_draft = i.status == InvoiceStatus::Draft && i.created.by.is_some() && i.created.by == ctx.person();
    if i.status != InvoiceStatus::Draft && i.status != InvoiceStatus::Void {
        return Err(EngineError::conflict(format!("{} is {}; void it instead", label(&i), i.status)));
    }
    if !is_admin(ctx, i.project) && !own_draft {
        return Err(EngineError::forbidden("only admins can delete invoices (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    ctx.state.owner_invoices.update(i.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": i.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<OwnerInvoice> = p.id("id")?;
    let i = in_project(&ctx.state.owner_invoices, project, id, "invoice")?;
    if i.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", label(i))));
    }
    ctx.state.owner_invoices.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Invoicing) < Level::Admin {
            return Err(EngineError::forbidden("only invoicing admins can see the recycle bin"));
        }
        let bin: Vec<&OwnerInvoice> = v.state.owner_invoices.iter().filter(|i| i.project == project && i.deleted.is_some()).collect();
        return json(&bin);
    }
    let contract = p.opt_id::<PrimeContract>("contract")?;
    let list: Vec<&OwnerInvoice> = invoices_for(v.state, v.actor, project).into_iter().filter(|i| contract.is_none_or(|c| i.contract == c)).collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<OwnerInvoice> = p.id("id")?;
    let i = in_project(&v.state.owner_invoices, project, id, "invoice")?;
    if !can_see(v.state, v.actor, i) {
        return Err(EngineError::not_found(format!("invoice {id} not found in this project")));
    }
    let s = view::invoice_summary(v.state, i)?;
    let rows: Vec<Value> = s
        .rows
        .iter()
        .map(|r| json!({"row": r.row.key(), "description": r.row.description, "cost_code": r.row.cost_code, "scheduled": r.row.scheduled, "previous": r.previous, "this_period": r.this_period, "to_date": r.to_date(), "balance": r.balance()}))
        .collect();
    Ok(json!({
        "invoice": i,
        "rows": rows,
        "totals": {"scheduled": s.scheduled, "previous": s.previous, "this_period": s.this_period, "to_date": s.to_date, "retainage_to_date": s.retainage_to_date, "retainage_this": s.retainage_this, "net_due": s.net_due},
    }))
}
