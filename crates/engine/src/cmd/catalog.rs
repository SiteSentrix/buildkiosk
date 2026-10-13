//! The company's price catalogue and assemblies: unit costs and groups of them that estimators reuse across
//! projects. Anyone with Standard access to Estimating on some project keeps them; anyone who can read
//! estimates on some project may look. They are company-wide: a line made from one is copied into the
//! estimate, so changing the catalogue later never changes an estimate that was already priced.

use buildkiosk_core::{Id, Money, text};
use buildkiosk_model::estimate::{Assembly, AssemblyPart, CatalogItem};
use buildkiosk_model::project::CostCode;
use buildkiosk_model::{CompanyState, Level, Tool, perm};
use serde_json::{Value, json};

use super::estimate::parse_quantity;
use super::{Access, CommandSpec};
use crate::{Actor, Ctx, EngineError, Params, Result, View, view};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "catalog.save",
        "Save catalogue item",
        Tool::Estimating,
        Access::Member,
        "id?: item id (changes it; without it a new one is made); name: text; unit_cost: amount; unit?: text (sf, cy, ea); cost_code?: cost code id; category?: text; notes?: text",
        save,
    ),
    CommandSpec::mutate(
        "catalog.delete",
        "Delete catalogue item",
        Tool::Estimating,
        Access::Member,
        "id: item id (not while an assembly uses it)",
        delete,
    ),
    CommandSpec::mutate(
        "catalog.import",
        "Import catalogue",
        Tool::Estimating,
        Access::Member,
        "text: CSV lines `name, unit, unit cost[, category[, cost code]]`; an optional first line starting with `name` is skipped; an item with the same name is changed",
        import,
    ),
    CommandSpec::query("catalog.list", "List catalogue", Tool::Estimating, Access::Member, "q?: text; category?: text", list),
    CommandSpec::mutate(
        "assembly.save",
        "Save assembly",
        Tool::Estimating,
        Access::Member,
        "id?: assembly id; name: text; unit?: text; notes?: text; parts: [{item: catalogue item id, quantity: number per unit of the assembly}]",
        assembly_save,
    ),
    CommandSpec::mutate("assembly.delete", "Delete assembly", Tool::Estimating, Access::Member, "id: assembly id", assembly_delete),
    CommandSpec::query("assembly.list", "List assemblies", Tool::Estimating, Access::Member, "(none) (with the cost of one unit)", assembly_list),
];

const MAX_ITEMS: usize = 20_000;
const MAX_ASSEMBLIES: usize = 2_000;
const MAX_PARTS: usize = 100;

/// Whether the person may look at the catalogue (and so at the unit costs): company admins, and anyone who can
/// read estimates on some project.
pub fn can_read(state: &CompanyState, actor: Actor) -> bool {
    match actor {
        Actor::System => true,
        Actor::Person(p) => {
            perm::company_level(state, p, Tool::CompanyAdmin) == Level::Admin
                || perm::visible_projects(state, p).into_iter().any(|id| perm::project_level(state, p, id, Tool::Estimating) >= Level::Read)
        }
    }
}

/// Whether the person may change it: company admins, and anyone with Standard access to estimating on some project.
pub fn can_write(state: &CompanyState, actor: Actor) -> bool {
    match actor {
        Actor::System => true,
        Actor::Person(p) => {
            perm::company_level(state, p, Tool::CompanyAdmin) == Level::Admin
                || perm::visible_projects(state, p).into_iter().any(|id| perm::project_level(state, p, id, Tool::Estimating) >= Level::Standard)
        }
    }
}

fn need_write(ctx: &Ctx<'_>) -> Result<()> {
    if can_write(ctx.state, ctx.actor) {
        Ok(())
    } else {
        Err(EngineError::forbidden("keeping the catalogue takes Standard access to Estimating on a project"))
    }
}

fn cost_code(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<Id<CostCode>>> {
    let code: Option<Id<CostCode>> = p.opt_id("cost_code")?;
    if let Some(c) = code
        && !ctx.state.cost_codes.get(c).is_some_and(|x| x.deleted.is_none())
    {
        return Err(p.bad("cost_code", format!("cost code {c} not found")));
    }
    Ok(code)
}

fn positive(p: &Params<'_>, key: &str) -> Result<Money> {
    let m = p.money(key)?.ok_or_else(|| p.bad(key, "is required"))?;
    if m.is_negative() {
        return Err(p.bad(key, "must not be negative"));
    }
    Ok(m)
}

fn save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    need_write(ctx)?;
    let name = p.req_line("name", text::SHORT)?;
    let unit_cost = positive(p, "unit_cost")?;
    let unit = p.line_or_empty("unit", 20)?;
    let category = p.line_or_empty("category", 100)?;
    let notes = p.text_or_empty("notes", text::SHORT)?;
    let code = cost_code(ctx, p)?;
    let stamp = ctx.stamp();
    let existing = p.opt_id::<CatalogItem>("id")?;
    if ctx.state.catalog_items.iter().any(|i| i.deleted.is_none() && Some(i.id) != existing && i.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already in the catalogue"));
    }
    let id = match existing {
        Some(id) => {
            if !ctx.state.catalog_items.get(id).is_some_and(|i| i.deleted.is_none()) {
                return Err(EngineError::not_found(format!("catalogue item {id} not found")));
            }
            ctx.state.catalog_items.update(id, |i| {
                (i.name, i.unit, i.unit_cost, i.cost_code, i.category, i.notes, i.updated) = (name, unit, unit_cost, code, category, notes, stamp);
            });
            id
        }
        None => {
            if ctx.state.catalog_items.iter().filter(|i| i.deleted.is_none()).count() >= MAX_ITEMS {
                return Err(EngineError::conflict(format!("the catalogue holds at most {MAX_ITEMS} items")));
            }
            let id = ctx.state.catalog_items.alloc();
            ctx.state.catalog_items.put(CatalogItem {
                id,
                name,
                unit,
                unit_cost,
                cost_code: code,
                category,
                notes,
                created: stamp,
                updated: stamp,
                deleted: None,
            });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    need_write(ctx)?;
    let id: Id<CatalogItem> = p.id("id")?;
    if !ctx.state.catalog_items.get(id).is_some_and(|i| i.deleted.is_none()) {
        return Err(EngineError::not_found(format!("catalogue item {id} not found")));
    }
    if ctx.state.assemblies.iter().any(|a| a.deleted.is_none() && a.parts.iter().any(|x| x.item == id)) {
        return Err(EngineError::conflict("an assembly uses this item; take it out of the assembly first"));
    }
    let stamp = ctx.stamp();
    ctx.state.catalog_items.update(id, |i| i.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    need_write(ctx)?;
    let body = p.req_text("text", 1_000_000)?;
    /// One line of a price list: name, unit, unit cost, category and cost code.
    type Row = (String, String, Money, String, Option<Id<CostCode>>);
    let mut rows: Vec<Row> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cells = super::submittal::csv_cells(line);
        let get = |i: usize| cells.get(i).map(String::as_str).unwrap_or("");
        if rows.is_empty() && get(0).eq_ignore_ascii_case("name") {
            continue;
        }
        if rows.len() >= 5_000 {
            return Err(p.bad("text", "has more than 5,000 lines"));
        }
        let at = |what: &str, e: String| p.bad("text", format!("line {}: {what} {e}", n + 1));
        let name = text::required_line(get(0), text::SHORT).map_err(|e| at("name", e.to_string()))?;
        let unit = text::line(get(1), 20).map_err(|e| at("unit", e.to_string()))?;
        let cost = Money::parse(get(2)).map_err(|e| at("unit cost", e.to_string()))?;
        if cost.is_negative() {
            return Err(at("unit cost", "must not be negative".into()));
        }
        let category = text::line(get(3), 100).map_err(|e| at("category", e.to_string()))?;
        let code = match get(4) {
            "" => None,
            c => Some(
                view::company_cost_codes(ctx.state)
                    .into_iter()
                    .find(|x| x.code.eq_ignore_ascii_case(c))
                    .map(|x| x.id)
                    .ok_or_else(|| at("cost code", format!("`{c}` is not in the company's cost code list")))?,
            ),
        };
        rows.push((name, unit, cost, category, code));
    }
    let (mut added, mut changed) = (0usize, 0usize);
    let stamp = ctx.stamp();
    for (name, unit, unit_cost, category, code) in rows {
        let existing = ctx.state.catalog_items.iter().find(|i| i.deleted.is_none() && i.name.eq_ignore_ascii_case(&name)).map(|i| i.id);
        match existing {
            Some(id) => {
                ctx.state.catalog_items.update(id, |i| {
                    (i.unit, i.unit_cost, i.category, i.updated) = (unit, unit_cost, category, stamp);
                    if code.is_some() {
                        i.cost_code = code;
                    }
                });
                changed += 1;
            }
            None => {
                let id = ctx.state.catalog_items.alloc();
                ctx.state.catalog_items.put(CatalogItem {
                    id,
                    name,
                    unit,
                    unit_cost,
                    cost_code: code,
                    category,
                    notes: String::new(),
                    created: stamp,
                    updated: stamp,
                    deleted: None,
                });
                added += 1;
            }
        }
    }
    Ok(json!({ "added": added, "changed": changed, "created": added + changed }))
}

/// The catalogue, by category and name.
pub fn items(state: &CompanyState) -> Vec<&CatalogItem> {
    let mut v: Vec<&CatalogItem> = state.catalog_items.iter().filter(|i| i.deleted.is_none()).collect();
    v.sort_by_key(|i| (i.category.to_lowercase(), i.name.to_lowercase()));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    if !can_read(v.state, v.actor) {
        return Err(EngineError::forbidden("the catalogue is for people who work on estimates"));
    }
    let q = p.line("q", 100)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let category = p.line("category", 100)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let found: Vec<&CatalogItem> = items(v.state)
        .into_iter()
        .filter(|i| q.as_ref().is_none_or(|q| i.name.to_lowercase().contains(q.as_str())))
        .filter(|i| category.as_ref().is_none_or(|c| i.category.to_lowercase() == *c))
        .take(2_000)
        .collect();
    view::json(&found)
}

fn parts_from(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Vec<AssemblyPart>> {
    let rows = p.items("parts")?;
    if rows.is_empty() || rows.len() > MAX_PARTS {
        return Err(p.bad("parts", format!("needs 1 to {MAX_PARTS} parts")));
    }
    let mut out: Vec<AssemblyPart> = Vec::new();
    for r in rows {
        let q = r.line_or_empty("quantity", 30)?;
        let Some(item) = r.opt_id::<CatalogItem>("item")? else {
            if q.is_empty() {
                continue; // a spare blank row on the form
            }
            return Err(r.bad("item", "is required"));
        };
        if !ctx.state.catalog_items.get(item).is_some_and(|i| i.deleted.is_none()) {
            return Err(r.bad("item", format!("catalogue item {item} not found")));
        }
        if out.iter().any(|x| x.item == item) {
            return Err(r.bad("item", "is in the assembly twice; give it one line"));
        }
        let quantity_milli = if q.is_empty() {
            1000
        } else {
            parse_quantity(&q).filter(|v| *v > 0).ok_or_else(|| r.bad("quantity", format!("`{q}` is not a quantity above zero")))?
        };
        out.push(AssemblyPart { item, quantity_milli });
    }
    Ok(out)
}

fn assembly_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    need_write(ctx)?;
    let name = p.req_line("name", text::SHORT)?;
    let unit = p.line_or_empty("unit", 20)?;
    let notes = p.text_or_empty("notes", text::SHORT)?;
    let parts = parts_from(ctx, p)?;
    let stamp = ctx.stamp();
    let existing = p.opt_id::<Assembly>("id")?;
    if ctx.state.assemblies.iter().any(|a| a.deleted.is_none() && Some(a.id) != existing && a.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already used by another assembly"));
    }
    let id = match existing {
        Some(id) => {
            if !ctx.state.assemblies.get(id).is_some_and(|a| a.deleted.is_none()) {
                return Err(EngineError::not_found(format!("assembly {id} not found")));
            }
            ctx.state.assemblies.update(id, |a| (a.name, a.unit, a.notes, a.parts, a.updated) = (name, unit, notes, parts, stamp));
            id
        }
        None => {
            if ctx.state.assemblies.iter().filter(|a| a.deleted.is_none()).count() >= MAX_ASSEMBLIES {
                return Err(EngineError::conflict(format!("at most {MAX_ASSEMBLIES} assemblies")));
            }
            let id = ctx.state.assemblies.alloc();
            ctx.state.assemblies.put(Assembly { id, name, unit, notes, parts, created: stamp, updated: stamp, deleted: None });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn assembly_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    need_write(ctx)?;
    let id: Id<Assembly> = p.id("id")?;
    if !ctx.state.assemblies.get(id).is_some_and(|a| a.deleted.is_none()) {
        return Err(EngineError::not_found(format!("assembly {id} not found")));
    }
    let stamp = ctx.stamp();
    ctx.state.assemblies.update(id, |a| a.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// The assemblies, by name.
pub fn assemblies(state: &CompanyState) -> Vec<&Assembly> {
    let mut v: Vec<&Assembly> = state.assemblies.iter().filter(|a| a.deleted.is_none()).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

/// What one unit of the assembly costs at today's catalogue prices.
pub fn unit_cost(state: &CompanyState, a: &Assembly) -> Option<Money> {
    Money::sum(
        a.parts
            .iter()
            .map(|x| state.catalog_items.get(x.item).and_then(|i| i.unit_cost.checked_mul_int(x.quantity_milli)?.checked_div_int(1000)))
            .collect::<Option<Vec<_>>>()?,
    )
}

fn assembly_list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    if !can_read(v.state, v.actor) {
        return Err(EngineError::forbidden("assemblies are for people who work on estimates"));
    }
    let rows: Vec<Value> = assemblies(v.state)
        .into_iter()
        .map(|a| {
            let mut o = serde_json::to_value(a).unwrap_or(Value::Null);
            if let Some(m) = o.as_object_mut() {
                m.insert("unit_cost".into(), json!(unit_cost(v.state, a)));
            }
            o
        })
        .collect();
    Ok(Value::Array(rows))
}
