//! Bidding: send a package of work out to invited companies, record the bids that come back,
//! compare them, and award. Awarding can start a draft commitment for the winner.
//!
//! Bidders have no accounts: a person on the team enters each bid as it arrives, with the unit prices
//! of the bid form, the alternates, what the bid covers of each scope item (and what it would cost to
//! bring it up to the full scope), the bid bond and the files the company sent. Addenda go out while the
//! package is out for bid and each company's acknowledgement is recorded.

use buildkiosk_core::{Date, Id, Money, Percent, text};
use buildkiosk_model::bidding::{
    Acknowledgement, Addendum, Bid, BidAlternate, BidInvitation, BidItem, BidLine, BidPackage, BidScore, BidStatus, BondReceipt, Criterion,
    ScopeAnswer, ScopeResponse,
};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::finance::{Commitment, CommitmentKind, CommitmentLine, CommitmentStatus};
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::usable_code;
use super::estimate::parse_quantity;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "bid_package.create",
        "Create bid package",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project: project id; title: text; scope?: text; due?: YYYY-MM-DD; cost_code?: cost code id; invite?: [company ids]; \
         scope_items?: [{text}] (what each bid should cover, to compare them on the same scope); form_items?: [{description, unit?, quantity}] (the lines bidders price); \
         alternates?: [{name}] (options bidders price apart); bond_percent?: share of the bid a bid bond must cover (blank: none). The three lists can't change once a bid is in",
        create,
    ),
    CommandSpec::mutate(
        "bid_package.update",
        "Edit bid package",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; title?, scope?, due?, cost_code?, invite?, scope_items?, form_items?, alternates?, bond_percent? (the invited list is replaced; not once awarded)",
        update,
    ),
    CommandSpec::mutate(
        "bid_package.open",
        "Send out for bid",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids (needs at least one invited company)",
        open,
    ),
    CommandSpec::mutate(
        "bid_package.invite",
        "Send bid link",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor: company id (added to the invited list; must be prequalified when the company requires it); email: where the link is sent; expires?: YYYY-MM-DD (last day the link works; blank: until the package closes). \
         The server emails the link; the command returns the invitation number",
        invite,
    ),
    CommandSpec::mutate(
        "bid_package.resend_invite",
        "Send bid link again",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; number: invitation number; email?: a new address; expires?: YYYY-MM-DD (the earlier link stops working and a new one is emailed)",
        resend_invite,
    ),
    CommandSpec::mutate(
        "bid_package.revoke_invite",
        "Cancel bid link",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; number: invitation number (the link stops working at once; bids already received stay)",
        revoke_invite,
    ),
    CommandSpec::mutate(
        "bid_package.invite_sent",
        "Record bid link emailed",
        Tool::Bidding,
        Access::System,
        "project, id: ids; number: invitation number; generation: the link that went out (the server records this after it emails a link)",
        invite_sent,
    ),
    CommandSpec::query(
        "bid_package.invitations",
        "List bid links",
        Tool::Bidding,
        Access::Project(Level::Read),
        "project, id: ids (each invitation with its company, address, state and expiry; never the link itself)",
        invitations,
    ),
    CommandSpec::mutate(
        "bid_package.unseal",
        "Open sealed bids",
        Tool::Bidding,
        Access::Project(Level::Admin),
        "project, id: ids (bids that companies sent through their links were kept unseen; this shows them to the team now instead of when bidding closes)",
        unseal,
    ),
    CommandSpec::query(
        "bid_package.portal_view",
        "Show a bidder their package",
        Tool::Bidding,
        Access::System,
        "project, id: ids; invitation: link number; generation: the link's generation (what a company sees: the package, its form, its own bid; never other companies; run by the server once a link is checked)",
        portal_view,
    ),
    CommandSpec::mutate(
        "bid_package.portal_submit",
        "Bidder sends a bid",
        Tool::Bidding,
        Access::System,
        "project, id, invitation, generation as portal_view; amount?, lines?, alternates?, scope?, notes? as record_bid (replaces the company's earlier bid; the bond receipt and files are kept)",
        portal_submit,
    ),
    CommandSpec::mutate(
        "bid_package.portal_withdraw",
        "Bidder withdraws a bid",
        Tool::Bidding,
        Access::System,
        "project, id, invitation, generation as portal_view",
        portal_withdraw,
    ),
    CommandSpec::mutate(
        "bid_package.portal_ack",
        "Bidder acknowledges an addendum",
        Tool::Bidding,
        Access::System,
        "project, id, invitation, generation as portal_view; number: addendum number",
        portal_ack,
    ),
    CommandSpec::mutate(
        "bid_package.record_bid",
        "Record bid",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor: company id; amount?: amount (above zero; not needed when lines are given); received?: YYYY-MM-DD (default today); notes?: text; \
         lines?: [{item: bid form line from 0, unit_price}] (price every line: the amount is their total); alternates?: [{index: from 0, amount (negative for a deduct)}]; \
         scope?: [{item: from 0, answer: included|excluded|unclear, adjustment?: amount to add to bring the bid up to the full scope}]; \
         bond_received?: bool; bond_amount?: amount; bond_note?: text (replaces that company's earlier bid, keeping its files; an uninvited company is added to the invited list)",
        record_bid,
    ),
    CommandSpec::mutate(
        "bid_package.addendum",
        "Issue addendum",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; title: text; body?: text (a change to the package while it is out for bid; bidders are recorded as having acknowledged it)",
        addendum,
    ),
    CommandSpec::mutate(
        "bid_package.acknowledge",
        "Record addendum acknowledged",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor: company id; number: addendum number",
        acknowledge,
    ),
    CommandSpec::mutate(
        "bid_package.attach",
        "Attach file to bid package",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor?: company id (to file it with that company's bid instead); sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        attach,
    ),
    CommandSpec::mutate(
        "bid_package.detach",
        "Remove file from bid package",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor?: company id (a file of that company's bid); sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "bid_package.set_criteria",
        "Set bid scoring criteria",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; text: one criterion per line as `Name, weight` (weights add up to 100; up to 10); add `, price` to the one that is scored from the leveled amounts (the lowest bid gets the top score). Blank text means price alone. Changing them clears the scores given",
        set_criteria,
    ),
    CommandSpec::mutate(
        "bid_package.score",
        "Score a bid",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor: company id; scores: {criterion number from 1: 0 to 10} (the price criterion is worked out, not scored)",
        score,
    ),
    CommandSpec::query(
        "bid_package.level",
        "Level the bids",
        Tool::Bidding,
        Access::Project(Level::Read),
        "project, id: ids (each standing bid with its adjustments, leveled amount, bond, addenda and scope answers, cheapest leveled first)",
        level,
    ),
    CommandSpec::mutate(
        "bid_package.withdraw_bid",
        "Withdraw bid",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; vendor: company id",
        withdraw_bid,
    ),
    CommandSpec::mutate("bid_package.close", "Close bidding", Tool::Bidding, Access::Project(Level::Standard), "project, id: ids", close),
    CommandSpec::mutate(
        "bid_package.reopen",
        "Reopen bidding",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids (back to out for bid; not once awarded)",
        reopen,
    ),
    CommandSpec::mutate(
        "bid_package.award",
        "Award package",
        Tool::Bidding,
        Access::Project(Level::Admin),
        "project, id: ids; vendor: company id (must have a standing bid); commitment?: bool (start a draft commitment for the bid amount; needs the package's cost code)",
        award,
    ),
    CommandSpec::mutate("bid_package.delete", "Delete bid package", Tool::Bidding, Access::Project(Level::Admin), "project, id: ids", delete),
    CommandSpec::mutate("bid_package.restore", "Restore bid package", Tool::Bidding, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "bid_package.list",
        "List bid packages",
        Tool::Bidding,
        Access::Project(Level::Read),
        "project: project id; status?: draft|open|closed|awarded; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query(
        "bid_package.get",
        "Show bid package",
        Tool::Bidding,
        Access::Project(Level::Read),
        "project, id: ids (with the bids lowest first)",
        get,
    ),
];

const MAX_INVITED: usize = 100;
const MAX_ROWS: usize = 200;
const MAX_ADDENDA: usize = 100;
const MAX_INVITATIONS: usize = 300;
const MAX_ATTACHMENTS: usize = 50;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<BidPackage> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&ctx.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {id} not found in this project")));
    }
    Ok(b.clone())
}

fn invite_list(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Vec<Id<Vendor>>> {
    let mut ids = p.ids::<Vendor>("invite")?;
    ids.sort();
    ids.dedup();
    if ids.len() > MAX_INVITED {
        return Err(p.bad("invite", format!("lists more than {MAX_INVITED} companies")));
    }
    for id in &ids {
        ctx.check_vendor(*id)?;
    }
    Ok(ids)
}

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, b: &mut BidPackage) -> Result<()> {
    if p.present("title") {
        b.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("scope") {
        b.scope = p.text_or_empty("scope", text::LONG)?;
    }
    if p.present("due") {
        b.due = p.date("due")?;
    }
    if p.present("cost_code") {
        let c: Option<Id<CostCode>> = p.opt_id("cost_code")?;
        if let Some(c) = c {
            usable_code(ctx.state, b.project, c)?;
        }
        b.cost_code = c;
    }
    if p.present("invite") {
        b.invited = invite_list(ctx, p)?;
    }
    if p.present("sealed") {
        let sealed = p.flag_or("sealed", true)?;
        if sealed != b.sealed && !b.sealed_bids.is_empty() {
            return Err(EngineError::conflict("sealed bids are waiting; open them before changing this"));
        }
        b.sealed = sealed;
    }
    let structural = ["scope_items", "form_items", "alternates"].iter().any(|k| p.present(k));
    if structural && (b.bids.iter().any(|x| !x.withdrawn) || !b.sealed_bids.is_empty()) {
        return Err(EngineError::conflict(
            "bids are in, and they refer to the scope items, form lines and alternates by place; withdraw them before changing those",
        ));
    }
    if p.present("scope_items") {
        let mut out = Vec::new();
        for r in p.items("scope_items")? {
            let t = r.line_or_empty("text", text::SHORT)?;
            if !t.is_empty() {
                out.push(t);
            }
        }
        if out.len() > MAX_ROWS {
            return Err(p.bad("scope_items", format!("holds more than {MAX_ROWS} items")));
        }
        b.scope_items = out;
    }
    if p.present("alternates") {
        let mut out = Vec::new();
        for r in p.items("alternates")? {
            let t = r.line_or_empty("name", text::SHORT)?;
            if !t.is_empty() {
                out.push(t);
            }
        }
        if out.len() > MAX_ROWS {
            return Err(p.bad("alternates", format!("holds more than {MAX_ROWS} alternates")));
        }
        b.alternates = out;
    }
    if p.present("form_items") {
        let mut out = Vec::new();
        for r in p.items("form_items")? {
            let description = r.line_or_empty("description", text::SHORT)?;
            let q = r.line_or_empty("quantity", 30)?;
            if description.is_empty() && q.is_empty() {
                continue;
            }
            if description.is_empty() {
                return Err(r.bad("description", "is required on every line"));
            }
            let quantity_milli = if q.is_empty() {
                1000
            } else {
                parse_quantity(&q).filter(|v| *v > 0).ok_or_else(|| r.bad("quantity", format!("`{q}` is not a quantity above zero")))?
            };
            out.push(BidItem { description, unit: r.line_or_empty("unit", 20)?, quantity_milli });
        }
        if out.len() > MAX_ROWS {
            return Err(p.bad("form_items", format!("holds more than {MAX_ROWS} lines")));
        }
        b.form_items = out;
    }
    if p.present("bond_percent") {
        b.bond_required = match p.percent("bond_percent")? {
            Some(x) if x <= Percent::ZERO || x > Percent::HUNDRED => return Err(p.bad("bond_percent", "must be above 0 and at most 100")),
            other => other,
        };
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.bid_packages.iter().filter(|b| b.project == project).map(|b| b.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut b = BidPackage {
        id: ctx.state.bid_packages.peek_next(),
        project,
        number,
        title: String::new(),
        scope: String::new(),
        due: None,
        cost_code: None,
        status: BidStatus::Draft,
        invited: Vec::new(),
        scope_items: Vec::new(),
        form_items: Vec::new(),
        alternates: Vec::new(),
        bond_required: None,
        criteria: Vec::new(),
        addenda: Vec::new(),
        acknowledgements: Vec::new(),
        attachments: Vec::new(),
        bids: Vec::new(),
        sealed: p.flag_or("sealed", true)?,
        sealed_bids: Vec::new(),
        invitations: Vec::new(),
        awarded_to: None,
        commitment: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut b)?;
    if b.title.is_empty() {
        return Err(p.bad("title", "is required"));
    }
    let id = ctx.state.bid_packages.alloc();
    b.id = id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if b.status == BidStatus::Awarded {
        return Err(EngineError::conflict(format!("{} is awarded", b.label())));
    }
    apply(ctx, p, &mut b)?;
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn set_status(ctx: &mut Ctx<'_>, p: &Params<'_>, from: &[BidStatus], to: BidStatus) -> Result<BidPackage> {
    let mut b = load(ctx, p)?;
    if !from.contains(&b.status) {
        return Err(EngineError::conflict(format!("{} is {}", b.label(), b.status)));
    }
    b.status = to;
    b.updated = ctx.stamp();
    ctx.state.bid_packages.put(b.clone());
    Ok(b)
}

fn open(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let b = load(ctx, p)?;
    if b.invited.is_empty() {
        return Err(EngineError::conflict("invite at least one company first"));
    }
    let b = set_status(ctx, p, &[BidStatus::Draft], BidStatus::Open)?;
    Ok(json!({ "id": b.id }))
}

fn close(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = set_status(ctx, p, &[BidStatus::Open], BidStatus::Closed)?;
    // Bids sent through links were kept unseen; closing opens them.
    let opened = open_sealed(&mut b);
    ctx.state.bid_packages.put(b.clone());
    Ok(json!({ "id": b.id, "opened": opened }))
}

/// Moves the bids kept unseen into the bids the team sees, each replacing its company's earlier bid. Returns how many.
fn open_sealed(b: &mut BidPackage) -> usize {
    let sealed = std::mem::take(&mut b.sealed_bids);
    let n = sealed.len();
    for bid in sealed {
        b.bids.retain(|x| x.vendor != bid.vendor);
        b.bids.push(bid);
    }
    n
}

fn unseal(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    editable(&b)?;
    if b.sealed_bids.is_empty() {
        return Err(EngineError::conflict("no sealed bids are waiting"));
    }
    let opened = open_sealed(&mut b);
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "opened": opened }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let b = set_status(ctx, p, &[BidStatus::Closed], BidStatus::Open)?;
    Ok(json!({ "id": b.id }))
}

/// What a bid says about the package: the price, the lines, the alternates and the scope answers.
struct ParsedBid {
    amount: Money,
    lines: Vec<BidLine>,
    alternates: Vec<BidAlternate>,
    scope: Vec<ScopeResponse>,
}

fn parse_bid(p: &Params<'_>, b: &BidPackage) -> Result<ParsedBid> {
    // Unit prices for the bid form: every line, and the amount is their total.
    let mut lines: Vec<BidLine> = Vec::new();
    for r in p.items("lines")? {
        let item: u32 = r.int("item", 0..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| r.bad("item", "is required"))?;
        let unit_price = r.money("unit_price")?.ok_or_else(|| r.bad("unit_price", "is required"))?;
        if unit_price.is_negative() {
            return Err(r.bad("unit_price", "must not be negative"));
        }
        if b.form_items.get(item as usize).is_none() {
            return Err(r.bad("item", format!("the bid form has no line {item}")));
        }
        if lines.iter().any(|l| l.item == item) {
            return Err(r.bad("item", "is priced twice"));
        }
        lines.push(BidLine { item, unit_price });
    }
    lines.sort_by_key(|l| l.item);
    let amount = if lines.is_empty() {
        p.money("amount")?.ok_or_else(|| p.bad("amount", "is required"))?
    } else {
        if lines.len() != b.form_items.len() {
            return Err(p.bad("lines", format!("price every line of the bid form ({} of {} given)", lines.len(), b.form_items.len())));
        }
        let parts: Option<Vec<Money>> =
            lines.iter().map(|l| l.unit_price.checked_mul_int(b.form_items.get(l.item as usize)?.quantity_milli)?.checked_div_int(1000)).collect();
        parts.and_then(Money::sum).ok_or_else(|| p.bad("lines", "the total is too large"))?
    };
    if amount.is_negative() || amount.is_zero() {
        return Err(p.bad("amount", "must be above zero"));
    }
    let mut alternates: Vec<BidAlternate> = Vec::new();
    for r in p.items("alternates")? {
        let index: u32 = r.int("index", 0..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| r.bad("index", "is required"))?;
        if b.alternates.get(index as usize).is_none() {
            return Err(r.bad("index", format!("the package has no alternate {index}")));
        }
        if alternates.iter().any(|a| a.index == index) {
            return Err(r.bad("index", "is priced twice"));
        }
        alternates.push(BidAlternate { index, amount: r.money("amount")?.ok_or_else(|| r.bad("amount", "is required"))? });
    }
    alternates.sort_by_key(|a| a.index);
    let mut scope: Vec<ScopeResponse> = Vec::new();
    for r in p.items("scope")? {
        let item: u32 = r.int("item", 0..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| r.bad("item", "is required"))?;
        if b.scope_items.get(item as usize).is_none() {
            return Err(r.bad("item", format!("the package has no scope item {item}")));
        }
        if scope.iter().any(|s| s.item == item) {
            return Err(r.bad("item", "is answered twice"));
        }
        let answer = r.choice::<ScopeAnswer>("answer")?.ok_or_else(|| r.bad("answer", "is required: included, excluded or unclear"))?;
        let adjustment: Option<Money> = r.money("adjustment")?;
        if adjustment.is_some_and(Money::is_negative) {
            return Err(r.bad("adjustment", "must not be negative"));
        }
        scope.push(ScopeResponse { item, answer, adjustment });
    }
    scope.sort_by_key(|s| s.item);
    Ok(ParsedBid { amount, lines, alternates, scope })
}

fn record_bid(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if !matches!(b.status, BidStatus::Open | BidStatus::Closed) {
        return Err(EngineError::conflict(format!("{} is {}; bids can be recorded once it is out for bid", b.label(), b.status)));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    let ParsedBid { amount, lines, alternates, scope } = parse_bid(p, &b)?;
    let received: Date = p.date("received")?.unwrap_or_else(|| ctx.today());
    let notes = p.text_or_empty("notes", text::LONG)?;
    let bond = BondReceipt {
        received: p.flag_or("bond_received", false)?,
        amount: p.money("bond_amount")?,
        note: p.line_or_empty("bond_note", text::SHORT)?,
    };
    if bond.amount.is_some_and(Money::is_negative) {
        return Err(p.bad("bond_amount", "must not be negative"));
    }
    if !b.invited.contains(&vendor) {
        if b.invited.len() >= MAX_INVITED {
            return Err(EngineError::conflict(format!("a package invites at most {MAX_INVITED} companies")));
        }
        b.invited.push(vendor);
    }
    if let (Some(pct), true, Some(got)) = (b.bond_required, bond.received, bond.amount)
        && amount.percent(pct).is_some_and(|need| got < need)
    {
        return Err(
            p.bad("bond_amount", format!("is below the {pct} of the bid this package requires ({})", amount.percent(pct).unwrap_or_default()))
        );
    }
    // A new bid from the same company replaces the old one but keeps the files it sent and the scores given.
    let (files, scores) = b.bids.iter().find(|x| x.vendor == vendor).map(|x| (x.files.clone(), x.scores.clone())).unwrap_or_default();
    b.bids.retain(|x| x.vendor != vendor);
    b.bids.push(Bid { vendor, via: None, amount, received, notes, withdrawn: false, lines, alternates, scope, bond, scores, files });
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "amount": amount }))
}

fn editable(b: &BidPackage) -> Result<()> {
    if b.status == BidStatus::Awarded {
        return Err(EngineError::conflict(format!("{} is awarded", b.label())));
    }
    Ok(())
}

fn addendum(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    editable(&b)?;
    if b.addenda.len() >= MAX_ADDENDA {
        return Err(EngineError::conflict(format!("a package has at most {MAX_ADDENDA} addenda")));
    }
    let number = b.addenda.iter().map(|a| a.number).max().unwrap_or(0).saturating_add(1);
    b.addenda.push(Addendum { number, title: p.req_line("title", text::SHORT)?, body: p.text_or_empty("body", text::LONG)?, issued: ctx.stamp() });
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number }))
}

fn acknowledge(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    let number: u32 = p.int("number", 1..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("number", "is required"))?;
    if !b.addenda.iter().any(|a| a.number == number) {
        return Err(EngineError::not_found(format!("{} has no addendum {number}", b.label())));
    }
    if b.acknowledgements.iter().any(|a| a.vendor == vendor && a.number == number) {
        return Err(EngineError::conflict("that company already acknowledged this addendum"));
    }
    if !b.invited.contains(&vendor) {
        return Err(EngineError::conflict("that company was not invited"));
    }
    b.acknowledgements.push(Acknowledgement { vendor, number, stamp: ctx.stamp() });
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn invitation_number(p: &Params<'_>) -> Result<u32> {
    p.int("number", 1..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("number", "is required"))
}

fn expiry(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<Date>> {
    let day = p.date("expires")?;
    if day.is_some_and(|d| d < ctx.today()) {
        return Err(p.bad("expires", "is in the past"));
    }
    Ok(day)
}

fn invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if !matches!(b.status, BidStatus::Draft | BidStatus::Open) {
        return Err(EngineError::conflict(format!("{} is {}; links go out while it is a draft or out for bid", b.label(), b.status)));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
    super::prequal::check_prequalified(ctx, vendor)?;
    let email = p.email("email")?.ok_or_else(|| p.bad("email", "is required"))?;
    let expires = expiry(ctx, p)?;
    if b.invitations.iter().any(|i| i.vendor == vendor && i.email == email && i.revoked.is_none()) {
        return Err(EngineError::conflict("that address already has a link for this package; send it again instead"));
    }
    if b.invitations.len() >= MAX_INVITATIONS {
        return Err(EngineError::conflict(format!("a package can have at most {MAX_INVITATIONS} links")));
    }
    if !b.invited.contains(&vendor) {
        if b.invited.len() >= MAX_INVITED {
            return Err(EngineError::conflict(format!("a package can invite at most {MAX_INVITED} companies")));
        }
        b.invited.push(vendor);
    }
    let number = b.invitations.iter().map(|i| i.number).max().unwrap_or(0).checked_add(1).ok_or_else(|| EngineError::conflict("too many links"))?;
    b.invitations.push(BidInvitation { number, vendor, email, generation: 0, expires, created: ctx.stamp(), sent: None, revoked: None });
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number }))
}

fn resend_invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if !matches!(b.status, BidStatus::Draft | BidStatus::Open) {
        return Err(EngineError::conflict(format!("{} is {}; links go out while it is a draft or out for bid", b.label(), b.status)));
    }
    let number = invitation_number(p)?;
    let email = p.email("email")?;
    let expires_given = p.present("expires");
    let expires = expiry(ctx, p)?;
    let stamp = ctx.stamp();
    let inv =
        b.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this package")))?;
    if inv.revoked.is_some() {
        return Err(EngineError::conflict("that link was cancelled; send a new one with bid_package.invite"));
    }
    if let Some(email) = email {
        inv.email = email;
    }
    if expires_given {
        inv.expires = expires;
    }
    inv.generation = inv.generation.checked_add(1).ok_or_else(|| EngineError::conflict("that link was sent too many times"))?;
    inv.sent = None;
    let generation = inv.generation;
    b.updated = stamp;
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number, "generation": generation }))
}

fn revoke_invite(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    let number = invitation_number(p)?;
    let stamp = ctx.stamp();
    let inv =
        b.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this package")))?;
    if inv.revoked.is_some() {
        return Err(EngineError::conflict("that link is already cancelled"));
    }
    inv.revoked = Some(stamp);
    b.updated = stamp;
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number }))
}

fn invite_sent(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    let number = invitation_number(p)?;
    let generation: u32 =
        p.int("generation", 0..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("generation", "is required"))?;
    let stamp = ctx.stamp();
    let inv =
        b.invitations.iter_mut().find(|i| i.number == number).ok_or_else(|| EngineError::not_found(format!("no link {number} on this package")))?;
    // A link sent again while the first email was on its way stays unsent, so the newer one goes out.
    if inv.generation == generation && inv.revoked.is_none() {
        inv.sent = Some(stamp);
    }
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "number": number }))
}

fn invitations(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&v.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {id} not found in this project")));
    }
    let rows: Vec<Value> = b
        .invitations
        .iter()
        .map(|i| {
            let state = if i.revoked.is_some() {
                "cancelled"
            } else if i.expires.is_some_and(|e| v.today() > e) {
                "expired"
            } else if i.sent.is_some() {
                "sent"
            } else {
                "waiting to be sent"
            };
            json!({
                "number": i.number, "vendor": i.vendor, "company": crate::view::vendor_name(v.state, i.vendor), "email": i.email,
                "state": state, "expires": i.expires, "generation": i.generation,
            })
        })
        .collect();
    Ok(json!({ "invitations": rows }))
}

// ---- the bidder portal: commands the server runs (as the system) once it has checked a bidder's link ----

/// The one answer for every link that doesn't work, so nobody can tell a wrong number from a cancelled link.
fn bad_link() -> EngineError {
    EngineError::not_found("this link is not valid")
}

/// The invitation a checked link names: it must belong to the package, be the generation the server verified, and
/// not be cancelled or past its last day.
fn link<'a>(b: &'a BidPackage, p: &Params<'_>, today: Date) -> Result<&'a BidInvitation> {
    let number = invitation_arg(p, "invitation")?;
    let generation = p.int("generation", 0..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(bad_link)?;
    b.invitations.iter().find(|i| i.number == number && i.generation == generation && i.active(today)).ok_or_else(bad_link)
}

fn invitation_arg(p: &Params<'_>, key: &str) -> Result<u32> {
    p.int(key, 1..=1_000_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(bad_link)
}

/// Bidders can send, change or withdraw a bid only while the package is out for bid and its due day hasn't passed.
fn taking_bids(b: &BidPackage, today: Date) -> Result<()> {
    if b.status != BidStatus::Open {
        return Err(EngineError::conflict("this package is not taking bids"));
    }
    if b.due.is_some_and(|d| today > d) {
        return Err(EngineError::conflict("the due date for this package has passed; ask the team to reopen it"));
    }
    Ok(())
}

/// The company's own bid on the package, wherever it is kept.
fn own_bid(b: &BidPackage, vendor: Id<Vendor>) -> Option<&Bid> {
    b.sealed_bids.iter().chain(b.bids.iter()).find(|x| x.vendor == vendor && !x.withdrawn)
}

fn portal_package(v: &View<'_>, p: &Params<'_>) -> Result<BidPackage> {
    let project: Id<Project> = p.id("project").map_err(|_| bad_link())?;
    let id: Id<BidPackage> = p.id("id").map_err(|_| bad_link())?;
    let b = in_project(&v.state.bid_packages, project, id, "bid package").map_err(|_| bad_link())?;
    if b.deleted.is_some() {
        return Err(bad_link());
    }
    Ok(b.clone())
}

fn portal_view(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let b = portal_package(v, p)?;
    let today = v.today();
    let inv = link(&b, p, today)?;
    let vendor = inv.vendor;
    let bid = own_bid(&b, vendor);
    let documents: Vec<Value> = b.attachments.iter().map(|f| json!({ "name": f.name, "sha256": f.sha256, "size": f.size, "mime": f.mime })).collect();
    let addenda: Vec<Value> = b
        .addenda
        .iter()
        .map(|a| json!({ "number": a.number, "title": a.title, "body": a.body, "acknowledged": b.acknowledgements.iter().any(|x| x.vendor == vendor && x.number == a.number) }))
        .collect();
    let form: Vec<Value> =
        b.form_items.iter().map(|i| json!({ "description": i.description, "unit": i.unit, "quantity_milli": i.quantity_milli })).collect();
    Ok(json!({
        "package": { "number": b.number, "title": b.title, "scope": b.scope, "due": b.due, "status": b.status },
        "taking_bids": taking_bids(&b, today).is_ok(),
        "company": crate::view::vendor_name(v.state, vendor),
        "email": inv.email,
        "form_items": form,
        "alternates": b.alternates,
        "scope_items": b.scope_items,
        "bond_required": b.bond_required,
        "documents": documents,
        "addenda": addenda,
        "bid": bid.map(|x| json!({
            "amount": x.amount, "received": x.received, "notes": x.notes, "lines": x.lines, "alternates": x.alternates, "scope": x.scope,
            "files": x.files.iter().map(|f| json!({ "name": f.name, "sha256": f.sha256, "size": f.size })).collect::<Vec<_>>(),
        })),
    }))
}

fn portal_submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p).map_err(|_| bad_link())?;
    let today = ctx.today();
    let inv = link(&b, p, today)?.clone();
    taking_bids(&b, today)?;
    let vendor = inv.vendor;
    let ParsedBid { amount, lines, alternates, scope } = parse_bid(p, &b)?;
    let notes = p.text_or_empty("notes", text::LONG)?;
    // The team keeps the bond receipt, the files received and the scores; a revised bid inherits them.
    let (bond, files, scores) = b
        .sealed_bids
        .iter()
        .chain(b.bids.iter())
        .find(|x| x.vendor == vendor)
        .map(|x| (x.bond.clone(), x.files.clone(), x.scores.clone()))
        .unwrap_or_default();
    let bid = Bid { vendor, via: Some(inv.number), amount, received: today, notes, withdrawn: false, lines, alternates, scope, bond, scores, files };
    let kept = if b.sealed { &mut b.sealed_bids } else { &mut b.bids };
    kept.retain(|x| x.vendor != vendor);
    kept.push(bid);
    b.updated = ctx.stamp();
    let (id, number) = (b.id, b.number);
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "reference": format!("BP{number}-{}", inv.number), "amount": amount, "received": today }))
}

fn portal_withdraw(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p).map_err(|_| bad_link())?;
    let today = ctx.today();
    let vendor = link(&b, p, today)?.vendor;
    taking_bids(&b, today)?;
    let before = b.sealed_bids.len();
    b.sealed_bids.retain(|x| x.vendor != vendor);
    let mut done = b.sealed_bids.len() != before;
    if let Some(x) = b.bids.iter_mut().find(|x| x.vendor == vendor && !x.withdrawn) {
        x.withdrawn = true;
        done = true;
    }
    if !done {
        return Err(EngineError::conflict("you have no bid on this package"));
    }
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn portal_ack(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p).map_err(|_| bad_link())?;
    let today = ctx.today();
    let vendor = link(&b, p, today)?.vendor;
    if b.status != BidStatus::Open {
        return Err(EngineError::conflict("this package is not taking bids"));
    }
    let number: u32 = p.int("number", 1..=1_000)?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("number", "is required"))?;
    if !b.addenda.iter().any(|a| a.number == number) {
        return Err(EngineError::not_found(format!("this package has no addendum {number}")));
    }
    if !b.acknowledgements.iter().any(|a| a.vendor == vendor && a.number == number) {
        b.acknowledgements.push(Acknowledgement { vendor, number, stamp: ctx.stamp() });
        b.updated = ctx.stamp();
        ctx.state.bid_packages.put(b);
    }
    Ok(json!({ "number": number }))
}

/// A package as the team sees it: bids kept unseen are counted, not shown.
fn team_view(b: &BidPackage) -> Result<Value> {
    let mut out = serde_json::to_value(b).map_err(|e| EngineError::Internal { cmd: "bid_package".into(), msg: e.to_string() })?;
    if let Some(o) = out.as_object_mut() {
        o.remove("sealed_bids");
        let waiting: Vec<Value> = b.sealed_bids.iter().map(|x| json!({ "vendor": x.vendor, "received": x.received })).collect();
        o.insert("sealed_waiting".into(), Value::Array(waiting));
    }
    Ok(out)
}

fn file_params(p: &Params<'_>) -> Result<FileRef> {
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    Ok(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime })
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    let file = file_params(p)?;
    let files: &mut Vec<FileRef> = match p.opt_id::<Vendor>("vendor")? {
        Some(v) => {
            &mut b
                .bids
                .iter_mut()
                .find(|x| x.vendor == v && !x.withdrawn)
                .ok_or_else(|| EngineError::not_found("that company has no standing bid"))?
                .files
        }
        None => {
            editable(&b)?;
            &mut b.attachments
        }
    };
    if files.iter().any(|f| f.sha256 == file.sha256) {
        return Ok(json!({ "id": p.id::<BidPackage>("id")? }));
    }
    if files.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("that already has {MAX_ATTACHMENTS} files")));
    }
    files.push(file);
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let files: &mut Vec<FileRef> = match p.opt_id::<Vendor>("vendor")? {
        Some(v) => &mut b.bids.iter_mut().find(|x| x.vendor == v).ok_or_else(|| EngineError::not_found("that company has no bid"))?.files,
        None => {
            editable(&b)?;
            &mut b.attachments
        }
    };
    let before = files.len();
    files.retain(|f| f.sha256 != sha256);
    if files.len() == before {
        return Err(EngineError::not_found("that file is not attached"));
    }
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn level(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&v.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {id} not found in this project")));
    }
    let mut rows: Vec<&Bid> = b.standing();
    rows.sort_by_key(|x| (x.leveled().unwrap_or(x.amount), x.amount));
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|x| {
            let bond_ok = b.bond_required.map(|pct| x.bond.received && x.amount.percent(pct).is_none_or(|need| x.bond.amount.is_none_or(|got| got >= need)));
            json!({
                "vendor": x.vendor, "company": crate::view::vendor_name(v.state, x.vendor), "amount": x.amount, "adjustments": x.adjustments(), "leveled": x.leveled(),
                "bond_ok": bond_ok, "unacknowledged": b.unacknowledged(x.vendor), "scope": x.scope, "alternates": x.alternates, "lines": x.lines,
                "scores": x.scores, "weighted": b.weighted(x).map(|t| f64::from(t) / 100.0),
            })
        })
        .collect();
    Ok(json!({
        "package": b.id, "scope_items": b.scope_items, "form_items": b.form_items, "alternates": b.alternates, "bond_required": b.bond_required,
        "addenda": b.addenda.len(), "criteria": b.criteria, "rows": rows,
    }))
}

fn set_criteria(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    editable(&b)?;
    let mut out: Vec<Criterion> = Vec::new();
    for (n, raw) in p.text_or_empty("text", 2_000)?.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let at = |msg: String| p.bad("text", format!("line {}: {msg}", n + 1));
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        let name = text::line(cells.first().copied().unwrap_or(""), 80).map_err(|e| at(e.to_string()))?;
        if name.is_empty() {
            return Err(at("a criterion needs a name".into()));
        }
        let weight: u32 = cells
            .get(1)
            .and_then(|w| w.trim_end_matches('%').trim().parse().ok())
            .filter(|w| (1..=100).contains(w))
            .ok_or_else(|| at("give a weight from 1 to 100 after the name".into()))?;
        let price = match cells.get(2) {
            None | Some(&"") => false,
            Some(w) if w.eq_ignore_ascii_case("price") => true,
            Some(other) => return Err(at(format!("`{other}` is not understood; the third part can only be `price`"))),
        };
        if out.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            return Err(at(format!("{name} is listed twice")));
        }
        out.push(Criterion { name, weight, price });
        if out.len() > 10 {
            return Err(p.bad("text", "lists more than 10 criteria"));
        }
    }
    if !out.is_empty() && out.iter().map(|c| c.weight).sum::<u32>() != 100 {
        return Err(p.bad("text", format!("weights add up to {}, not 100", out.iter().map(|c| c.weight).sum::<u32>())));
    }
    if out.iter().filter(|c| c.price).count() > 1 {
        return Err(p.bad("text", "only one criterion can be the price"));
    }
    b.criteria = out;
    for bid in &mut b.bids {
        bid.scores.clear();
    }
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn score(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    editable(&b)?;
    if b.criteria.is_empty() {
        return Err(EngineError::conflict("set the scoring criteria first"));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    let given = p.object("scores").ok_or_else(|| p.bad("scores", "is required: {criterion number: 0 to 10}"))?;
    let mut scores: Vec<BidScore> = Vec::new();
    for (key, v) in given {
        // A blank box on the form means "not scored yet".
        if v.as_str().is_some_and(|t| t.trim().is_empty()) {
            continue;
        }
        let number: usize =
            key.trim().parse().ok().filter(|n| *n >= 1).ok_or_else(|| p.bad("scores", format!("`{key}` is not a criterion number")))?;
        let Some(c) = b.criteria.get(number - 1) else { return Err(p.bad("scores", format!("there is no criterion {number}"))) };
        if c.price {
            return Err(p.bad("scores", format!("{} is worked out from the bids, not scored", c.name)));
        }
        let value = v
            .as_u64()
            .or_else(|| v.as_str().and_then(|t| t.trim().parse().ok()))
            .filter(|s| *s <= 10)
            .ok_or_else(|| p.bad("scores", format!("{}: give a score from 0 to 10", c.name)))?;
        scores.push(BidScore { criterion: u32::try_from(number - 1).unwrap_or(0), score: u32::try_from(value).unwrap_or(0) });
    }
    let Some(bid) = b.bids.iter_mut().find(|x| x.vendor == vendor && !x.withdrawn) else {
        return Err(EngineError::not_found("that company has no standing bid"));
    };
    for s in scores {
        bid.scores.retain(|x| x.criterion != s.criterion);
        bid.scores.push(s);
    }
    bid.scores.sort_by_key(|s| s.criterion);
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn withdraw_bid(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if b.status == BidStatus::Awarded {
        return Err(EngineError::conflict(format!("{} is awarded", b.label())));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    let Some(bid) = b.bids.iter_mut().find(|x| x.vendor == vendor && !x.withdrawn) else {
        return Err(EngineError::not_found("that company has no standing bid"));
    };
    bid.withdrawn = true;
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id }))
}

fn award(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if !matches!(b.status, BidStatus::Open | BidStatus::Closed) {
        return Err(EngineError::conflict(format!("{} is {}", b.label(), b.status)));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    let Some(bid) = b.bids.iter().find(|x| x.vendor == vendor && !x.withdrawn).cloned() else {
        return Err(EngineError::conflict("that company has no standing bid"));
    };
    super::prequal::check_prequalified(ctx, vendor)?;
    if b.bond_required.is_some() && !bid.bond.received {
        return Err(EngineError::conflict("this package requires a bid bond; record that the company's bond was received first"));
    }
    if let (Some(pct), Some(got)) = (b.bond_required, bid.bond.amount)
        && bid.amount.percent(pct).is_some_and(|need| got < need)
    {
        return Err(EngineError::conflict(format!(
            "the recorded bond is below the {pct} of the bid this package requires; record the corrected bond first"
        )));
    }
    let mut commitment: Option<Id<Commitment>> = None;
    if p.flag_or("commitment", false)? {
        let Some(code) = b.cost_code else { return Err(EngineError::conflict("set the package's cost code to start a commitment")) };
        let number = ctx.state.commitments.iter().filter(|c| c.project == b.project).map(|c| c.number).max().unwrap_or(0).saturating_add(1);
        let id = ctx.state.commitments.alloc();
        let stamp = ctx.stamp();
        ctx.state.commitments.put(Commitment {
            id,
            project: b.project,
            number,
            title: b.title.clone(),
            kind: CommitmentKind::default(),
            status: CommitmentStatus::Draft,
            vendor: Some(vendor),
            description: b.scope.clone(),
            lines: vec![CommitmentLine { cost_code: code, description: b.title.clone(), amount: bid.amount }],
            retainage: None,
            approved: None,
            attachments: Vec::new(),
            signature: Default::default(),
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        commitment = Some(id);
    }
    b.status = BidStatus::Awarded;
    b.awarded_to = Some(vendor);
    b.commitment = commitment;
    b.updated = ctx.stamp();
    let id = b.id;
    ctx.state.bid_packages.put(b);
    Ok(json!({ "id": id, "commitment": commitment }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let b = load(ctx, p)?;
    let stamp = ctx.stamp();
    ctx.state.bid_packages.update(b.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": b.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&ctx.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_none() {
        return Err(EngineError::conflict("the package is not in the recycle bin"));
    }
    ctx.state.bid_packages.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// The project's packages, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&BidPackage> {
    let mut v: Vec<&BidPackage> = state.bid_packages.iter().filter(|b| b.project == project && b.deleted.is_none()).collect();
    v.sort_by_key(|b| std::cmp::Reverse(b.number));
    v
}

/// The lowest, average and highest standing bid, when there are any.
pub fn spread(b: &BidPackage) -> Option<(Money, Money, Money)> {
    let v = b.standing();
    let low = v.first()?.amount;
    let high = v.last()?.amount;
    let sum = Money::sum(v.iter().map(|x| x.amount))?;
    let avg = sum.checked_div_int(i64::try_from(v.len()).ok()?)?;
    Some((low, avg, high))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Bidding) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Result<Vec<Value>> = v.state.bid_packages.iter().filter(|b| b.project == project && b.deleted.is_some()).map(team_view).collect();
        return Ok(Value::Array(bin?));
    }
    let status = p.choice::<BidStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Result<Vec<Value>> = visible(v.state, project)
        .into_iter()
        .filter(|b| status.is_none_or(|s| b.status == s))
        .filter(|b| q.as_ref().is_none_or(|q| b.title.to_lowercase().contains(q.as_str()) || b.scope.to_lowercase().contains(q.as_str())))
        .map(team_view)
        .collect();
    Ok(Value::Array(list?))
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&v.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {id} not found in this project")));
    }
    let mut out = team_view(b)?;
    if let Some(o) = out.as_object_mut() {
        o.insert("standing".into(), serde_json::to_value(b.standing()).unwrap_or(Value::Null));
    }
    Ok(out)
}
