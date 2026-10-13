//! Read helpers shared by queries and the UI.
//!
//! The UI reads the state directly (it's thin: it shows engine state and acts through commands);
//! these helpers keep the rules for what a person may see in one place, so a page and the query
//! behind the API answer the same.

use buildkiosk_core::{Id, Money, Percent};
use buildkiosk_model::daily_log::{DailyLog, LogStatus};
use buildkiosk_model::directory::{Person, Vendor};
use buildkiosk_model::document::{Document, Folder};
use buildkiosk_model::finance::{
    ChangeEventStatus, ChangeStatus, Commitment, CommitmentInvoice, CommitmentStatus, InvoiceLine, InvoiceStatus, OwnerInvoice, PrimeChange,
    PrimeContract,
};
use buildkiosk_model::inspection::Inspection;
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::rfi::{Rfi, RfiStatus};
use buildkiosk_model::submittal::{Submittal, SubmittalStatus};
use buildkiosk_model::{CompanyState, Level, Tool, perm};
use serde::Serialize;
use serde_json::Value;

use crate::{Actor, EngineError};

/// Serializes a query result.
pub fn json<T: Serialize>(value: &T) -> Result<Value, EngineError> {
    serde_json::to_value(value).map_err(|e| EngineError::Internal { cmd: "view".into(), msg: e.to_string() })
}

pub fn person_name(state: &CompanyState, id: Id<Person>) -> String {
    state.people.get(id).map(Person::name).unwrap_or_else(|| format!("Unknown person {id}"))
}

pub fn people_names(state: &CompanyState, ids: &[Id<Person>]) -> String {
    ids.iter().map(|id| person_name(state, *id)).collect::<Vec<_>>().join(", ")
}

pub fn vendor_name(state: &CompanyState, id: Id<Vendor>) -> String {
    state.vendors.get(id).map(|v| v.name.clone()).unwrap_or_else(|| format!("Unknown company {id}"))
}

/// The company a person works for ("our" company when `vendor` is empty).
pub fn employer_name(state: &CompanyState, person: &Person) -> String {
    match person.vendor {
        Some(v) => vendor_name(state, v),
        None => state.company().map(|c| c.name.clone()).unwrap_or_default(),
    }
}

/// The actor's level in a project tool.
pub fn level(state: &CompanyState, actor: Actor, project: Id<Project>, tool: Tool) -> Level {
    match actor {
        Actor::System => Level::Admin,
        Actor::Person(p) => perm::project_level(state, p, project, tool),
    }
}

/// Projects the actor may open, newest first.
pub fn projects_for(state: &CompanyState, actor: Actor) -> Vec<&Project> {
    let mut list: Vec<&Project> = match actor {
        Actor::System => state.projects.iter().filter(|p| p.deleted.is_none()).collect(),
        Actor::Person(person) => perm::visible_projects(state, person).into_iter().filter_map(|id| state.projects.get(id)).collect(),
    };
    list.reverse();
    list
}

/// Whether the actor may see an RFI: Read access, and for drafts and private RFIs, being part of it
/// (or an RFI admin).
pub fn can_see_rfi(state: &CompanyState, actor: Actor, rfi: &Rfi) -> bool {
    let lvl = level(state, actor, rfi.project, Tool::Rfis);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || rfi.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match rfi.status {
        RfiStatus::Draft => rfi.created.by == Some(person) || rfi.manager == Some(person),
        _ => !rfi.private || rfi.involves(person),
    }
}

/// The RFIs of a project the actor may see, in number order.
pub fn rfis_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Rfi> {
    let mut list: Vec<&Rfi> = state.rfis.iter().filter(|r| r.project == project && can_see_rfi(state, actor, r)).collect();
    list.sort_by_key(|r| r.number);
    list
}

/// Whether the actor may see a submittal: Read access, and for drafts and private submittals,
/// being part of it (or a submittals admin).
pub fn can_see_submittal(state: &CompanyState, actor: Actor, s: &Submittal) -> bool {
    let lvl = level(state, actor, s.project, Tool::Submittals);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || s.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match s.status {
        SubmittalStatus::Draft => s.created.by == Some(person) || s.manager == Some(person),
        _ => !s.private || s.involves(person),
    }
}

/// The submittals of a project the actor may see, in number order.
pub fn submittals_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Submittal> {
    let mut list: Vec<&Submittal> = state.submittals.iter().filter(|s| s.project == project && can_see_submittal(state, actor, s)).collect();
    list.sort_by_key(|s| s.number);
    list
}

/// Whether the actor may see a daily log: Read access, and for drafts, being the author (or a
/// daily log admin).
pub fn can_see_daily_log(state: &CompanyState, actor: Actor, log: &DailyLog) -> bool {
    let lvl = level(state, actor, log.project, Tool::DailyLog);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || log.deleted.is_some() {
        return false;
    }
    log.status == LogStatus::Submitted || (actor.person().is_some() && log.created.by == actor.person())
}

/// The daily logs of a project the actor may see, newest day first.
pub fn daily_logs_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&DailyLog> {
    let mut list: Vec<&DailyLog> = state.daily_logs.iter().filter(|l| l.project == project && can_see_daily_log(state, actor, l)).collect();
    list.sort_by_key(|l| std::cmp::Reverse(l.date));
    list
}

/// Whether the actor may see an inspection: Read access to the tool.
pub fn can_see_inspection(state: &CompanyState, actor: Actor, i: &Inspection) -> bool {
    let lvl = level(state, actor, i.project, Tool::Inspections);
    lvl == Level::Admin || (i.deleted.is_none() && lvl >= Level::Read)
}

/// The inspections of a project the actor may see, in number order.
pub fn inspections_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Inspection> {
    let mut list: Vec<&Inspection> =
        state.inspections.iter().filter(|i| i.project == project && i.deleted.is_none() && can_see_inspection(state, actor, i)).collect();
    list.sort_by_key(|i| i.number);
    list
}

/// One cost code's line of the budget view.
#[derive(Clone, Debug)]
pub struct BudgetRow {
    pub cost_code: Id<CostCode>,
    /// Whether the code has a budget line (costs can land on codes that have none).
    pub budgeted: bool,
    pub original: Money,
    /// Approved budget changes.
    pub changes: Money,
    /// Approved and complete commitments, with their approved change orders.
    pub committed: Money,
    pub direct: Money,
    /// The estimates of open change events: possible cost not yet committed.
    pub pending: Money,
    /// An estimate of the cost still to come (the budget line's forecast).
    pub forecast_to_complete: Option<Money>,
}

impl BudgetRow {
    /// The cost expected at completion: cost to date plus the forecast when there is one, else the
    /// larger of the revised budget and the cost so far (the budget is assumed to be spent).
    pub fn projected(&self) -> Money {
        match self.forecast_to_complete {
            Some(f) => self.cost().checked_add(f).unwrap_or(Money::ZERO),
            None => self.revised().max(self.cost()),
        }
    }

    /// Revised budget less projected cost: negative means a projected overrun.
    pub fn variance(&self) -> Money {
        self.revised().checked_sub(self.projected()).unwrap_or(Money::ZERO)
    }

    /// Committed plus direct costs.
    pub fn cost(&self) -> Money {
        self.committed.checked_add(self.direct).unwrap_or(Money::ZERO)
    }

    /// The original budget with its approved changes.
    pub fn revised(&self) -> Money {
        self.original.checked_add(self.changes).unwrap_or(Money::ZERO)
    }

    /// What is left of the revised budget.
    pub fn remaining(&self) -> Money {
        self.revised().checked_sub(self.cost()).unwrap_or(Money::ZERO)
    }
}

/// A cost code usable in a project: a company-wide code or one of the project's own.
pub fn project_cost_codes(state: &CompanyState, project: Id<Project>) -> Vec<&CostCode> {
    let mut list: Vec<&CostCode> =
        state.cost_codes.iter().filter(|c| c.deleted.is_none() && (c.project.is_none() || c.project == Some(project))).collect();
    list.sort_by(|a, b| a.code.cmp(&b.code));
    list
}

fn too_big() -> EngineError {
    EngineError::conflict("the amounts are too large to add up")
}

fn budget_row(rows: &mut std::collections::BTreeMap<Id<CostCode>, BudgetRow>, code: Id<CostCode>) -> &mut BudgetRow {
    rows.entry(code).or_insert(BudgetRow {
        cost_code: code,
        budgeted: false,
        original: Money::ZERO,
        changes: Money::ZERO,
        committed: Money::ZERO,
        direct: Money::ZERO,
        pending: Money::ZERO,
        forecast_to_complete: None,
    })
}

/// The budget of a project by cost code: original budget, committed cost and direct cost, for every
/// code that has any of them, in code order.
pub fn budget_rows(state: &CompanyState, project: Id<Project>) -> Result<Vec<BudgetRow>, EngineError> {
    let mut rows: std::collections::BTreeMap<Id<CostCode>, BudgetRow> = std::collections::BTreeMap::new();
    for b in state.budget_lines.iter().filter(|b| b.project == project && b.deleted.is_none()) {
        let r = budget_row(&mut rows, b.cost_code);
        r.budgeted = true;
        r.original = r.original.checked_add(b.original).ok_or_else(too_big)?;
        if let Some(f) = b.forecast_to_complete {
            r.forecast_to_complete = Some(r.forecast_to_complete.unwrap_or(Money::ZERO).checked_add(f).ok_or_else(too_big)?);
        }
    }
    for c in state.commitments.iter().filter(|c| c.project == project && c.deleted.is_none() && c.status.counts()) {
        for l in &c.lines {
            let r = budget_row(&mut rows, l.cost_code);
            r.committed = r.committed.checked_add(l.amount).ok_or_else(too_big)?;
        }
    }
    // Approved change orders count with their commitment, as long as the commitment counts.
    for c in state.commitment_changes.iter().filter(|c| c.project == project && c.deleted.is_none() && c.status == ChangeStatus::Approved) {
        if !state.commitments.get(c.commitment).is_some_and(|m| m.deleted.is_none() && m.status.counts()) {
            continue;
        }
        for l in &c.lines {
            let r = budget_row(&mut rows, l.cost_code);
            r.committed = r.committed.checked_add(l.amount).ok_or_else(too_big)?;
        }
    }
    for c in state.budget_changes.iter().filter(|c| c.project == project && c.deleted.is_none() && c.status == ChangeStatus::Approved) {
        for l in &c.lines {
            let r = budget_row(&mut rows, l.cost_code);
            r.changes = r.changes.checked_add(l.amount).ok_or_else(too_big)?;
        }
    }
    for e in state.change_events.iter().filter(|e| e.project == project && e.deleted.is_none() && e.status == ChangeEventStatus::Open) {
        for l in &e.lines {
            let r = budget_row(&mut rows, l.cost_code);
            r.pending = r.pending.checked_add(l.amount).ok_or_else(too_big)?;
        }
    }
    for d in state.direct_costs.iter().filter(|d| d.project == project && d.deleted.is_none()) {
        let r = budget_row(&mut rows, d.cost_code);
        r.direct = r.direct.checked_add(d.amount).ok_or_else(too_big)?;
    }
    let mut list: Vec<BudgetRow> = rows.into_values().collect();
    list.sort_by(|a, b| {
        let code = |id: Id<CostCode>| state.cost_codes.get(id).map(|c| c.code.clone()).unwrap_or_default();
        code(a.cost_code).cmp(&code(b.cost_code))
    });
    Ok(list)
}

/// Totals of the budget rows.
#[derive(Clone, Copy, Debug)]
pub struct BudgetTotals {
    pub original: Money,
    pub changes: Money,
    pub committed: Money,
    pub direct: Money,
    pub pending: Money,
    pub projected: Money,
}

impl BudgetTotals {
    /// Revised budget less projected cost.
    pub fn variance(&self) -> Money {
        self.revised().checked_sub(self.projected).unwrap_or(Money::ZERO)
    }

    pub fn revised(&self) -> Money {
        self.original.checked_add(self.changes).unwrap_or(Money::ZERO)
    }

    pub fn cost(&self) -> Money {
        self.committed.checked_add(self.direct).unwrap_or(Money::ZERO)
    }

    pub fn remaining(&self) -> Money {
        self.revised().checked_sub(self.cost()).unwrap_or(Money::ZERO)
    }
}

pub fn budget_totals(rows: &[BudgetRow]) -> Result<BudgetTotals, EngineError> {
    let sum = |f: fn(&BudgetRow) -> Money| Money::sum(rows.iter().map(f)).ok_or_else(too_big);
    Ok(BudgetTotals {
        original: sum(|r| r.original)?,
        changes: sum(|r| r.changes)?,
        committed: sum(|r| r.committed)?,
        direct: sum(|r| r.direct)?,
        pending: sum(|r| r.pending)?,
        projected: sum(|r| r.projected())?,
    })
}

/// A commitment's total with its approved change orders (None if the sum doesn't fit).
pub fn commitment_revised_total(state: &CompanyState, c: &Commitment) -> Option<Money> {
    let changes = state.commitment_changes.iter().filter(|x| x.commitment == c.id && x.deleted.is_none() && x.status == ChangeStatus::Approved);
    Money::sum(c.total().into_iter().chain(changes.map(|x| Money::sum(x.lines.iter().map(|l| l.amount))).collect::<Option<Vec<_>>>()?))
}

/// Whether the actor may see a commitment: Read access, and drafts only for their author and admins.
pub fn can_see_commitment(state: &CompanyState, actor: Actor, c: &Commitment) -> bool {
    let lvl = level(state, actor, c.project, Tool::Commitments);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || c.deleted.is_some() {
        return false;
    }
    c.status != CommitmentStatus::Draft || (actor.person().is_some() && c.created.by == actor.person())
}

/// The commitments of a project the actor may see, in number order.
pub fn commitments_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Commitment> {
    let mut list: Vec<&Commitment> =
        state.commitments.iter().filter(|c| c.project == project && c.deleted.is_none() && can_see_commitment(state, actor, c)).collect();
    list.sort_by_key(|c| c.number);
    list
}

/// Whether the actor may see a commitment change order (drafts: its author and admins).
pub fn can_see_commitment_change(state: &CompanyState, actor: Actor, c: &buildkiosk_model::finance::CommitmentChange) -> bool {
    crate::costdoc::can_see(state, actor, c)
}

// -- billing (owner invoices and commitment invoices)

/// One billable row: a line of a contract's or commitment's schedule of values, or of one of its
/// approved change orders.
#[derive(Clone, Debug)]
pub struct BillRow {
    /// The raw id of the change order the line belongs to, if any.
    pub change: Option<u64>,
    pub line: u32,
    pub description: String,
    pub cost_code: Id<CostCode>,
    pub scheduled: Money,
}

impl BillRow {
    /// The key a form uses for the row: `c.3` or `p.12.0`.
    pub fn key(&self) -> String {
        match self.change {
            None => format!("c.{}", self.line),
            Some(c) => format!("p.{c}.{}", self.line),
        }
    }
}

fn base_rows(lines: &[buildkiosk_model::finance::CostLine]) -> Vec<BillRow> {
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| BillRow {
            change: None,
            line: u32::try_from(i).unwrap_or(u32::MAX),
            description: l.description.clone(),
            cost_code: l.cost_code,
            scheduled: l.amount,
        })
        .collect()
}

fn change_rows(rows: &mut Vec<BillRow>, number: u32, id: u64, lines: &[buildkiosk_model::finance::CostLine]) {
    for (i, l) in lines.iter().enumerate() {
        let description = if l.description.is_empty() { format!("Change order #{number}") } else { format!("CO #{number}: {}", l.description) };
        rows.push(BillRow { change: Some(id), line: u32::try_from(i).unwrap_or(u32::MAX), description, cost_code: l.cost_code, scheduled: l.amount });
    }
}

/// The rows of a prime contract that can be billed: its schedule of values, then the lines of its
/// approved change orders in number order.
pub fn billable_rows(state: &CompanyState, contract: &PrimeContract) -> Vec<BillRow> {
    let mut rows = base_rows(&contract.lines);
    let mut changes: Vec<&PrimeChange> =
        state.prime_changes.iter().filter(|c| c.contract == contract.id && c.deleted.is_none() && c.status == ChangeStatus::Approved).collect();
    changes.sort_by_key(|c| c.number);
    for c in changes {
        change_rows(&mut rows, c.number, c.id.raw(), &c.lines);
    }
    rows
}

/// The rows of a commitment that can be billed: its lines, then the lines of its approved change
/// orders in number order.
pub fn commitment_billable_rows(state: &CompanyState, commitment: &Commitment) -> Vec<BillRow> {
    let mut rows = base_rows(&commitment.lines);
    let mut changes: Vec<&buildkiosk_model::finance::CommitmentChange> = state
        .commitment_changes
        .iter()
        .filter(|c| c.commitment == commitment.id && c.deleted.is_none() && c.status == ChangeStatus::Approved)
        .collect();
    changes.sort_by_key(|c| c.number);
    for c in changes {
        change_rows(&mut rows, c.number, c.id.raw(), &c.lines);
    }
    rows
}

/// A row of an invoice with its running amounts.
#[derive(Clone, Debug)]
pub struct InvoiceRow {
    pub row: BillRow,
    /// Billed on earlier invoices.
    pub previous: Money,
    pub this_period: Money,
}

impl InvoiceRow {
    pub fn to_date(&self) -> Money {
        self.previous.checked_add(self.this_period).unwrap_or(Money::ZERO)
    }

    pub fn balance(&self) -> Money {
        self.row.scheduled.checked_sub(self.to_date()).unwrap_or(Money::ZERO)
    }
}

/// An invoice worked out: the rows and the totals a payment application shows.
#[derive(Clone, Debug)]
pub struct InvoiceSummary {
    pub rows: Vec<InvoiceRow>,
    pub scheduled: Money,
    pub previous: Money,
    pub this_period: Money,
    pub to_date: Money,
    pub retainage_rate: Percent,
    /// Withheld on all work to date.
    pub retainage_to_date: Money,
    /// Withheld on this invoice's work (the change in retainage held).
    pub retainage_this: Money,
    /// What is owed for this invoice: this period's work less the retainage withheld on it.
    pub net_due: Money,
}

fn overflow() -> EngineError {
    EngineError::conflict("the amounts are too large to add up")
}

/// Works out an invoice from the billable rows, the lines of earlier billed invoices and its own.
fn summarize(rows: Vec<BillRow>, earlier: &[&[InvoiceLine]], this: &[InvoiceLine], rate: Percent) -> Result<InvoiceSummary, EngineError> {
    let mut out = Vec::new();
    for row in rows {
        let amounts = |lines: &[InvoiceLine]| -> Vec<Money> {
            lines.iter().filter(|l| l.change == row.change && l.line == row.line).map(|l| l.this_period).collect()
        };
        let previous = Money::sum(earlier.iter().flat_map(|l| amounts(l))).ok_or_else(overflow)?;
        let this_period = Money::sum(amounts(this)).ok_or_else(overflow)?;
        out.push(InvoiceRow { row, previous, this_period });
    }
    let sum = |f: fn(&InvoiceRow) -> Money| Money::sum(out.iter().map(f)).ok_or_else(overflow);
    let scheduled = Money::sum(out.iter().map(|r| r.row.scheduled)).ok_or_else(overflow)?;
    let (previous, this_period) = (sum(|r| r.previous)?, sum(|r| r.this_period)?);
    let to_date = previous.checked_add(this_period).ok_or_else(overflow)?;
    let retainage_to_date = to_date.percent(rate).ok_or_else(overflow)?;
    let retainage_previous = previous.percent(rate).ok_or_else(overflow)?;
    let retainage_this = retainage_to_date.checked_sub(retainage_previous).ok_or_else(overflow)?;
    let net_due = this_period.checked_sub(retainage_this).ok_or_else(overflow)?;
    Ok(InvoiceSummary { rows: out, scheduled, previous, this_period, to_date, retainage_rate: rate, retainage_to_date, retainage_this, net_due })
}

/// Works out an owner invoice.
pub fn invoice_summary(state: &CompanyState, inv: &OwnerInvoice) -> Result<InvoiceSummary, EngineError> {
    let contract = state.prime_contracts.get(inv.contract).ok_or_else(|| EngineError::not_found("the invoice's prime contract is missing"))?;
    let earlier: Vec<&[InvoiceLine]> = state
        .owner_invoices
        .iter()
        .filter(|o| o.contract == inv.contract && o.deleted.is_none() && o.status.billed() && o.number < inv.number && o.id != inv.id)
        .map(|o| o.lines.as_slice())
        .collect();
    summarize(billable_rows(state, contract), &earlier, &inv.lines, contract.retainage.unwrap_or(Percent::ZERO))
}

/// Works out a commitment invoice.
pub fn commitment_invoice_summary(state: &CompanyState, inv: &CommitmentInvoice) -> Result<InvoiceSummary, EngineError> {
    let commitment = state.commitments.get(inv.commitment).ok_or_else(|| EngineError::not_found("the invoice's commitment is missing"))?;
    let earlier: Vec<&[InvoiceLine]> = state
        .commitment_invoices
        .iter()
        .filter(|o| o.commitment == inv.commitment && o.deleted.is_none() && o.status.billed() && o.number < inv.number && o.id != inv.id)
        .map(|o| o.lines.as_slice())
        .collect();
    summarize(commitment_billable_rows(state, commitment), &earlier, &inv.lines, commitment.retainage.unwrap_or(Percent::ZERO))
}

/// How far billing on a contract or commitment has got.
#[derive(Clone, Copy, Debug)]
pub struct ContractSummary {
    pub original: Money,
    pub changes: Money,
    pub revised: Money,
    pub billed: Money,
    pub retainage_held: Money,
    /// Net amounts of paid invoices.
    pub paid: Money,
    pub balance_to_finish: Money,
}

fn billing_summary(
    original: Money,
    changes: Money,
    rate: Percent,
    invoices: Vec<(InvoiceStatus, InvoiceSummary)>,
) -> Result<ContractSummary, EngineError> {
    let revised = original.checked_add(changes).ok_or_else(overflow)?;
    let mut billed = Money::ZERO;
    let mut paid = Money::ZERO;
    for (status, s) in &invoices {
        billed = billed.checked_add(s.this_period).ok_or_else(overflow)?;
        if *status == InvoiceStatus::Paid {
            paid = paid.checked_add(s.net_due).ok_or_else(overflow)?;
        }
    }
    let retainage_held = billed.percent(rate).ok_or_else(overflow)?;
    let balance_to_finish = revised.checked_sub(billed).ok_or_else(overflow)?;
    Ok(ContractSummary { original, changes, revised, billed, retainage_held, paid, balance_to_finish })
}

pub fn contract_summary(state: &CompanyState, contract: &PrimeContract) -> Result<ContractSummary, EngineError> {
    let original = Money::sum(contract.lines.iter().map(|l| l.amount)).ok_or_else(overflow)?;
    let mut totals = Vec::new();
    for c in state.prime_changes.iter().filter(|c| c.contract == contract.id && c.deleted.is_none() && c.status == ChangeStatus::Approved) {
        totals.push(Money::sum(c.lines.iter().map(|l| l.amount)).ok_or_else(overflow)?);
    }
    let changes = Money::sum(totals).ok_or_else(overflow)?;
    let mut invoices = Vec::new();
    for inv in state.owner_invoices.iter().filter(|o| o.contract == contract.id && o.deleted.is_none() && o.status.billed()) {
        invoices.push((inv.status, invoice_summary(state, inv)?));
    }
    billing_summary(original, changes, contract.retainage.unwrap_or(Percent::ZERO), invoices)
}

pub fn commitment_billing(state: &CompanyState, c: &Commitment) -> Result<ContractSummary, EngineError> {
    let original = c.total().ok_or_else(overflow)?;
    let mut totals = Vec::new();
    for x in state.commitment_changes.iter().filter(|x| x.commitment == c.id && x.deleted.is_none() && x.status == ChangeStatus::Approved) {
        totals.push(Money::sum(x.lines.iter().map(|l| l.amount)).ok_or_else(overflow)?);
    }
    let changes = Money::sum(totals).ok_or_else(overflow)?;
    let mut invoices = Vec::new();
    for inv in state.commitment_invoices.iter().filter(|o| o.commitment == c.id && o.deleted.is_none() && o.status.billed()) {
        invoices.push((inv.status, commitment_invoice_summary(state, inv)?));
    }
    billing_summary(original, changes, c.retainage.unwrap_or(Percent::ZERO), invoices)
}

/// Folders nest at most this deep.
pub const MAX_FOLDER_DEPTH: usize = 12;

/// Whether the actor may open a folder: Documents Read access, and for a private folder (or one
/// inside a private folder) being listed on it or a Documents admin.
pub fn can_see_folder(state: &CompanyState, actor: Actor, folder: &Folder) -> bool {
    let lvl = level(state, actor, folder.project, Tool::Documents);
    if lvl < Level::Read {
        return false;
    }
    if lvl == Level::Admin {
        return true;
    }
    let mut at = Some(folder);
    for _ in 0..=MAX_FOLDER_DEPTH {
        let Some(f) = at else { return true };
        if f.deleted.is_some() {
            return false;
        }
        if f.private && !actor.person().is_some_and(|p| f.access.contains(&p)) {
            return false;
        }
        at = f.parent.and_then(|id| state.folders.get(id));
    }
    false
}

/// Whether the actor may open a document: its folder must be visible to them.
pub fn can_see_document(state: &CompanyState, actor: Actor, doc: &Document) -> bool {
    if level(state, actor, doc.project, Tool::Documents) < Level::Read {
        return false;
    }
    match doc.folder {
        None => true,
        Some(f) => state.folders.get(f).is_some_and(|f| can_see_folder(state, actor, f)),
    }
}
