//! Owner invoices (against the prime contract) and commitment invoices (from a subcontractor or
//! supplier, against a commitment). The shared commands live in [`crate::invoicedoc`].

use buildkiosk_core::{Id, Percent};
use buildkiosk_model::finance::{ChangeStatus, Commitment, CommitmentInvoice, InvoiceStatus, OwnerInvoice, PrimeContract};
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, Table, Tool};

use super::CommandSpec;
use crate::ctx::{Read, in_project};
use crate::invoicedoc::InvoiceDoc;
use crate::view::{self, InvoiceSummary};
use crate::{Ctx, EngineError, Params, Result};

const OWNER: [CommandSpec; 11] =
    crate::invoice_commands!("owner_invoice", "owner invoice", Tool::Invoicing, OwnerInvoice, "contract: prime contract id (must be approved)");
const SUB: [CommandSpec; 11] = crate::invoice_commands!(
    "commitment_invoice",
    "commitment invoice",
    Tool::Invoicing,
    CommitmentInvoice,
    "commitment: commitment id (must be approved)"
);

pub const COMMANDS: &[CommandSpec] = &[
    OWNER[0], OWNER[1], OWNER[2], OWNER[3], OWNER[4], OWNER[5], OWNER[6], OWNER[7], OWNER[8], OWNER[9], OWNER[10], SUB[0], SUB[1], SUB[2], SUB[3],
    SUB[4], SUB[5], SUB[6], SUB[7], SUB[8], SUB[9], SUB[10],
];

fn next_number<T: InvoiceDoc>(state: &CompanyState, project: Id<Project>) -> u32 {
    T::table(state).iter().filter(|i| i.project() == project).map(|i| i.number()).max().unwrap_or(0).saturating_add(1)
}

impl InvoiceDoc for OwnerInvoice {
    const TOOL: Tool = Tool::Invoicing;
    const PARENT: &'static str = "contract";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.owner_invoices
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.owner_invoices
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>, parent: u64) -> Self {
        let stamp = ctx.stamp();
        OwnerInvoice {
            id: ctx.state.owner_invoices.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            contract: Id::new(parent),
            period_end: ctx.today(),
            status: InvoiceStatus::Draft,
            notes: String::new(),
            lines: Vec::new(),
            submitted: None,
            approved: None,
            paid_on: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        }
    }
    fn check_parent(ctx: &Ctx<'_>, project: Id<Project>, p: &Params<'_>) -> Result<u64> {
        let id: Id<PrimeContract> = p.id("contract")?;
        let contract = in_project(&ctx.state.prime_contracts, project, id, "prime contract")?;
        if contract.deleted.is_some() || contract.status != ChangeStatus::Approved {
            return Err(EngineError::conflict("only an approved prime contract can be invoiced"));
        }
        Ok(id.raw())
    }
    fn parent_raw(&self) -> u64 {
        self.contract.raw()
    }
    fn summary(&self, state: &CompanyState) -> Result<InvoiceSummary> {
        view::invoice_summary(state, self)
    }
    crate::invoice_doc_fields!();
}

impl InvoiceDoc for CommitmentInvoice {
    const TOOL: Tool = Tool::Invoicing;
    const PARENT: &'static str = "commitment";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.commitment_invoices
    }
    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.commitment_invoices
    }
    fn blank(ctx: &Ctx<'_>, project: Id<Project>, parent: u64) -> Self {
        let stamp = ctx.stamp();
        CommitmentInvoice {
            id: ctx.state.commitment_invoices.peek_next(),
            project,
            number: next_number::<Self>(ctx.state, project),
            commitment: Id::new(parent),
            period_end: ctx.today(),
            status: InvoiceStatus::Draft,
            notes: String::new(),
            lines: Vec::new(),
            submitted: None,
            approved: None,
            paid_on: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        }
    }
    fn check_parent(ctx: &Ctx<'_>, project: Id<Project>, p: &Params<'_>) -> Result<u64> {
        let id: Id<Commitment> = p.id("commitment")?;
        let c = in_project(&ctx.state.commitments, project, id, "commitment")?;
        if c.deleted.is_some() || !c.status.counts() {
            return Err(EngineError::conflict("only an approved commitment can be invoiced"));
        }
        Ok(id.raw())
    }
    fn parent_raw(&self) -> u64 {
        self.commitment.raw()
    }
    fn summary(&self, state: &CompanyState) -> Result<InvoiceSummary> {
        view::commitment_invoice_summary(state, self)
    }
    crate::invoice_doc_fields!();
}

/// A retainage percentage between 0 and 100 (shared by contracts and commitments).
pub(crate) fn retainage(p: &Params<'_>) -> Result<Option<Percent>> {
    let r = p.percent("retainage")?;
    if r.is_some_and(|r| r.hundredths() < 0 || r.hundredths() > Percent::HUNDRED.hundredths()) {
        return Err(p.bad("retainage", "must be between 0 and 100"));
    }
    Ok(r)
}
