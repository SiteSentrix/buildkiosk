//! Signing a contract in BuildKiosk: the other party opens a link, reads the contract and signs it by typing their name.
//!
//! Sending a commitment, prime contract or prime change order for signature (`*.send_for_signature`) can now carry an
//! email address; that makes a link. The server works the link out from its secret (nothing stored can open it), emails
//! it, and runs the `contract.portal_*` commands below on the signer's behalf. They are system-only, so a person, API
//! token or agent can't call them, and each re-checks the link (the contract, the generation the server verified, not
//! past its last day). Every failure looks the same.
//!
//! What a signature records: the typed name, the day and moment, that it came through the link, and the moment the
//! contract was last changed (the terms on the page). The signer must send the terms they were shown: if the contract
//! changed after the page opened, the signature is refused and the page is read again. Changing the lines of a contract
//! that is out for signature or signed clears the signature, as before.

use buildkiosk_core::{Id, Money, Percent, text};
use buildkiosk_model::finance::{
    Commitment, CommitmentStatus, ContractSignature, CostLine, MAX_REMINDERS, PrimeChange, PrimeContract, SignatureLink, SignatureStatus,
};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "contract.link_sent",
        "Record that a signing link was emailed",
        Tool::Commitments,
        Access::System,
        "kind: commitment|prime_contract|prime_change; id: its id; generation: the link's generation that was sent",
        link_sent,
    ),
    CommandSpec::mutate(
        "contract.reminder_sent",
        "Record that a signing reminder was emailed",
        Tool::Commitments,
        Access::System,
        "kind: commitment|prime_contract|prime_change; id: its id; generation: the link's generation the reminder was for (at most three reminders a link)",
        reminder_sent,
    ),
    CommandSpec::query(
        "contract.portal_view",
        "The contract a signer reads",
        Tool::Commitments,
        Access::System,
        "kind, id, generation (what the signer sees through the link; run by the server after it checked the link)",
        portal_view,
    ),
    CommandSpec::mutate(
        "contract.portal_sign",
        "Sign a contract through its link",
        Tool::Commitments,
        Access::System,
        "kind, id, generation; name: the signer's full name; agree: true; terms: the contract version the page showed (refused if it changed since)",
        portal_sign,
    ),
    CommandSpec::mutate(
        "contract.portal_decline",
        "Decline to sign through the link",
        Tool::Commitments,
        Access::System,
        "kind, id, generation; note?: why",
        portal_decline,
    ),
];

/// Which kind of contract a link is for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commitment,
    PrimeContract,
    PrimeChange,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        match s.trim() {
            "commitment" => Some(Kind::Commitment),
            "prime_contract" => Some(Kind::PrimeContract),
            "prime_change" => Some(Kind::PrimeChange),
            _ => None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Kind::Commitment => "commitment",
            Kind::PrimeContract => "prime_contract",
            Kind::PrimeChange => "prime_change",
        }
    }

    pub fn tool(self) -> Tool {
        match self {
            Kind::Commitment => Tool::Commitments,
            Kind::PrimeContract => Tool::PrimeContracts,
            Kind::PrimeChange => Tool::ChangeOrders,
        }
    }

    pub fn path(self) -> &'static str {
        match self {
            Kind::Commitment => "commitments",
            Kind::PrimeContract => "prime-contracts",
            Kind::PrimeChange => "prime-change-orders",
        }
    }
}

fn bad_link() -> EngineError {
    EngineError::not_found("this link is not valid")
}

/// A new link for a contract being sent: `email` makes one, `expires` is its last day. The generation comes from the
/// clock (seconds), so a link from an earlier sending, even after the signature was cleared, never opens a later one.
pub(super) fn new_link(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<SignatureLink>> {
    let Some(email) = p.email("email")? else { return Ok(None) };
    let expires = p.date("expires")?;
    if expires.is_some_and(|d| d < ctx.today()) {
        return Err(p.bad("expires", "is in the past"));
    }
    let stamp = ctx.stamp();
    let generation = u32::try_from((stamp.at.millis() / 1000).rem_euclid(4_000_000_000)).unwrap_or(1);
    Ok(Some(SignatureLink { email, generation, expires, created: stamp, sent: None, reminders: 0, reminded: None }))
}

/// What a signer reads: one contract's terms.
struct Doc {
    project: Id<Project>,
    label: String,
    title: String,
    description: String,
    party: String,
    lines: Vec<CostLine>,
    retainage: Option<Percent>,
    open: bool,
    signature: ContractSignature,
    updated: buildkiosk_model::common::Stamp,
}

fn load_doc(state: &buildkiosk_model::CompanyState, kind: Kind, id: u64) -> Result<Doc> {
    let name = |v: Option<Id<buildkiosk_model::directory::Vendor>>| v.map(|v| crate::view::vendor_name(state, v)).unwrap_or_default();
    match kind {
        Kind::Commitment => {
            let c: &Commitment = state.commitments.get(Id::new(id)).filter(|c| c.deleted.is_none()).ok_or_else(bad_link)?;
            Ok(Doc {
                project: c.project,
                label: c.label(),
                title: c.title.clone(),
                description: c.description.clone(),
                party: name(c.vendor),
                lines: c.lines.clone(),
                retainage: c.retainage,
                open: !matches!(c.status, CommitmentStatus::Complete | CommitmentStatus::Void),
                signature: c.signature.clone(),
                updated: c.updated,
            })
        }
        Kind::PrimeContract => {
            let c: &PrimeContract = state.prime_contracts.get(Id::new(id)).filter(|c| c.deleted.is_none()).ok_or_else(bad_link)?;
            Ok(Doc {
                project: c.project,
                label: format!("Prime contract #{}", c.number),
                title: c.title.clone(),
                description: c.description.clone(),
                party: name(c.owner),
                lines: c.lines.clone(),
                retainage: c.retainage,
                open: c.status != buildkiosk_model::finance::ChangeStatus::Void,
                signature: c.signature.clone(),
                updated: c.updated,
            })
        }
        Kind::PrimeChange => {
            let c: &PrimeChange = state.prime_changes.get(Id::new(id)).filter(|c| c.deleted.is_none()).ok_or_else(bad_link)?;
            let owner = state.prime_contracts.get(c.contract).and_then(|k| k.owner);
            Ok(Doc {
                project: c.project,
                label: format!("Prime change order #{}", c.number),
                title: c.title.clone(),
                description: c.description.clone(),
                party: name(owner),
                lines: c.lines.clone(),
                retainage: None,
                open: c.status != buildkiosk_model::finance::ChangeStatus::Void,
                signature: c.signature.clone(),
                updated: c.updated,
            })
        }
    }
}

/// The contract and link a checked link names.
fn linked(state: &buildkiosk_model::CompanyState, p: &Params<'_>, today: buildkiosk_core::Date) -> Result<(Kind, u64, Doc)> {
    let kind = p.req_line("kind", 20).ok().and_then(|k| Kind::parse(&k)).ok_or_else(bad_link)?;
    let id: u64 = p.id::<Commitment>("id").map_err(|_| bad_link())?.raw();
    let generation: u32 = p.int("generation", 0..=4_000_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(bad_link)?;
    let doc = load_doc(state, kind, id)?;
    let ok = doc.signature.link.as_ref().is_some_and(|l| l.generation == generation && l.active(today));
    if !ok {
        return Err(bad_link());
    }
    Ok((kind, id, doc))
}

fn link_sent(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let kind = p
        .req_line("kind", 20)
        .ok()
        .and_then(|k| Kind::parse(&k))
        .ok_or_else(|| p.bad("kind", "must be commitment, prime_contract or prime_change"))?;
    let id: u64 = p.id::<Commitment>("id")?.raw();
    let generation: u32 =
        p.int("generation", 0..=4_000_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("generation", "is required"))?;
    let doc = load_doc(ctx.state, kind, id)?;
    let mut sig = doc.signature;
    let stamp = ctx.stamp();
    // A link sent again while the first email was on its way stays unsent, so the newer one goes out.
    if let Some(link) = sig.link.as_mut().filter(|l| l.generation == generation) {
        link.sent = Some(stamp);
        store_signature_keep(ctx, kind, id, sig);
    }
    Ok(json!({ "id": id }))
}

fn reminder_sent(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let kind = p
        .req_line("kind", 20)
        .ok()
        .and_then(|k| Kind::parse(&k))
        .ok_or_else(|| p.bad("kind", "must be commitment, prime_contract or prime_change"))?;
    let id: u64 = p.id::<Commitment>("id")?.raw();
    let generation: u32 =
        p.int("generation", 0..=4_000_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("generation", "is required"))?;
    let doc = load_doc(ctx.state, kind, id)?;
    let mut sig = doc.signature;
    let stamp = ctx.stamp();
    // Only for the live link of a contract still out for signature, and never past the most reminders.
    if sig.status == SignatureStatus::Sent
        && let Some(link) = sig.link.as_mut().filter(|l| l.generation == generation && l.reminders < MAX_REMINDERS)
    {
        link.reminders += 1;
        link.reminded = Some(stamp);
        store_signature_keep(ctx, kind, id, sig);
    }
    Ok(json!({ "id": id }))
}

/// Stores a signature without counting as a change to the contract's terms (recording that an email went out does not
/// change what was agreed).
fn store_signature_keep(ctx: &mut Ctx<'_>, kind: Kind, id: u64, sig: ContractSignature) {
    // The contract was found a moment ago, so the update always lands.
    let _ = match kind {
        Kind::Commitment => ctx.state.commitments.update(Id::new(id), |x| x.signature = sig),
        Kind::PrimeContract => ctx.state.prime_contracts.update(Id::new(id), |x| x.signature = sig),
        Kind::PrimeChange => ctx.state.prime_changes.update(Id::new(id), |x| x.signature = sig),
    };
}

fn portal_view(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let (kind, id, doc) = linked(v.state, p, v.today())?;
    let total: Option<Money> = Money::sum(doc.lines.iter().map(|l| l.amount));
    let lines: Vec<Value> = doc
        .lines
        .iter()
        .map(|l| {
            let code = v.state.cost_codes.get(l.cost_code).map(|c| c.display()).unwrap_or_default();
            json!({ "cost_code": code, "description": l.description, "amount": l.amount })
        })
        .collect();
    let project = v.state.projects.get(doc.project).map(|x| x.name.clone()).unwrap_or_default();
    let own = v.state.company().map(|c| c.name.clone()).unwrap_or_default();
    let sig = &doc.signature;
    Ok(json!({
        "kind": kind.key(),
        "id": id,
        "label": doc.label,
        "title": doc.title,
        "description": doc.description,
        "project": project,
        "company": own,
        "party": doc.party,
        "lines": lines,
        "total": total,
        "retainage": doc.retainage,
        "open": doc.open,
        "status": sig.status,
        "signer": sig.signer,
        "signed_on": sig.signed_on,
        "note": sig.note,
        "terms": doc.updated.at.millis(),
        "email": sig.link.as_ref().map(|l| l.email.clone()),
    }))
}

/// Tells the people who run the tool on the project something happened on the contract.
fn tell_team(ctx: &mut Ctx<'_>, kind: Kind, id: u64, doc: &Doc, what: &str) {
    let people: Vec<_> =
        ctx.state.people.iter().filter(|x| perm::project_level(ctx.state, x.id, doc.project, kind.tool()) >= Level::Standard).map(|x| x.id).collect();
    let text = format!("{what} on {}: {}", doc.label, doc.title);
    super::notification::notify(ctx, &people, doc.project, &text, &format!("/projects/{}/{}/{id}", doc.project, kind.path()));
}

fn portal_sign(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (kind, id, doc) = linked(ctx.state, p, ctx.today())?;
    if !doc.open {
        return Err(EngineError::conflict("this contract can't be signed now"));
    }
    match doc.signature.status {
        SignatureStatus::Sent => {}
        SignatureStatus::Signed => return Err(EngineError::conflict("this contract is already signed")),
        _ => return Err(EngineError::conflict("this contract is not out for signature")),
    }
    if doc.lines.is_empty() {
        return Err(EngineError::conflict("this contract has no lines to sign for"));
    }
    if !p.flag_or("agree", false)? {
        return Err(p.bad("agree", "tick the box to say you have read the contract and agree to sign it"));
    }
    let name = p.req_line("name", text::SHORT)?;
    if name.chars().filter(|c| c.is_alphabetic()).count() < 3 {
        return Err(p.bad("name", "type your full name"));
    }
    let seen = p.int("terms", 0..=i64::MAX)?.ok_or_else(|| p.bad("terms", "is required"))?;
    if seen != doc.updated.at.millis() {
        return Err(EngineError::conflict("the contract was changed after you opened it; read it again before signing"));
    }
    let stamp = ctx.stamp();
    let sig = ContractSignature {
        status: SignatureStatus::Signed,
        signed_on: Some(ctx.today()),
        signer: name,
        signed_at: Some(stamp),
        signed_terms: Some(doc.updated),
        note: String::new(),
        ..doc.signature.clone()
    };
    // Signing does not change what was agreed, so the contract's own "last changed" moment stays the same.
    store_signature_keep(ctx, kind, id, sig);
    tell_team(ctx, kind, id, &doc, &format!("{} signed", if doc.party.is_empty() { "The other party".to_string() } else { doc.party.clone() }));
    Ok(json!({ "id": id, "signed_on": ctx.today() }))
}

fn portal_decline(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let (kind, id, doc) = linked(ctx.state, p, ctx.today())?;
    if doc.signature.status != SignatureStatus::Sent {
        return Err(EngineError::conflict("this contract is not out for signature"));
    }
    let sig = ContractSignature { status: SignatureStatus::Declined, note: p.line_or_empty("note", text::SHORT)?, ..doc.signature.clone() };
    store_signature_keep(ctx, kind, id, sig);
    tell_team(
        ctx,
        kind,
        id,
        &doc,
        &format!("{} declined to sign", if doc.party.is_empty() { "The other party".to_string() } else { doc.party.clone() }),
    );
    Ok(json!({ "id": id }))
}

/// The contract a team page can show the signing link for, if it has a live one.
pub fn live_link(state: &buildkiosk_model::CompanyState, kind: Kind, id: u64, today: buildkiosk_core::Date) -> Option<SignatureLink> {
    load_doc(state, kind, id).ok().and_then(|d| d.signature.link).filter(|l| l.active(today))
}
