//! Change management: change events (possible changes, tracked while they are priced), commitment
//! change orders (approved changes to what a subcontract or purchase order costs) and budget
//! changes (approved moves of budget between cost codes). The shared record commands are in
//! [`crate::costdoc`]; this module adds each record's own steps.
//!
//! - change event: open → closed (its author or an admin) → reopened (admins); admins void it;
//! - commitment change order and budget change: a draft its author edits → approved by an admin
//!   (which makes its amounts count in the budget) → voided by an admin if it was a mistake.

use buildkiosk_core::Id;
use buildkiosk_model::finance::{BudgetChange, ChangeEvent, ChangeEventStatus, ChangeStatus, Commitment, CommitmentChange, CostLine};
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, Level, Table, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::costdoc::{self, CostDoc, label, load};
use crate::ctx::in_project;
use crate::{Ctx, EngineError, Params, Result};

const CHANGE_EVENT: [CommandSpec; 6] = crate::cost_doc_commands!(
    "change_event",
    "change event",
    Tool::ChangeEvents,
    ChangeEvent,
    "project: project id; title: text; description?, reason?: text; lines?: [{cost_code: cost code id, description?: text, amount: estimated amount}]"
);
const COMMITMENT_CHANGE: [CommandSpec; 6] = crate::cost_doc_commands!(
    "commitment_change",
    "commitment change order",
    Tool::ChangeOrders,
    CommitmentChange,
    "project: project id; commitment: commitment id (can't change later); title: text; description?: text; change_event?: change event id; lines?: [{cost_code: cost code id, description?: text, amount: amount, may be negative}]"
);
const BUDGET_CHANGE: [CommandSpec; 6] = crate::cost_doc_commands!(
    "budget_change",
    "budget change",
    Tool::ChangeOrders,
    BudgetChange,
    "project: project id; title: text; description?: text; change_event?: change event id; lines?: [{cost_code: cost code id, description?: text, amount: amount, may be negative}]"
);

pub const COMMANDS: &[CommandSpec] = &[
    CHANGE_EVENT[0],
    CHANGE_EVENT[1],
    CommandSpec::mutate(
        "change_event.close",
        "Close change event",
        Tool::ChangeEvents,
        Access::Project(Level::Standard),
        "project, id: ids",
        close_event,
    ),
    CommandSpec::mutate(
        "change_event.reopen",
        "Reopen change event",
        Tool::ChangeEvents,
        Access::Project(Level::Admin),
        "project, id: ids",
        reopen_event,
    ),
    CommandSpec::mutate("change_event.void", "Void change event", Tool::ChangeEvents, Access::Project(Level::Admin), "project, id: ids", void_event),
    CHANGE_EVENT[2],
    CHANGE_EVENT[3],
    CHANGE_EVENT[4],
    CHANGE_EVENT[5],
    COMMITMENT_CHANGE[0],
    COMMITMENT_CHANGE[1],
    CommandSpec::mutate(
        "commitment_change.approve",
        "Approve commitment change order",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids",
        approve_commitment_change,
    ),
    CommandSpec::mutate(
        "commitment_change.void",
        "Void commitment change order",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids",
        void_commitment_change,
    ),
    COMMITMENT_CHANGE[2],
    COMMITMENT_CHANGE[3],
    COMMITMENT_CHANGE[4],
    COMMITMENT_CHANGE[5],
    BUDGET_CHANGE[0],
    BUDGET_CHANGE[1],
    CommandSpec::mutate(
        "budget_change.approve",
        "Approve budget change",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids",
        approve_budget_change,
    ),
    CommandSpec::mutate(
        "budget_change.void",
        "Void budget change",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids",
        void_budget_change,
    ),
    BUDGET_CHANGE[2],
    BUDGET_CHANGE[3],
    BUDGET_CHANGE[4],
    BUDGET_CHANGE[5],
];

fn next_number<T: CostDoc>(state: &CompanyState, project: Id<Project>) -> u32 {
    T::table(state).iter().filter(|d| d.project() == project).map(|d| d.number()).max().unwrap_or(0).saturating_add(1)
}

fn check_event(ctx: &Ctx<'_>, project: Id<Project>, p: &Params<'_>) -> Result<Option<Id<ChangeEvent>>> {
    let e = p.opt_id::<ChangeEvent>("change_event")?;
    if let Some(e) = e {
        in_project(&ctx.state.change_events, project, e, "change event")?;
    }
    Ok(e)
}

// -- change events

impl CostDoc for ChangeEvent {
    const TOOL: Tool = Tool::ChangeEvents;
    const NOUN: &'static str = "change event";
    const LABEL: &'static str = "Change event";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.change_events
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.change_events
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        ChangeEvent {
            id: ctx.state.change_events.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            title: String::new(),
            description: String::new(),
            reason: String::new(),
            status: ChangeEventStatus::Open,
            lines: Vec::new(),
            created: stamp,
            updated: stamp,
            deleted: None,
        }
    }
    crate::cost_doc_fields!();
    fn is_editable(&self) -> bool {
        self.status == ChangeEventStatus::Open
    }
    fn is_void(&self) -> bool {
        self.status == ChangeEventStatus::Void
    }
    fn apply_extra(&mut self, _ctx: &Ctx<'_>, p: &Params<'_>, _creating: bool) -> Result<()> {
        if p.present("reason") {
            self.reason = p.line_or_empty("reason", buildkiosk_core::text::SHORT)?;
        }
        Ok(())
    }
}

fn set_event_status(ctx: &mut Ctx<'_>, e: ChangeEvent, from: ChangeEventStatus, to: ChangeEventStatus, verb: &str) -> Result<Value> {
    if e.status != from {
        return Err(EngineError::conflict(format!("{} is {}; only {} change events can be {verb}", label(&e), e.status, from)));
    }
    let stamp = ctx.stamp();
    ctx.state.change_events.update(e.id, |x| {
        x.status = to;
        x.updated = stamp;
    });
    Ok(json!({ "id": e.id }))
}

fn close_event(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e: ChangeEvent = load(ctx, p)?;
    if !costdoc::can_edit(ctx.state, ctx.actor, &e) {
        return Err(EngineError::forbidden(format!("only the author or an admin can close {}", label(&e))));
    }
    set_event_status(ctx, e, ChangeEventStatus::Open, ChangeEventStatus::Closed, "closed")
}

fn reopen_event(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e: ChangeEvent = load(ctx, p)?;
    set_event_status(ctx, e, ChangeEventStatus::Closed, ChangeEventStatus::Open, "reopened")
}

fn void_event(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e: ChangeEvent = load(ctx, p)?;
    if e.status == ChangeEventStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", label(&e))));
    }
    let stamp = ctx.stamp();
    ctx.state.change_events.update(e.id, |x| {
        x.status = ChangeEventStatus::Void;
        x.updated = stamp;
    });
    Ok(json!({ "id": e.id }))
}

// -- commitment change orders

impl CostDoc for CommitmentChange {
    const TOOL: Tool = Tool::ChangeOrders;
    const NOUN: &'static str = "commitment change order";
    const LABEL: &'static str = "Commitment change order";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.commitment_changes
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.commitment_changes
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        CommitmentChange {
            id: ctx.state.commitment_changes.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            commitment: Id::new(0),
            title: String::new(),
            description: String::new(),
            change_event: None,
            status: ChangeStatus::Draft,
            lines: Vec::new(),
            approved: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        }
    }
    crate::cost_doc_fields!();
    fn is_editable(&self) -> bool {
        self.status == ChangeStatus::Draft
    }
    fn is_void(&self) -> bool {
        self.status == ChangeStatus::Void
    }
    fn is_private(&self) -> bool {
        self.status == ChangeStatus::Draft
    }
    fn apply_extra(&mut self, ctx: &Ctx<'_>, p: &Params<'_>, creating: bool) -> Result<()> {
        if creating {
            let c: Id<Commitment> = p.id("commitment")?;
            let commitment = in_project(&ctx.state.commitments, self.project, c, "commitment")?;
            if commitment.deleted.is_some() || commitment.status == buildkiosk_model::finance::CommitmentStatus::Void {
                return Err(EngineError::conflict(format!("{} is void", commitment.label())));
            }
            self.commitment = c;
        } else if p.present("commitment") && p.opt_id::<Commitment>("commitment")? != Some(self.commitment) {
            return Err(p.bad("commitment", "can't be changed; create a new change order instead"));
        }
        if p.present("change_event") {
            self.change_event = check_event(ctx, self.project, p)?;
        }
        Ok(())
    }
}

fn need_lines(lines: &[CostLine], label_text: &str) -> Result<()> {
    if lines.is_empty() {
        return Err(EngineError::conflict(format!("add at least one line to {label_text} before approving it")));
    }
    Ok(())
}

fn approve_commitment_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: CommitmentChange = load(ctx, p)?;
    if c.status != ChangeStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}; only drafts can be approved", label(&c), c.status)));
    }
    need_lines(&c.lines, &label(&c))?;
    let commitment = in_project(&ctx.state.commitments, c.project, c.commitment, "commitment")?;
    if !commitment.status.counts() {
        return Err(EngineError::conflict(format!("{} must be approved before its change orders can be", commitment.label())));
    }
    let stamp = ctx.stamp();
    ctx.state.commitment_changes.update(c.id, |x| {
        x.status = ChangeStatus::Approved;
        x.approved = Some(stamp);
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

fn void_commitment_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: CommitmentChange = load(ctx, p)?;
    if c.status == ChangeStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", label(&c))));
    }
    let billed = ctx.state.commitment_invoices.iter().any(|o| {
        o.deleted.is_none()
            && o.status != buildkiosk_model::finance::InvoiceStatus::Void
            && o.lines.iter().any(|l| l.change == Some(c.id.raw()) && !l.this_period.is_zero())
    });
    if billed {
        return Err(EngineError::conflict("an invoice bills this change order's lines; void or edit that invoice first"));
    }
    let stamp = ctx.stamp();
    ctx.state.commitment_changes.update(c.id, |x| {
        x.status = ChangeStatus::Void;
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

// -- budget changes

impl CostDoc for BudgetChange {
    const TOOL: Tool = Tool::ChangeOrders;
    const NOUN: &'static str = "budget change";
    const LABEL: &'static str = "Budget change";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.budget_changes
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.budget_changes
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        BudgetChange {
            id: ctx.state.budget_changes.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            title: String::new(),
            description: String::new(),
            change_event: None,
            status: ChangeStatus::Draft,
            lines: Vec::new(),
            approved: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        }
    }
    crate::cost_doc_fields!();
    fn is_editable(&self) -> bool {
        self.status == ChangeStatus::Draft
    }
    fn is_void(&self) -> bool {
        self.status == ChangeStatus::Void
    }
    fn is_private(&self) -> bool {
        self.status == ChangeStatus::Draft
    }
    fn apply_extra(&mut self, ctx: &Ctx<'_>, p: &Params<'_>, _creating: bool) -> Result<()> {
        if p.present("change_event") {
            self.change_event = check_event(ctx, self.project, p)?;
        }
        Ok(())
    }
}

fn approve_budget_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: BudgetChange = load(ctx, p)?;
    if c.status != ChangeStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}; only drafts can be approved", label(&c), c.status)));
    }
    need_lines(&c.lines, &label(&c))?;
    let stamp = ctx.stamp();
    ctx.state.budget_changes.update(c.id, |x| {
        x.status = ChangeStatus::Approved;
        x.approved = Some(stamp);
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

fn void_budget_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: BudgetChange = load(ctx, p)?;
    if c.status == ChangeStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", label(&c))));
    }
    let stamp = ctx.stamp();
    ctx.state.budget_changes.update(c.id, |x| {
        x.status = ChangeStatus::Void;
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}
