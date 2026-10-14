//! Estimating: price a project by cost code (quantity × unit cost), add markups, issue it, and
//! put the result into the budget.

use buildkiosk_core::{Id, Money, Percent, text};
use buildkiosk_model::estimate::{Alternate, Estimate, EstimateLine, EstimateStatus, EstimateTemplate, Markup, TemplateAlternate, TemplateLine};
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::{Choice, CompanyState, Level, Tool, perm};
use serde_json::{Value, json};

use super::budget::{upsert, usable_code};
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "estimate.create",
        "Create estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project: project id; name: text; notes?: text; lines?: [{cost_code: cost code id, description: text, quantity: number (up to 3 decimals), unit?: text, unit_cost: amount}]; markups?: [{name: text, percent: percent of the subtotal}]; alternates?: [{name: text, cost_code: cost code id, amount: amount (negative for a deduct), description?: text, accepted?: bool}] (priced apart from the base estimate; accepted ones are added to the subtotal); template?: estimate template id (used when no lines are given: its lines, markups and alternates, matched to this project's cost codes by number; refused when the project lacks one)",
        create,
    ),
    CommandSpec::mutate(
        "estimate.update",
        "Edit estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids; then any estimate.create field (lists replace the old ones; drafts only)",
        update,
    ),
    CommandSpec::mutate(
        "estimate.finalize",
        "Issue estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (needs at least one line)",
        finalize,
    ),
    CommandSpec::mutate(
        "estimate.accept_alternate",
        "Accept or decline an alternate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids; index: alternate number (from 0); accepted: bool (accepted alternates are added to the subtotal; not once the estimate is in the budget)",
        accept_alternate,
    ),
    CommandSpec::mutate(
        "estimate.duplicate",
        "Copy estimate as a new version",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids; name?: text (default: the old name with `copy` added). A draft with the same lines, markups and alternates (none accepted), noting which estimate it came from",
        duplicate,
    ),
    CommandSpec::mutate(
        "estimate.template_save",
        "Save estimate as template",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids of the estimate; name: text (a company-wide template of the lines, markups and alternates, with cost codes kept by number; a template of the same name is replaced)",
        template_save,
    ),
    CommandSpec::mutate(
        "estimate.template_delete",
        "Delete estimate template",
        Tool::Estimating,
        Access::Member,
        "id: template id (its author or a company admin)",
        template_delete,
    ),
    CommandSpec::query("estimate.template_list", "List estimate templates", Tool::Estimating, Access::Member, "(none)", template_list),
    CommandSpec::query(
        "estimate.compare",
        "Compare two estimates",
        Tool::Estimating,
        Access::Project(Level::Read),
        "project: project id; id: estimate id; with: the estimate to compare it with (by cost code: each one's total and the difference)",
        compare,
    ),
    CommandSpec::mutate(
        "estimate.add_from_catalog",
        "Add a catalogue item to an estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (a draft); item: catalogue item id; quantity: number (up to 3 decimals); cost_code?: cost code id (default: the item's, when the project has it); description?: text",
        add_from_catalog,
    ),
    CommandSpec::mutate(
        "estimate.add_assembly",
        "Add an assembly to an estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (a draft); assembly: assembly id; quantity: how many units of the assembly; cost_code?: cost code id for the parts whose items have none the project can use. One line is added per part",
        add_assembly,
    ),
    CommandSpec::mutate(
        "estimate.add_from_bid",
        "Add a bid to an estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (a draft); package: bid package id; vendor?: company id (default: the lowest standing bid). One lump-sum line at the bid amount, under the package's cost code",
        add_from_bid,
    ),
    CommandSpec::query(
        "takeoff.summary",
        "Measurements on a sheet",
        Tool::Estimating,
        Access::Project(Level::Read),
        "project: project id; drawing: drawing id (the current revision). The lengths, areas and counts drawn on it, added up by what they were labelled",
        takeoff_summary,
    ),
    CommandSpec::mutate(
        "estimate.add_from_takeoff",
        "Add a takeoff total to an estimate",
        Tool::Estimating,
        Access::Project(Level::Standard),
        "project, id: ids (a draft); drawing: drawing id; kind: dim (lengths)|area|pin (counts); label?: the text on the markups to add up (blank: those with none); cost_code: cost code id; unit_cost: amount per unit; description?: text. One line: the total as the quantity (feet or metres, square feet or metres, or each)",
        add_from_takeoff,
    ),
    CommandSpec::mutate("estimate.reopen", "Reopen estimate", Tool::Estimating, Access::Project(Level::Standard), "project, id: ids", reopen),
    CommandSpec::mutate(
        "estimate.apply_to_budget",
        "Put estimate in budget",
        Tool::Estimating,
        Access::Project(Level::Admin),
        "project, id: ids; markup_cost_code?: cost code id (the budget line that holds the markups; without it the markups are left out) (sets each cost code's original budget to the estimate's amount; the budget must be unlocked)",
        apply_to_budget,
    ),
    CommandSpec::mutate("estimate.delete", "Delete estimate", Tool::Estimating, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("estimate.restore", "Restore estimate", Tool::Estimating, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "estimate.list",
        "List estimates",
        Tool::Estimating,
        Access::Project(Level::Read),
        "project: project id; status?: draft|final; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("estimate.get", "Show estimate", Tool::Estimating, Access::Project(Level::Read), "project, id: ids (with its totals)", get),
];

const MAX_LINES: usize = 2_000;
const MAX_MARKUPS: usize = 20;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Estimate> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&ctx.state.estimates, project, id, "estimate")?;
    if e.deleted.is_some() {
        return Err(EngineError::not_found(format!("estimate {id} not found in this project")));
    }
    Ok(e.clone())
}

/// "2.5" → 2500 thousandths; at most three decimals.
pub fn parse_quantity(s: &str) -> Option<i64> {
    let s = s.trim().replace(',', "");
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r.to_string()),
        None => (false, s),
    };
    let (whole, frac) = s.split_once('.').unwrap_or((s.as_str(), ""));
    if (whole.is_empty() && frac.is_empty())
        || frac.len() > 3
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let w: i64 = if whole.is_empty() { 0 } else { whole.parse().ok()? };
    let f: i64 = if frac.is_empty() { 0 } else { format!("{frac:0<3}").parse().ok()? };
    let v = w.checked_mul(1000)?.checked_add(f)?;
    (v <= 1_000_000_000_000).then_some(if neg { -v } else { v })
}

fn apply(ctx: &Ctx<'_>, p: &Params<'_>, e: &mut Estimate) -> Result<()> {
    if p.present("name") {
        e.name = p.req_line("name", text::SHORT)?;
    }
    if p.present("notes") {
        e.notes = p.text_or_empty("notes", text::LONG)?;
    }
    if p.present("lines") {
        let rows = p.items("lines")?;
        if rows.len() > MAX_LINES {
            return Err(p.bad("lines", format!("holds more than {MAX_LINES} lines")));
        }
        let mut out = Vec::new();
        for r in rows {
            let code: Option<Id<CostCode>> = r.opt_id("cost_code")?;
            let description = r.line_or_empty("description", text::SHORT)?;
            let qty_text = r.line_or_empty("quantity", 30)?;
            let cost: Option<Money> = r.money("unit_cost")?;
            if code.is_none() && description.is_empty() && qty_text.is_empty() && cost.is_none() {
                continue; // a spare blank row
            }
            let code = code.ok_or_else(|| r.bad("cost_code", "is required on every line"))?;
            usable_code(ctx.state, e.project, code)?;
            let quantity_milli = if qty_text.is_empty() {
                1000
            } else {
                parse_quantity(&qty_text).ok_or_else(|| r.bad("quantity", format!("`{qty_text}` is not a quantity (up to three decimals)")))?
            };
            let unit_cost = cost.ok_or_else(|| r.bad("unit_cost", "is required on every line"))?;
            let line = EstimateLine { cost_code: code, description, quantity_milli, unit: r.line_or_empty("unit", 20)?, unit_cost };
            if line.total().is_none() {
                return Err(r.bad("unit_cost", "the line total is too large"));
            }
            out.push(line);
        }
        e.lines = out;
    }
    if p.present("markups") {
        let rows = p.items("markups")?;
        if rows.len() > MAX_MARKUPS {
            return Err(p.bad("markups", format!("holds more than {MAX_MARKUPS} markups")));
        }
        let mut out = Vec::new();
        for r in rows {
            let name = r.line_or_empty("name", 100)?;
            let pct: Option<Percent> = r.percent("percent")?;
            if name.is_empty() && pct.is_none() {
                continue;
            }
            if name.is_empty() {
                return Err(r.bad("name", "is required on every markup"));
            }
            out.push(Markup { name, percent: pct.unwrap_or(Percent::ZERO) });
        }
        e.markups = out;
    }
    if p.present("alternates") {
        let rows = p.items("alternates")?;
        if rows.len() > MAX_MARKUPS {
            return Err(p.bad("alternates", format!("holds more than {MAX_MARKUPS} alternates")));
        }
        let mut out = Vec::new();
        for r in rows {
            let name = r.line_or_empty("name", 100)?;
            let amount: Option<Money> = r.money("amount")?;
            if name.is_empty() && amount.is_none() {
                continue;
            }
            if name.is_empty() {
                return Err(r.bad("name", "is required on every alternate"));
            }
            let code: Id<CostCode> = r.id("cost_code")?;
            usable_code(ctx.state, e.project, code)?;
            let amount = amount.ok_or_else(|| r.bad("amount", "is required on every alternate"))?;
            out.push(Alternate {
                name,
                description: r.line_or_empty("description", text::SHORT)?,
                cost_code: code,
                amount,
                accepted: r.flag_or("accepted", false)?,
            });
        }
        e.alternates = out;
    }
    if e.totals().is_none() {
        return Err(p.bad("lines", "the totals are too large to add up"));
    }
    Ok(())
}

fn draft(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Estimate> {
    let e = load(ctx, p)?;
    if e.status != EstimateStatus::Draft {
        return Err(EngineError::conflict(format!("{} was issued; reopen it to edit it", e.label())));
    }
    Ok(e)
}

fn accept_alternate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.applied.is_some() {
        return Err(EngineError::conflict(format!("{} is already in the budget", e.label())));
    }
    let index = usize::try_from(p.int("index", 0..=1_000)?.ok_or_else(|| p.bad("index", "is required"))?).unwrap_or(usize::MAX);
    let accepted = p.flag("accepted")?.ok_or_else(|| p.bad("accepted", "is required"))?;
    let Some(a) = e.alternates.get_mut(index) else {
        return Err(EngineError::not_found(format!("{} has no alternate {index}", e.label())));
    };
    a.accepted = accepted;
    if e.totals().is_none() {
        return Err(EngineError::conflict("the totals are too large to add up"));
    }
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn duplicate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let from = load(ctx, p)?;
    let number = ctx.state.estimates.iter().filter(|e| e.project == from.project).map(|e| e.number).max().unwrap_or(0).saturating_add(1);
    let name = match p.line("name", text::SHORT)?.filter(|n| !n.is_empty()) {
        Some(n) => n,
        None => format!("{} (copy)", from.name),
    };
    let stamp = ctx.stamp();
    let id = ctx.state.estimates.alloc();
    let mut copy = from.clone();
    copy.id = id;
    copy.number = number;
    copy.name = name;
    copy.status = EstimateStatus::Draft;
    copy.applied = None;
    copy.based_on = Some(from.id);
    for a in &mut copy.alternates {
        a.accepted = false;
    }
    copy.created = stamp;
    copy.updated = stamp;
    copy.deleted = None;
    ctx.state.estimates.put(copy);
    Ok(json!({ "id": id, "number": number }))
}

fn compare(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let with: Id<Estimate> = p.id("with")?;
    let get = |id| {
        in_project(&v.state.estimates, project, id, "estimate")
            .ok()
            .filter(|e| e.deleted.is_none())
            .ok_or_else(|| EngineError::not_found(format!("estimate {id} not found in this project")))
    };
    let (a, b) = (get(id)?, get(with)?);
    let by_code = |e: &Estimate| -> Vec<(Id<CostCode>, Money)> {
        let mut out: Vec<(Id<CostCode>, Money)> = Vec::new();
        let lines = e.lines.iter().map(|l| (l.cost_code, l.total().unwrap_or(Money::ZERO)));
        let alts = e.alternates.iter().filter(|x| x.accepted).map(|x| (x.cost_code, x.amount));
        for (code, amt) in lines.chain(alts) {
            match out.iter_mut().find(|(c, _)| *c == code) {
                Some((_, m)) => *m = m.checked_add(amt).unwrap_or(*m),
                None => out.push((code, amt)),
            }
        }
        out
    };
    let (ma, mb) = (by_code(a), by_code(b));
    let mut codes: Vec<Id<CostCode>> = ma.iter().map(|(c, _)| *c).chain(mb.iter().map(|(c, _)| *c)).collect();
    codes.sort();
    codes.dedup();
    let amount = |m: &[(Id<CostCode>, Money)], c: Id<CostCode>| m.iter().find(|(x, _)| *x == c).map_or(Money::ZERO, |(_, v)| *v);
    let rows: Vec<Value> = codes
        .into_iter()
        .map(|c| {
            let (x, y) = (amount(&ma, c), amount(&mb, c));
            let code = v.state.cost_codes.get(c);
            json!({ "cost_code": c, "code": code.map(|c| c.code.clone()), "description": code.map(|c| c.description.clone()), "this": x, "other": y, "difference": x.checked_sub(y) })
        })
        .collect();
    let (ta, tb) = (a.totals(), b.totals());
    Ok(json!({
        "this": { "id": a.id, "number": a.number, "name": a.name, "total": ta.as_ref().map(|t| t.total) },
        "other": { "id": b.id, "number": b.number, "name": b.name, "total": tb.as_ref().map(|t| t.total) },
        "difference": ta.zip(tb).and_then(|(x, y)| x.total.checked_sub(y.total)),
        "rows": rows,
    }))
}

fn push_line(e: &mut Estimate, line: EstimateLine) -> Result<()> {
    if e.lines.len() >= MAX_LINES {
        return Err(EngineError::conflict(format!("{} has the most lines it can hold", e.label())));
    }
    if line.total().is_none() {
        return Err(EngineError::conflict("the line total is too large"));
    }
    e.lines.push(line);
    if e.totals().is_none() {
        return Err(EngineError::conflict("the totals are too large to add up"));
    }
    Ok(())
}

fn catalog_read(ctx: &Ctx<'_>) -> Result<()> {
    if super::catalog::can_read(ctx.state, ctx.actor) {
        Ok(())
    } else {
        Err(EngineError::forbidden("the catalogue is for people who work on estimates"))
    }
}

fn quantity(p: &Params<'_>) -> Result<i64> {
    let q = p.line_or_empty("quantity", 30)?;
    parse_quantity(&q).filter(|v| *v > 0).ok_or_else(|| p.bad("quantity", "is required: a number above zero (up to three decimals)"))
}

/// The cost code for a catalogue line: the one asked for, else the item's when the project can use it.
fn code_for(
    ctx: &Ctx<'_>,
    project: Id<Project>,
    asked: Option<Id<CostCode>>,
    item: &buildkiosk_model::estimate::CatalogItem,
) -> Result<Id<CostCode>> {
    if let Some(c) = asked {
        usable_code(ctx.state, project, c)?;
        return Ok(c);
    }
    match item.cost_code.filter(|c| usable_code(ctx.state, project, *c).is_ok()) {
        Some(c) => Ok(c),
        None => Err(EngineError::conflict(format!("`{}` has no cost code this project uses; choose one", item.name))),
    }
}

fn add_from_catalog(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    catalog_read(ctx)?;
    let mut e = draft(ctx, p)?;
    let id: Id<buildkiosk_model::estimate::CatalogItem> = p.id("item")?;
    let item = ctx
        .state
        .catalog_items
        .get(id)
        .filter(|i| i.deleted.is_none())
        .cloned()
        .ok_or_else(|| EngineError::not_found(format!("catalogue item {id} not found")))?;
    let code = code_for(ctx, e.project, p.opt_id("cost_code")?, &item)?;
    let description = p.line("description", text::SHORT)?.filter(|d| !d.is_empty()).unwrap_or_else(|| item.name.clone());
    push_line(
        &mut e,
        EstimateLine { cost_code: code, description, quantity_milli: quantity(p)?, unit: item.unit.clone(), unit_cost: item.unit_cost },
    )?;
    e.updated = ctx.stamp();
    let eid = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": eid }))
}

fn add_assembly(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    catalog_read(ctx)?;
    let mut e = draft(ctx, p)?;
    let id: Id<buildkiosk_model::estimate::Assembly> = p.id("assembly")?;
    let assembly = ctx
        .state
        .assemblies
        .get(id)
        .filter(|a| a.deleted.is_none())
        .cloned()
        .ok_or_else(|| EngineError::not_found(format!("assembly {id} not found")))?;
    let units = quantity(p)?;
    let asked: Option<Id<CostCode>> = p.opt_id("cost_code")?;
    let before = e.lines.len();
    for part in &assembly.parts {
        let item = ctx
            .state
            .catalog_items
            .get(part.item)
            .filter(|i| i.deleted.is_none())
            .cloned()
            .ok_or_else(|| EngineError::conflict("an item of this assembly is no longer in the catalogue"))?;
        // The item's own code wins where the project has it; the one asked for covers the rest.
        let code = match item.cost_code.filter(|c| usable_code(ctx.state, e.project, *c).is_ok()) {
            Some(c) => c,
            None => code_for(ctx, e.project, asked, &item)?,
        };
        // Units of the assembly times the part's quantity per unit, in thousandths, rounded to the nearest.
        let q = i128::from(units) * i128::from(part.quantity_milli);
        let quantity_milli =
            i64::try_from((q + if q >= 0 { 500 } else { -500 }) / 1000).map_err(|_| EngineError::conflict("the quantity is too large"))?;
        push_line(
            &mut e,
            EstimateLine {
                cost_code: code,
                description: format!("{}: {}", assembly.name, item.name),
                quantity_milli,
                unit: item.unit.clone(),
                unit_cost: item.unit_cost,
            },
        )?;
    }
    let added = e.lines.len() - before;
    e.updated = ctx.stamp();
    let eid = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": eid, "lines_added": added }))
}

fn add_from_bid(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = draft(ctx, p)?;
    let pkg: Id<buildkiosk_model::bidding::BidPackage> = p.id("package")?;
    let b = in_project(&ctx.state.bid_packages, e.project, pkg, "bid package")?;
    if b.deleted.is_some() {
        return Err(EngineError::not_found(format!("bid package {pkg} not found in this project")));
    }
    let code = b.cost_code.ok_or_else(|| EngineError::conflict("the bid package has no cost code; set one first"))?;
    let wanted: Option<Id<buildkiosk_model::directory::Vendor>> = p.opt_id("vendor")?;
    let bid = match wanted {
        Some(v) => b.bids.iter().find(|x| x.vendor == v && !x.withdrawn),
        None => b.bids.iter().filter(|x| !x.withdrawn).min_by_key(|x| x.amount),
    }
    .cloned()
    .ok_or_else(|| EngineError::conflict("there is no standing bid to add"))?;
    let title = b.title.clone();
    usable_code(ctx.state, e.project, code)?;
    let vendor = crate::view::vendor_name(ctx.state, bid.vendor);
    push_line(
        &mut e,
        EstimateLine {
            cost_code: code,
            description: format!("{title}: bid from {vendor}"),
            quantity_milli: 1000,
            unit: "ls".into(),
            unit_cost: bid.amount,
        },
    )?;
    e.updated = ctx.stamp();
    let eid = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": eid }))
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.estimates.iter().filter(|e| e.project == project).map(|e| e.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut e = Estimate {
        id: ctx.state.estimates.peek_next(),
        project,
        number,
        name: String::new(),
        notes: String::new(),
        status: EstimateStatus::Draft,
        lines: Vec::new(),
        markups: Vec::new(),
        alternates: Vec::new(),
        based_on: None,
        applied: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply(ctx, p, &mut e)?;
    if e.name.is_empty() {
        return Err(p.bad("name", "is required"));
    }
    if e.lines.is_empty() && p.present("template") {
        use_template(ctx, p, &mut e)?;
    }
    let id = ctx.state.estimates.alloc();
    e.id = id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id, "number": number }))
}

/// Fills an empty estimate from a template, finding each cost code by its number in the project.
fn use_template(ctx: &Ctx<'_>, p: &Params<'_>, e: &mut Estimate) -> Result<()> {
    let tid: Id<EstimateTemplate> = p.id("template")?;
    let t = ctx
        .state
        .estimate_templates
        .get(tid)
        .filter(|t| t.deleted.is_none())
        .ok_or_else(|| p.bad("template", format!("estimate template {tid} not found")))?;
    let codes = view::project_cost_codes(ctx.state, e.project);
    let find = |code: &str| codes.iter().find(|c| c.code == code).map(|c| c.id);
    let mut missing: Vec<&str> = Vec::new();
    let wanted = t.lines.iter().map(|l| l.code.as_str()).chain(t.alternates.iter().map(|a| a.code.as_str()));
    for code in wanted {
        if find(code).is_none() && !missing.contains(&code) {
            missing.push(code);
        }
    }
    if !missing.is_empty() {
        let shown: Vec<&str> = missing.iter().take(10).copied().collect();
        return Err(EngineError::conflict(format!(
            "this project has no cost code {}{}; add them to the project first",
            shown.join(", "),
            if missing.len() > shown.len() { " and others" } else { "" }
        )));
    }
    for l in &t.lines {
        let Some(cost_code) = find(&l.code) else { continue };
        e.lines.push(EstimateLine {
            cost_code,
            description: l.description.clone(),
            quantity_milli: l.quantity_milli,
            unit: l.unit.clone(),
            unit_cost: l.unit_cost,
        });
    }
    if e.notes.is_empty() {
        e.notes = t.notes.clone();
    }
    if e.markups.is_empty() {
        e.markups = t.markups.clone();
    }
    if e.alternates.is_empty() {
        for a in &t.alternates {
            let Some(cost_code) = find(&a.code) else { continue };
            e.alternates.push(Alternate { name: a.name.clone(), description: a.description.clone(), cost_code, amount: a.amount, accepted: false });
        }
    }
    Ok(())
}

fn template_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e = load(ctx, p)?;
    let name = p.req_line("name", text::SHORT)?;
    if e.lines.is_empty() {
        return Err(EngineError::conflict("the estimate has no lines to keep"));
    }
    let code_of = |id: Id<CostCode>| ctx.state.cost_codes.get(id).map(|c| c.code.clone()).unwrap_or_default();
    let lines: Vec<TemplateLine> = e
        .lines
        .iter()
        .map(|l| TemplateLine {
            code: code_of(l.cost_code),
            description: l.description.clone(),
            quantity_milli: l.quantity_milli,
            unit: l.unit.clone(),
            unit_cost: l.unit_cost,
        })
        .collect();
    let alternates: Vec<TemplateAlternate> = e
        .alternates
        .iter()
        .map(|a| TemplateAlternate { name: a.name.clone(), description: a.description.clone(), code: code_of(a.cost_code), amount: a.amount })
        .collect();
    let stamp = ctx.stamp();
    let existing = ctx.state.estimate_templates.iter().find(|t| t.deleted.is_none() && t.name.eq_ignore_ascii_case(&name)).map(|t| t.id);
    let id = match existing {
        Some(id) => {
            ctx.state.estimate_templates.update(id, |t| {
                t.notes = e.notes.clone();
                t.lines = lines;
                t.markups = e.markups.clone();
                t.alternates = alternates;
            });
            id
        }
        None => {
            if ctx.state.estimate_templates.iter().filter(|t| t.deleted.is_none()).count() >= 500 {
                return Err(EngineError::conflict("the company keeps at most 500 estimate templates"));
            }
            let id = ctx.state.estimate_templates.alloc();
            ctx.state.estimate_templates.put(EstimateTemplate {
                id,
                name,
                notes: e.notes.clone(),
                lines,
                markups: e.markups.clone(),
                alternates,
                created: stamp,
                deleted: None,
            });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn template_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<EstimateTemplate> = p.id("id")?;
    let Some(t) = ctx.state.estimate_templates.get(id).filter(|t| t.deleted.is_none()) else {
        return Err(EngineError::not_found(format!("estimate template {id} not found")));
    };
    let admin = match ctx.actor {
        Actor::System => true,
        Actor::Person(x) => perm::company_level(ctx.state, x, Tool::CompanyAdmin) == Level::Admin,
    };
    if !admin && !(ctx.person().is_some() && t.created.by == ctx.person()) {
        return Err(EngineError::forbidden("only the template's author or a company admin can delete it"));
    }
    let stamp = ctx.stamp();
    ctx.state.estimate_templates.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// Whether the person may look at estimate templates: company admins and anyone who can read estimating on a project.
pub fn can_read_templates(state: &CompanyState, actor: Actor) -> bool {
    match actor {
        Actor::System => true,
        Actor::Person(p) => {
            perm::company_level(state, p, Tool::CompanyAdmin) == Level::Admin
                || perm::visible_projects(state, p).into_iter().any(|id| perm::project_level(state, p, id, Tool::Estimating) >= Level::Read)
        }
    }
}

/// The company's estimate templates, by name.
pub fn templates(state: &CompanyState) -> Vec<&EstimateTemplate> {
    let mut v: Vec<&EstimateTemplate> = state.estimate_templates.iter().filter(|t| t.deleted.is_none()).collect();
    v.sort_by_key(|t| t.name.to_lowercase());
    v
}

fn template_list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    if !can_read_templates(v.state, v.actor) {
        return Err(EngineError::forbidden("estimate templates need access to estimating on a project"));
    }
    json(&templates(v.state))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Draft {
        return Err(EngineError::conflict(format!("{} was issued; reopen it to edit it", e.label())));
    }
    apply(ctx, p, &mut e)?;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn finalize(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already issued", e.label())));
    }
    if e.lines.is_empty() {
        return Err(EngineError::conflict("add at least one line first"));
    }
    e.status = EstimateStatus::Final;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Final {
        return Err(EngineError::conflict(format!("{} is a draft", e.label())));
    }
    e.status = EstimateStatus::Draft;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id }))
}

fn apply_to_budget(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut e = load(ctx, p)?;
    if e.status != EstimateStatus::Final {
        return Err(EngineError::conflict("issue the estimate before putting it in the budget"));
    }
    if ctx.state.projects.get(e.project).is_some_and(|x| x.budget_locked) {
        return Err(EngineError::conflict("the original budget is locked; unlock it first"));
    }
    let t = e.totals().ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?;
    // Sum by cost code, in code order of first use; accepted alternates go under their own codes.
    let mut by_code: Vec<(Id<CostCode>, Money)> = Vec::new();
    let lines = e.lines.iter().map(|l| (l.cost_code, l.total().unwrap_or(Money::ZERO)));
    let alternates = e.alternates.iter().filter(|a| a.accepted).map(|a| (a.cost_code, a.amount));
    for (cost_code, amt) in lines.chain(alternates) {
        match by_code.iter_mut().find(|(c, _)| *c == cost_code) {
            Some((_, m)) => *m = m.checked_add(amt).ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?,
            None => by_code.push((cost_code, amt)),
        }
    }
    let markup_code: Option<Id<CostCode>> = p.opt_id("markup_cost_code")?;
    if let Some(mc) = markup_code {
        usable_code(ctx.state, e.project, mc)?;
        let extra = t.total.checked_sub(t.subtotal).unwrap_or(Money::ZERO);
        if !extra.is_zero() {
            match by_code.iter_mut().find(|(c, _)| *c == mc) {
                Some((_, m)) => *m = m.checked_add(extra).ok_or_else(|| EngineError::conflict("the totals are too large to add up"))?,
                None => by_code.push((mc, extra)),
            }
        }
    }
    let lines = by_code.len();
    for (code, amount) in by_code {
        upsert(ctx, e.project, code, amount, None);
    }
    e.applied = Some(ctx.stamp());
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id, "budget_lines": lines }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let e = load(ctx, p)?;
    let stamp = ctx.stamp();
    ctx.state.estimates.update(e.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": e.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&ctx.state.estimates, project, id, "estimate")?;
    if e.deleted.is_none() {
        return Err(EngineError::conflict("the estimate is not in the recycle bin"));
    }
    ctx.state.estimates.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// The project's estimates, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&Estimate> {
    let mut v: Vec<&Estimate> = state.estimates.iter().filter(|e| e.project == project && e.deleted.is_none()).collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Estimating) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Estimate> = v.state.estimates.iter().filter(|e| e.project == project && e.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<EstimateStatus>("status")?;
    let list: Vec<Value> = visible(v.state, project)
        .into_iter()
        .filter(|e| status.is_none_or(|s| e.status == s))
        .map(|e| json!({ "id": e.id, "number": e.number, "name": e.name, "status": e.status, "lines": e.lines.len(), "total": e.totals().map(|t| t.total), "applied": e.applied }))
        .collect();
    Ok(Value::Array(list))
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Estimate> = p.id("id")?;
    let e = in_project(&v.state.estimates, project, id, "estimate")?;
    if e.deleted.is_some() {
        return Err(EngineError::not_found(format!("estimate {id} not found in this project")));
    }
    let mut out = serde_json::to_value(e).map_err(|x| EngineError::Internal { cmd: "estimate.get".into(), msg: x.to_string() })?;
    if let (Some(o), Some(t)) = (out.as_object_mut(), e.totals()) {
        o.insert("totals".into(), json!({ "subtotal": t.subtotal, "markups": t.markups, "total": t.total }));
    }
    Ok(out)
}

/// One kind of measurement under one label on a sheet, added up.
struct Group {
    kind: buildkiosk_model::drawing::MarkupKind,
    label: String,
    count: usize,
    /// Micrometres for lengths, square micrometres for areas; unused for counts.
    total: u128,
    unit: Option<buildkiosk_model::drawing::LengthUnit>,
    /// More than one unit in the group: it can't be added up.
    mixed: bool,
}

impl Group {
    /// The quantity in thousandths and its unit text ("lf", "sf", "ea"), in the units the sheet was measured in.
    fn quantity(&self) -> Option<(i64, String)> {
        use buildkiosk_model::drawing::{LengthUnit as U, MarkupKind as K};
        match self.kind {
            K::Pin => Some((i64::try_from(self.count).ok()?.checked_mul(1000)?, "ea".into())),
            K::Dimension | K::Area if self.mixed => None,
            K::Dimension => {
                let unit = self.unit?;
                let milli = self.total.checked_mul(1000)? / u128::from(unit.micrometres());
                Some((
                    i64::try_from(milli).ok()?,
                    match unit {
                        U::Feet => "lf",
                        U::Inches => "in",
                        U::Meters => "m",
                        U::Millimeters => "mm",
                    }
                    .into(),
                ))
            }
            K::Area => {
                let unit = self.unit?;
                let one = u128::from(unit.micrometres());
                let milli = self.total.checked_mul(1000)? / one.checked_mul(one)?;
                Some((
                    i64::try_from(milli).ok()?,
                    match unit {
                        U::Feet => "sf",
                        U::Inches => "sq in",
                        U::Meters => "sq m",
                        U::Millimeters => "sq mm",
                    }
                    .into(),
                ))
            }
            _ => None,
        }
    }
}

/// The kind a markup is totalled under in a take-off: paths with lengths, outlines with areas.
pub fn takeoff_kind(kind: buildkiosk_model::drawing::MarkupKind) -> buildkiosk_model::drawing::MarkupKind {
    use buildkiosk_model::drawing::MarkupKind as K;
    match kind {
        K::Polyline => K::Dimension,
        K::Polygon => K::Area,
        other => other,
    }
}

/// The measurements on the current revision of a sheet, by kind and label.
fn groups(state: &buildkiosk_model::CompanyState, project: Id<Project>, drawing: Id<buildkiosk_model::drawing::Drawing>) -> Result<Vec<Group>> {
    use buildkiosk_model::drawing::MarkupKind as K;
    let d = in_project(&state.drawings, project, drawing, "drawing")?;
    if d.deleted.is_some() {
        return Err(EngineError::not_found(format!("drawing {drawing} not found in this project")));
    }
    let mut out: Vec<Group> = Vec::new();
    for m in d.current().into_iter().flat_map(|r| &r.markups) {
        if !(m.kind.measures_length() || m.kind.measures_area() || m.kind == K::Pin) || (m.kind != K::Pin && m.measure == 0) {
            continue;
        }
        // A path adds up with straight lengths, an outline with boxes.
        let kind = takeoff_kind(m.kind);
        let label = m.text.trim().to_string();
        let g = match out.iter().position(|g| g.kind == kind && g.label == label) {
            Some(i) => i,
            None => {
                out.push(Group { kind, label, count: 0, total: 0, unit: m.unit, mixed: false });
                out.len() - 1
            }
        };
        if let Some(g) = out.get_mut(g) {
            g.count += 1;
            g.total = g.total.saturating_add(u128::from(m.measure));
            g.mixed |= g.unit != m.unit;
        }
    }
    Ok(out)
}

fn takeoff_summary(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let drawing: Id<buildkiosk_model::drawing::Drawing> = p.id("drawing")?;
    let rows: Vec<Value> = groups(v.state, project, drawing)?
        .iter()
        .map(|g| {
            let q = g.quantity();
            json!({ "kind": g.kind, "label": g.label, "count": g.count, "quantity": q.as_ref().map(|(m, _)| format!("{}.{:03}", m / 1000, m % 1000)), "unit": q.map(|(_, u)| u) })
        })
        .collect();
    Ok(Value::Array(rows))
}

fn add_from_takeoff(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    use buildkiosk_model::drawing::MarkupKind as K;
    let mut e = draft(ctx, p)?;
    let drawing: Id<buildkiosk_model::drawing::Drawing> = p.id("drawing")?;
    let kind =
        p.choice::<K>("kind")?.filter(|k| matches!(k, K::Dimension | K::Area | K::Pin)).ok_or_else(|| p.bad("kind", "must be dim, area or pin"))?;
    let label = p.line_or_empty("label", text::SHORT)?;
    let code: Id<CostCode> = p.id("cost_code")?;
    usable_code(ctx.state, e.project, code)?;
    let unit_cost = p.money("unit_cost")?.ok_or_else(|| p.bad("unit_cost", "is required"))?;
    if unit_cost.is_negative() {
        return Err(p.bad("unit_cost", "must not be negative"));
    }
    let all = groups(ctx.state, e.project, drawing)?;
    let g = all
        .iter()
        .find(|g| g.kind == kind && g.label == label)
        .ok_or_else(|| EngineError::not_found("no measurements on the sheet match that kind and label"))?;
    let (quantity_milli, unit) =
        g.quantity().ok_or_else(|| EngineError::conflict("those measurements are in different units; set the sheet's scale to one unit"))?;
    if quantity_milli <= 0 {
        return Err(EngineError::conflict("the measurements add up to nothing"));
    }
    let sheet = ctx.state.drawings.get(drawing).map(|d| d.label()).unwrap_or_default();
    let what = if label.is_empty() { kind.label().to_string() } else { label.clone() };
    let description = p.line("description", text::SHORT)?.filter(|d| !d.is_empty()).unwrap_or_else(|| format!("{what} (takeoff, {sheet})"));
    push_line(&mut e, EstimateLine { cost_code: code, description, quantity_milli, unit, unit_cost })?;
    e.updated = ctx.stamp();
    let id = e.id;
    ctx.state.estimates.put(e);
    Ok(json!({ "id": id, "quantity_milli": quantity_milli }))
}
