//! Bidding: send a package of work out to invited companies, record the bids that come back,
//! compare them, and award. Awarding can start a draft commitment for the winner.
//!
//! BuildKiosk has no portal for bidders: a person on the team enters each bid as it arrives.

use buildkiosk_core::{Date, Id, Money, text};
use buildkiosk_model::bidding::{Bid, BidPackage, BidStatus};
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::finance::{Commitment, CommitmentKind, CommitmentLine, CommitmentStatus};
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::usable_code;
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
        "project: project id; title: text; scope?: text; due?: YYYY-MM-DD; cost_code?: cost code id; invite?: [company ids]",
        create,
    ),
    CommandSpec::mutate(
        "bid_package.update",
        "Edit bid package",
        Tool::Bidding,
        Access::Project(Level::Standard),
        "project, id: ids; title?, scope?, due?, cost_code?, invite? (the invited list is replaced; not once awarded)",
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
        "project, id: ids; vendor: company id; amount: amount (above zero); received?: YYYY-MM-DD (default today); notes?: text (replaces that company's earlier bid; an uninvited company is added to the invited list)",
        record_bid,
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
    let amount = p.money("amount")?.ok_or_else(|| p.bad("amount", "is required"))?;
    if amount.is_negative() || amount.is_zero() {
        return Err(p.bad("amount", "must be above zero"));
    }
    let received: Date = p.date("received")?.unwrap_or_else(|| ctx.today());
    let notes = p.text_or_empty("notes", text::LONG)?;
    if !b.invited.contains(&vendor) {
        if b.invited.len() >= MAX_INVITED {
            return Err(EngineError::conflict(format!("a package invites at most {MAX_INVITED} companies")));
        }
        b.invited.push(vendor);
    }
    b.bids.retain(|x| x.vendor != vendor);
    b.bids.push(Bid { vendor, amount, received, notes, withdrawn: false });
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
