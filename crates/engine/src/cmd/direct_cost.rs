//! Direct costs: money spent on a project without a commitment — a purchase, a payroll cost, a
//! rental — each coded to a cost code and counted in the budget.
//!
//! Anyone with Standard access records a cost; its author or an admin edits or deletes it.

use buildkiosk_core::{Id, Money, text};
use buildkiosk_model::directory::Vendor;
use buildkiosk_model::finance::DirectCost;
use buildkiosk_model::project::{CostCode, CostType, Project};
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::budget::usable_code;
use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::json;
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "direct_cost.create",
        "Record direct cost",
        Tool::DirectCosts,
        Access::Project(Level::Standard),
        "project: project id; cost_code: cost code id; amount: amount (credits may be negative, not zero); description: text; date?: YYYY-MM-DD (default: today); \
         kind?: labor|material|equipment|subcontract|other; vendor?: company id",
        create,
    ),
    CommandSpec::mutate(
        "direct_cost.update",
        "Edit direct cost",
        Tool::DirectCosts,
        Access::Project(Level::Standard),
        "project, id: ids; then any direct_cost.create field",
        update,
    ),
    CommandSpec::mutate("direct_cost.delete", "Delete direct cost", Tool::DirectCosts, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("direct_cost.restore", "Restore direct cost", Tool::DirectCosts, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "direct_cost.list",
        "List direct costs",
        Tool::DirectCosts,
        Access::Project(Level::Read),
        "project: project id; from?, to?: YYYY-MM-DD; cost_code?: cost code id; deleted?: bool (recycle bin, admins)",
        list,
    ),
];

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<DirectCost> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<DirectCost> = p.id("id")?;
    let d = in_project(&ctx.state.direct_costs, project, id, "direct cost")?;
    if d.deleted.is_some() && ctx.level(project, Tool::DirectCosts) < Level::Admin {
        return Err(EngineError::not_found(format!("direct cost {id} not found in this project")));
    }
    Ok(d.clone())
}

fn can_change(ctx: &Ctx<'_>, d: &DirectCost) -> bool {
    ctx.level(d.project, Tool::DirectCosts) == Level::Admin || (d.created.by.is_some() && d.created.by == ctx.person())
}

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, d: &mut DirectCost) -> Result<()> {
    if p.present("cost_code") {
        let code: Id<CostCode> = p.id("cost_code")?;
        usable_code(ctx.state, d.project, code)?;
        d.cost_code = code;
    }
    if p.present("amount") {
        let m: Money = p.money("amount")?.ok_or_else(|| p.bad("amount", "is required"))?;
        if m.is_zero() {
            return Err(p.bad("amount", "must not be zero"));
        }
        d.amount = m;
    }
    if p.present("description") {
        d.description = p.req_line("description", text::SHORT)?;
    }
    if let Some(date) = p.date("date")? {
        d.date = date;
    }
    if let Some(k) = p.choice::<CostType>("kind")? {
        d.kind = k;
    }
    if p.present("vendor") {
        let v = p.opt_id::<Vendor>("vendor")?;
        if let Some(v) = v {
            ctx.check_vendor(v)?;
        }
        d.vendor = v;
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if !p.present("cost_code") || !p.present("amount") || !p.present("description") {
        return Err(p.bad("cost_code", "cost_code, amount and description are required"));
    }
    let stamp = ctx.stamp();
    let mut d = DirectCost {
        id: ctx.state.direct_costs.peek_next(),
        project,
        date: ctx.today(),
        cost_code: Id::new(0),
        kind: CostType::default(),
        vendor: None,
        description: String::new(),
        amount: Money::ZERO,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(ctx, p, &mut d)?;
    let id = ctx.state.direct_costs.alloc();
    d.id = id;
    ctx.state.direct_costs.put(d);
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    if d.deleted.is_some() || !can_change(ctx, &d) {
        return Err(EngineError::forbidden("only the person who recorded a direct cost, or an admin, can change it"));
    }
    let before = d.clone();
    apply_fields(ctx, p, &mut d)?;
    let id = d.id;
    if d != before {
        d.updated = ctx.stamp();
        ctx.state.direct_costs.put(d);
    }
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let d = load(ctx, p)?;
    if d.deleted.is_some() || !can_change(ctx, &d) {
        return Err(EngineError::forbidden("only the person who recorded a direct cost, or an admin, can delete it"));
    }
    let stamp = ctx.stamp();
    ctx.state.direct_costs.update(d.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": d.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<DirectCost> = p.id("id")?;
    let d = in_project(&ctx.state.direct_costs, project, id, "direct cost")?;
    if d.deleted.is_none() {
        return Err(EngineError::conflict("this direct cost is not in the recycle bin"));
    }
    ctx.state.direct_costs.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::DirectCosts) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&DirectCost> = v.state.direct_costs.iter().filter(|d| d.project == project && d.deleted.is_some()).collect();
        return json(&bin);
    }
    let (from, to) = (p.date("from")?, p.date("to")?);
    let code = p.opt_id::<CostCode>("cost_code")?;
    let mut list: Vec<&DirectCost> = v
        .state
        .direct_costs
        .iter()
        .filter(|d| d.project == project && d.deleted.is_none())
        .filter(|d| from.is_none_or(|f| d.date >= f) && to.is_none_or(|t| d.date <= t) && code.is_none_or(|c| d.cost_code == c))
        .collect();
    list.sort_by_key(|d| std::cmp::Reverse((d.date, d.id)));
    json(&list)
}
