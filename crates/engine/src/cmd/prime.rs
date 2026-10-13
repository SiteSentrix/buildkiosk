//! The prime contract (what the owner pays) and its change orders. Both use the shared cost
//! document commands (see [`crate::costdoc`]); this module adds the contract's owner and
//! retainage, and the approve and void steps.
//!
//! A contract is a draft its author edits until an admin approves it; once invoices exist its
//! schedule of values is fixed. Approved prime change orders add billable lines to the contract.

use buildkiosk_core::Id;
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::finance::{ChangeEvent, ChangeStatus, InvoiceStatus, PrimeChange, PrimeContract};
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, Level, Table, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::costdoc::{CostDoc, label, load};
use crate::ctx::{Read, in_project};
use crate::{Ctx, EngineError, Params, Result};

const CONTRACT: [CommandSpec; 6] = crate::cost_doc_commands!(
    "prime_contract",
    "prime contract",
    Tool::PrimeContracts,
    PrimeContract,
    "project: project id; title: text; description?: text; owner?: company id; retainage?: percent (such as 10); lines?: [{cost_code: cost code id, description?: text, amount: scheduled value}]"
);
const CHANGE: [CommandSpec; 6] = crate::cost_doc_commands!(
    "prime_change",
    "prime change order",
    Tool::ChangeOrders,
    PrimeChange,
    "project: project id; contract: prime contract id (can't change later); title: text; description?: text; change_event?: change event id; lines?: [{cost_code: cost code id, description?: text, amount: amount, may be negative}]"
);

pub const COMMANDS: &[CommandSpec] = &[
    CONTRACT[0],
    CONTRACT[1],
    CommandSpec::mutate(
        "prime_contract.approve",
        "Approve prime contract",
        Tool::PrimeContracts,
        Access::Project(Level::Admin),
        "project, id: ids (needs at least one line)",
        approve_contract,
    ),
    CommandSpec::mutate(
        "prime_contract.void",
        "Void prime contract",
        Tool::PrimeContracts,
        Access::Project(Level::Admin),
        "project, id: ids (not once invoices exist)",
        void_contract,
    ),
    CONTRACT[2],
    CONTRACT[3],
    CONTRACT[4],
    CONTRACT[5],
    CHANGE[0],
    CHANGE[1],
    CommandSpec::mutate(
        "prime_change.approve",
        "Approve prime change order",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids",
        approve_change,
    ),
    CommandSpec::mutate(
        "prime_change.void",
        "Void prime change order",
        Tool::ChangeOrders,
        Access::Project(Level::Admin),
        "project, id: ids (not once an invoice bills its lines)",
        void_change,
    ),
    CHANGE[2],
    CHANGE[3],
    CHANGE[4],
    CHANGE[5],
];

fn next_number<T: CostDoc>(state: &CompanyState, project: Id<Project>) -> u32 {
    T::table(state).iter().filter(|d| d.project() == project).map(|d| d.number()).max().unwrap_or(0).saturating_add(1)
}

/// Whether any live invoice (not void) exists for the contract.
fn has_invoices(state: &CompanyState, contract: Id<PrimeContract>) -> bool {
    state.owner_invoices.iter().any(|o| o.contract == contract && o.deleted.is_none() && o.status != InvoiceStatus::Void)
}

impl CostDoc for PrimeContract {
    const TOOL: Tool = Tool::PrimeContracts;
    const NOUN: &'static str = "prime contract";
    const LABEL: &'static str = "Prime contract";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.prime_contracts
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.prime_contracts
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        PrimeContract {
            id: ctx.state.prime_contracts.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            title: String::new(),
            description: String::new(),
            owner: None,
            retainage: None,
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
        if p.present("owner") {
            let v = p.opt_id::<Vendor>("owner")?;
            if let Some(v) = v {
                ctx.check_vendor(v)?;
            }
            self.owner = v;
        }
        if p.present("retainage") {
            self.retainage = super::invoice::retainage(p)?;
        }
        if !creating && p.present("lines") && has_invoices(ctx.state, self.id) {
            return Err(EngineError::conflict("invoices exist for this contract, so its schedule of values can't change"));
        }
        Ok(())
    }
}

fn approve_contract(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: PrimeContract = load(ctx, p)?;
    if c.status != ChangeStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}; only drafts can be approved", label(&c), c.status)));
    }
    if c.lines.is_empty() {
        return Err(EngineError::conflict(format!("add at least one line to {} before approving it", label(&c))));
    }
    let stamp = ctx.stamp();
    ctx.state.prime_contracts.update(c.id, |x| {
        x.status = ChangeStatus::Approved;
        x.approved = Some(stamp);
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

fn void_contract(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: PrimeContract = load(ctx, p)?;
    if c.status == ChangeStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", label(&c))));
    }
    if has_invoices(ctx.state, c.id) {
        return Err(EngineError::conflict("invoices exist for this contract; void them first"));
    }
    let stamp = ctx.stamp();
    ctx.state.prime_contracts.update(c.id, |x| {
        x.status = ChangeStatus::Void;
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

impl CostDoc for PrimeChange {
    const TOOL: Tool = Tool::ChangeOrders;
    const NOUN: &'static str = "prime change order";
    const LABEL: &'static str = "Prime change order";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.prime_changes
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.prime_changes
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        PrimeChange {
            id: ctx.state.prime_changes.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            contract: Id::new(0),
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
            let c: Id<PrimeContract> = p.id("contract")?;
            let contract = in_project(&ctx.state.prime_contracts, self.project, c, "prime contract")?;
            if contract.deleted.is_some() || contract.status == ChangeStatus::Void {
                return Err(EngineError::conflict(format!("{} is void", label(contract))));
            }
            self.contract = c;
        } else if p.present("contract") && p.opt_id::<PrimeContract>("contract")? != Some(self.contract) {
            return Err(p.bad("contract", "can't be changed; create a new change order instead"));
        }
        if p.present("change_event") {
            let e = p.opt_id::<ChangeEvent>("change_event")?;
            if let Some(e) = e {
                in_project(&ctx.state.change_events, self.project, e, "change event")?;
            }
            self.change_event = e;
        }
        Ok(())
    }
}

fn approve_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: PrimeChange = load(ctx, p)?;
    if c.status != ChangeStatus::Draft {
        return Err(EngineError::conflict(format!("{} is {}; only drafts can be approved", label(&c), c.status)));
    }
    if c.lines.is_empty() {
        return Err(EngineError::conflict(format!("add at least one line to {} before approving it", label(&c))));
    }
    let approved_contract = ctx.state.prime_contracts.get(c.contract).is_some_and(|k| k.status == ChangeStatus::Approved);
    if !approved_contract {
        return Err(EngineError::conflict("the prime contract must be approved before its change orders can be"));
    }
    let stamp = ctx.stamp();
    ctx.state.prime_changes.update(c.id, |x| {
        x.status = ChangeStatus::Approved;
        x.approved = Some(stamp);
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}

fn void_change(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let c: PrimeChange = load(ctx, p)?;
    if c.status == ChangeStatus::Void {
        return Err(EngineError::conflict(format!("{} is already void", label(&c))));
    }
    let billed = ctx.state.owner_invoices.iter().any(|o| {
        o.deleted.is_none() && o.status != InvoiceStatus::Void && o.lines.iter().any(|l| l.change == Some(c.id.raw()) && !l.this_period.is_zero())
    });
    if billed {
        return Err(EngineError::conflict("an invoice bills this change order's lines; void or edit that invoice first"));
    }
    let stamp = ctx.stamp();
    ctx.state.prime_changes.update(c.id, |x| {
        x.status = ChangeStatus::Void;
        x.updated = stamp;
    });
    Ok(json!({ "id": c.id }))
}
