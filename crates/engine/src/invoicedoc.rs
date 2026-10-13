//! Invoices that bill a contract or commitment: owner invoices (against the prime contract) and
//! commitment invoices (from a subcontractor or supplier). Each enters the work completed this
//! period on the billable rows; the engine works out the rest (billed before, to date, balance to
//! finish, retainage, the amount due; see [`crate::view::InvoiceSummary`]) and refuses to bill a
//! row beyond its scheduled value or below nothing.
//!
//! Flow: draft (its author edits) → submitted (an admin can send it back to draft) → approved by
//! an admin → paid (an admin records the date). An admin can void any invoice that is not paid.
//! Only submitted, approved and paid invoices count as billed.
//!
//! A record opts in by implementing [`InvoiceDoc`]; its commands come from `invoice_commands!`.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::finance::{InvoiceLine, InvoiceStatus};
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, InProject, Level, Record, Stamp, Table, Tool, Trash};
use serde::Serialize;
use serde_json::{Value, json};

use crate::ctx::{Read, in_project};
use crate::view::{self, InvoiceSummary, json};
use crate::{Actor, Ctx, EngineError, Params, Result, View};

/// A record that runs the shared invoice commands.
pub trait InvoiceDoc: Record + InProject + Trash + Clone + PartialEq + Serialize + 'static {
    const TOOL: Tool;
    /// How messages name one: "invoice".
    const NOUN: &'static str = "invoice";
    /// The request parameter that names what is invoiced: `contract` or `commitment`.
    const PARENT: &'static str;

    fn table(state: &CompanyState) -> &Table<Self>;
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self>;
    /// A new draft against the parent with the given raw id.
    fn blank(ctx: &Ctx<'_>, project: Id<Project>, parent: u64) -> Self;
    /// Checks that the parent the request names can be invoiced, and returns its raw id.
    fn check_parent(ctx: &Ctx<'_>, project: Id<Project>, p: &Params<'_>) -> Result<u64>;
    fn parent_raw(&self) -> u64;
    /// Works the invoice out.
    fn summary(&self, state: &CompanyState) -> Result<InvoiceSummary>;
    fn set_id(&mut self, id: Id<Self>);
    fn number(&self) -> u32;
    fn status(&self) -> InvoiceStatus;
    fn set_status(&mut self, s: InvoiceStatus);
    fn lines(&self) -> &[InvoiceLine];
    fn set_lines(&mut self, lines: Vec<InvoiceLine>);
    fn set_period(&mut self, d: Date);
    fn set_notes(&mut self, n: String);
    fn set_submitted(&mut self, s: Option<Stamp>);
    fn set_approved(&mut self, s: Option<Stamp>);
    fn set_paid_on(&mut self, d: Option<Date>);
    fn created(&self) -> Stamp;
    fn set_updated(&mut self, s: Stamp);
}

/// "Invoice #3".
pub fn label<T: InvoiceDoc>(i: &T) -> String {
    format!("Invoice #{}", i.number())
}

/// Drafts are private to their author and admins; submitted ones are visible to everyone with Read.
pub fn can_see<T: InvoiceDoc>(state: &CompanyState, actor: Actor, i: &T) -> bool {
    let lvl = view::level(state, actor, i.project(), T::TOOL);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || i.deleted().is_some() {
        return false;
    }
    i.status() != InvoiceStatus::Draft || (actor.person().is_some() && i.created().by == actor.person())
}

/// The invoices of a project the actor may see, in number order.
pub fn invoices_for<T: InvoiceDoc>(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&T> {
    let mut list: Vec<&T> = T::table(state).iter().filter(|i| i.project() == project && i.deleted().is_none() && can_see(state, actor, *i)).collect();
    list.sort_by_key(|i| i.number());
    list
}

/// Whether the actor may edit the invoice now (a draft by its author, or an admin before approval).
pub fn can_edit<T: InvoiceDoc>(state: &CompanyState, actor: Actor, i: &T) -> bool {
    let lvl = view::level(state, actor, i.project(), T::TOOL);
    if matches!(i.status(), InvoiceStatus::Void | InvoiceStatus::Paid) {
        return false;
    }
    (lvl == Level::Admin && i.status() != InvoiceStatus::Approved)
        || (lvl >= Level::Standard && i.status() == InvoiceStatus::Draft && actor.person().is_some() && i.created().by == actor.person())
}

fn load<T: InvoiceDoc>(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<T> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let i = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if !can_see(ctx.state, ctx.actor, i) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    Ok(i.clone())
}

fn is_admin<T: InvoiceDoc>(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, T::TOOL) == Level::Admin
}

/// Reads `lines` (row key and amount); blank amounts are skipped.
fn read_lines(p: &Params<'_>) -> Result<Vec<InvoiceLine>> {
    let rows = p.items("lines")?;
    if rows.len() > 1_000 {
        return Err(p.bad("lines", "holds more than 1,000 lines"));
    }
    let mut out: Vec<InvoiceLine> = Vec::new();
    for row in rows {
        if row.line_or_empty("amount", 40)?.is_empty() {
            continue;
        }
        let amount = row.money("amount")?.unwrap_or(Money::ZERO);
        if amount.is_zero() {
            continue;
        }
        let key = row.req_line("row", 40)?;
        let parts: Vec<&str> = key.split('.').collect();
        let bad = || row.bad("row", format!("`{key}` is not a row key such as c.0 or p.12.0"));
        let line = match parts.as_slice() {
            ["c", n] => InvoiceLine { change: None, line: n.parse().map_err(|_| bad())?, this_period: amount },
            ["p", c, n] => {
                InvoiceLine { change: Some(c.parse::<u64>().map_err(|_| bad())?), line: n.parse().map_err(|_| bad())?, this_period: amount }
            }
            _ => return Err(bad()),
        };
        if out.iter().any(|l| l.change == line.change && l.line == line.line) {
            return Err(row.bad("row", format!("row {key} is listed twice")));
        }
        out.push(line);
    }
    Ok(out)
}

/// Checks the invoice's lines: every row exists, and none is billed below nothing or beyond its
/// scheduled value.
fn check<T: InvoiceDoc>(ctx: &Ctx<'_>, p: &Params<'_>, inv: &T) -> Result<()> {
    let summary = inv.summary(ctx.state)?;
    for l in inv.lines() {
        let Some(r) = summary.rows.iter().find(|r| r.row.change == l.change && r.row.line == l.line) else {
            return Err(p.bad("lines", "a row is not part of the contract (is its change order approved?)"));
        };
        let to_date = r.to_date();
        let over = !r.row.scheduled.is_negative() && to_date.checked_sub(r.row.scheduled).is_some_and(|o| !o.is_negative() && !o.is_zero());
        if to_date.is_negative() || over {
            return Err(p.bad(
                "lines",
                format!("{} would be billed {} to date against a scheduled value of {}", r.row.description, to_date, r.row.scheduled),
            ));
        }
    }
    Ok(())
}

pub fn create<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let parent = T::check_parent(ctx, project, p)?;
    let mut inv = T::blank(ctx, project, parent);
    let number = inv.number();
    if let Some(d) = p.date("period_end")? {
        inv.set_period(d);
    }
    inv.set_notes(p.text_or_empty("notes", text::LONG)?);
    inv.set_lines(read_lines(p)?);
    check(ctx, p, &inv)?;
    let id = T::table_mut(ctx.state).alloc();
    inv.set_id(id);
    T::table_mut(ctx.state).put(inv);
    Ok(json!({ "id": id, "number": number }))
}

pub fn update<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i: T = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("{} is {}; only drafts can be edited (by their author or an admin)", label(&i), i.status())));
    }
    let before = i.clone();
    if let Some(d) = p.date("period_end")? {
        i.set_period(d);
    }
    if p.present("notes") {
        i.set_notes(p.text_or_empty("notes", text::LONG)?);
    }
    if p.present("lines") {
        i.set_lines(read_lines(p)?);
    }
    check(ctx, p, &i)?;
    let id = i.id();
    if i != before {
        i.set_updated(ctx.stamp());
        T::table_mut(ctx.state).put(i);
    }
    Ok(json!({ "id": id }))
}

fn change_status<T: InvoiceDoc>(ctx: &mut Ctx<'_>, mut i: T, status: InvoiceStatus, apply: impl FnOnce(&mut T)) -> Value {
    let id = i.id();
    i.set_status(status);
    i.set_updated(ctx.stamp());
    apply(&mut i);
    T::table_mut(ctx.state).put(i);
    json!({ "id": id })
}

pub fn submit<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("only the author or an admin can submit {}", label(&i))));
    }
    if i.status() != InvoiceStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}", label(&i), i.status())));
    }
    if i.lines().is_empty() {
        return Err(EngineError::conflict(format!("enter some work completed on {} before submitting it", label(&i))));
    }
    let stamp = ctx.stamp();
    Ok(change_status(ctx, i, InvoiceStatus::Submitted, |x| x.set_submitted(Some(stamp))))
}

pub fn reopen<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    if i.status() != InvoiceStatus::Submitted {
        return Err(EngineError::conflict(format!("{} is {}; only submitted invoices can go back to draft", label(&i), i.status())));
    }
    Ok(change_status(ctx, i, InvoiceStatus::Draft, |x| x.set_submitted(None)))
}

pub fn approve<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    if i.status() != InvoiceStatus::Submitted {
        return Err(EngineError::conflict(format!("{} is {}; only submitted invoices can be approved", label(&i), i.status())));
    }
    let stamp = ctx.stamp();
    Ok(change_status(ctx, i, InvoiceStatus::Approved, |x| x.set_approved(Some(stamp))))
}

pub fn pay<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    if i.status() != InvoiceStatus::Approved {
        return Err(EngineError::conflict(format!("{} is {}; only approved invoices can be paid", label(&i), i.status())));
    }
    let on = p.date("paid_on")?.unwrap_or_else(|| ctx.today());
    Ok(change_status(ctx, i, InvoiceStatus::Paid, |x| x.set_paid_on(Some(on))))
}

pub fn void<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    if matches!(i.status(), InvoiceStatus::Paid | InvoiceStatus::Void) {
        return Err(EngineError::conflict(format!("{} is {}; it can't be voided", label(&i), i.status())));
    }
    // Later invoices were worked out on top of this one; keep the numbering honest.
    let later = T::table(ctx.state)
        .iter()
        .any(|o| o.parent_raw() == i.parent_raw() && o.deleted().is_none() && o.number() > i.number() && o.status().billed());
    if i.status().billed() && later {
        return Err(EngineError::conflict("later invoices build on this one; void them first"));
    }
    Ok(change_status(ctx, i, InvoiceStatus::Void, |_| {}))
}

pub fn delete<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i: T = load(ctx, p)?;
    let own_draft = i.status() == InvoiceStatus::Draft && i.created().by.is_some() && i.created().by == ctx.person();
    if !matches!(i.status(), InvoiceStatus::Draft | InvoiceStatus::Void) {
        return Err(EngineError::conflict(format!("{} is {}; void it instead", label(&i), i.status())));
    }
    if !is_admin::<T>(ctx, i.project()) && !own_draft {
        return Err(EngineError::forbidden("only admins can delete invoices (authors can delete their own drafts)"));
    }
    let stamp = ctx.stamp();
    T::table_mut(ctx.state).update(i.id(), |x| x.set_deleted(Some(stamp)));
    Ok(json!({ "id": i.id() }))
}

pub fn restore<T: InvoiceDoc>(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let i = in_project(T::table(ctx.state), project, id, T::NOUN)?;
    if i.deleted().is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", label(i))));
    }
    T::table_mut(ctx.state).update(id, |x| x.set_deleted(None));
    Ok(json!({ "id": id }))
}

pub fn list<T: InvoiceDoc>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, T::TOOL) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&T> = T::table(v.state).iter().filter(|i| i.project() == project && i.deleted().is_some()).collect();
        return json(&bin);
    }
    let parent = p.opt_id::<T>(T::PARENT)?.map(|x| x.raw());
    let list: Vec<&T> = invoices_for::<T>(v.state, v.actor, project).into_iter().filter(|i| parent.is_none_or(|c| i.parent_raw() == c)).collect();
    json(&list)
}

pub fn get<T: InvoiceDoc>(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<T> = p.id("id")?;
    let i = in_project(T::table(v.state), project, id, T::NOUN)?;
    if !can_see(v.state, v.actor, i) {
        return Err(EngineError::not_found(format!("{} {id} not found in this project", T::NOUN)));
    }
    let s = i.summary(v.state)?;
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

/// Implements the simple field accessors of [`InvoiceDoc`] for a record that has `id`, `number`,
/// `status`, `lines`, `period_end`, `notes`, `submitted`, `approved`, `paid_on`, `created` and
/// `updated`.
#[macro_export]
macro_rules! invoice_doc_fields {
    () => {
        fn set_id(&mut self, id: buildkiosk_core::Id<Self>) {
            self.id = id;
        }
        fn number(&self) -> u32 {
            self.number
        }
        fn status(&self) -> buildkiosk_model::finance::InvoiceStatus {
            self.status
        }
        fn set_status(&mut self, s: buildkiosk_model::finance::InvoiceStatus) {
            self.status = s;
        }
        fn lines(&self) -> &[buildkiosk_model::finance::InvoiceLine] {
            &self.lines
        }
        fn set_lines(&mut self, lines: Vec<buildkiosk_model::finance::InvoiceLine>) {
            self.lines = lines;
        }
        fn set_period(&mut self, d: buildkiosk_core::Date) {
            self.period_end = d;
        }
        fn set_notes(&mut self, n: String) {
            self.notes = n;
        }
        fn set_submitted(&mut self, s: Option<buildkiosk_model::Stamp>) {
            self.submitted = s;
        }
        fn set_approved(&mut self, s: Option<buildkiosk_model::Stamp>) {
            self.approved = s;
        }
        fn set_paid_on(&mut self, d: Option<buildkiosk_core::Date>) {
            self.paid_on = d;
        }
        fn created(&self) -> buildkiosk_model::Stamp {
            self.created
        }
        fn set_updated(&mut self, s: buildkiosk_model::Stamp) {
            self.updated = s;
        }
    };
}

/// The commands of one invoice kind under `$prefix`; `$parent` documents the parent parameter.
#[macro_export]
macro_rules! invoice_commands {
    ($prefix:literal, $noun:literal, $tool:expr, $ty:ty, $parent:literal) => {
        [
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".create"),
                concat!("Start ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                concat!(
                    "project: project id; ",
                    $parent,
                    "; period_end?: YYYY-MM-DD (default: today); notes?: text; \
                     lines?: [{row: row key like c.0 (line 0 of the schedule) or p.12.0 (line 0 of change order 12), amount: work completed this period}]"
                ),
                $crate::invoicedoc::create::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".update"),
                concat!("Edit ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Standard),
                "project, id: ids; period_end?, notes?, lines? (a lines list replaces the old one)",
                $crate::invoicedoc::update::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(concat!($prefix, ".submit"), concat!("Submit ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Standard), "project, id: ids", $crate::invoicedoc::submit::<$ty>),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".reopen"),
                concat!("Send ", $noun, " back to draft"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Admin),
                "project, id: ids (a submitted invoice only)",
                $crate::invoicedoc::reopen::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(concat!($prefix, ".approve"), concat!("Approve ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Admin), "project, id: ids", $crate::invoicedoc::approve::<$ty>),
            $crate::cmd::CommandSpec::mutate(
                concat!($prefix, ".pay"),
                concat!("Record payment on ", $noun),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Admin),
                "project, id: ids; paid_on?: YYYY-MM-DD (default: today)",
                $crate::invoicedoc::pay::<$ty>,
            ),
            $crate::cmd::CommandSpec::mutate(concat!($prefix, ".void"), concat!("Void ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Admin), "project, id: ids", $crate::invoicedoc::void::<$ty>),
            $crate::cmd::CommandSpec::mutate(concat!($prefix, ".delete"), concat!("Delete ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Standard), "project, id: ids (drafts and void invoices)", $crate::invoicedoc::delete::<$ty>),
            $crate::cmd::CommandSpec::mutate(concat!($prefix, ".restore"), concat!("Restore ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Admin), "project, id: ids", $crate::invoicedoc::restore::<$ty>),
            $crate::cmd::CommandSpec::query(
                concat!($prefix, ".list"),
                concat!("List ", $noun, "s"),
                $tool,
                $crate::cmd::Access::Project(buildkiosk_model::Level::Read),
                concat!("project: project id; ", $parent, "?: filter; deleted?: bool (recycle bin, admins)"),
                $crate::invoicedoc::list::<$ty>,
            ),
            $crate::cmd::CommandSpec::query(concat!($prefix, ".get"), concat!("Show ", $noun), $tool, $crate::cmd::Access::Project(buildkiosk_model::Level::Read), "project, id: ids (with the worked-out rows and totals)", $crate::invoicedoc::get::<$ty>),
        ]
    };
}
