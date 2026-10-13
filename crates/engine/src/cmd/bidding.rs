//! Bidding: send a package of work out to invited companies, record the bids that come back,
//! compare them, and award. Awarding can start a draft commitment for the winner.
//!
//! Bidders have no accounts: a person on the team enters each bid as it arrives, with the unit prices
//! of the bid form, the alternates, what the bid covers of each scope item (and what it would cost to
//! bring it up to the full scope), the bid bond and the files the company sent. Addenda go out while the
//! package is out for bid and each company's acknowledgement is recorded.

use buildkiosk_core::{Date, Id, Money, Percent, text};
use buildkiosk_model::bidding::{
    Acknowledgement, Addendum, Bid, BidAlternate, BidItem, BidLine, BidPackage, BidStatus, BondReceipt, ScopeAnswer, ScopeResponse,
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
use crate::view::json;
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
    let structural = ["scope_items", "form_items", "alternates"].iter().any(|k| p.present(k));
    if structural && b.bids.iter().any(|x| !x.withdrawn) {
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
        addenda: Vec::new(),
        acknowledgements: Vec::new(),
        attachments: Vec::new(),
        bids: Vec::new(),
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
    let b = set_status(ctx, p, &[BidStatus::Open], BidStatus::Closed)?;
    Ok(json!({ "id": b.id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let b = set_status(ctx, p, &[BidStatus::Closed], BidStatus::Open)?;
    Ok(json!({ "id": b.id }))
}

fn record_bid(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut b = load(ctx, p)?;
    if !matches!(b.status, BidStatus::Open | BidStatus::Closed) {
        return Err(EngineError::conflict(format!("{} is {}; bids can be recorded once it is out for bid", b.label(), b.status)));
    }
    let vendor: Id<Vendor> = p.id("vendor")?;
    ctx.check_vendor(vendor)?;
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
    // A new bid from the same company replaces the old one but keeps the files it sent.
    let files = b.bids.iter().find(|x| x.vendor == vendor).map(|x| x.files.clone()).unwrap_or_default();
    b.bids.retain(|x| x.vendor != vendor);
    b.bids.push(Bid { vendor, amount, received, notes, withdrawn: false, lines, alternates, scope, bond, files });
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
            })
        })
        .collect();
    Ok(json!({
        "package": b.id, "scope_items": b.scope_items, "form_items": b.form_items, "alternates": b.alternates, "bond_required": b.bond_required,
        "addenda": b.addenda.len(), "rows": rows,
    }))
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
        let bin: Vec<&BidPackage> = v.state.bid_packages.iter().filter(|b| b.project == project && b.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<BidStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&BidPackage> = visible(v.state, project)
        .into_iter()
        .filter(|b| status.is_none_or(|s| b.status == s))
        .filter(|b| q.as_ref().is_none_or(|q| b.title.to_lowercase().contains(q.as_str()) || b.scope.to_lowercase().contains(q.as_str())))
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<BidPackage> = p.id("id")?;
    let b = in_project(&v.state.bid_packages, project, id, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {id} not found in this project")));
    }
    let mut out = serde_json::to_value(b).map_err(|e| EngineError::Internal { cmd: "bid_package.get".into(), msg: e.to_string() })?;
    if let Some(o) = out.as_object_mut() {
        o.insert("standing".into(), serde_json::to_value(b.standing()).unwrap_or(Value::Null));
    }
    Ok(out)
}
