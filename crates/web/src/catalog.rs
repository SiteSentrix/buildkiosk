//! The company's price catalogue and assemblies, shared by every project's estimates.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::catalog::{assemblies, can_read, can_write, items, unit_cost};
use buildkiosk_engine::model::estimate::{Assembly, CatalogItem};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::not_found;
use buildkiosk_engine::model::Tool;

pub fn page(req: &Req<'_>, q: &str, edit_item: Option<Id<CatalogItem>>, edit_assembly: Option<Id<Assembly>>) -> Markup {
    let state = req.engine.state();
    if !can_read(state, req.actor()) {
        return not_found(req);
    }
    let write = can_write(state, req.actor());
    let needle = q.trim().to_lowercase();
    let all = items(state);
    let shown: Vec<&CatalogItem> = all
        .iter()
        .copied()
        .filter(|i| needle.is_empty() || i.name.to_lowercase().contains(&needle) || i.category.to_lowercase().contains(&needle))
        .take(500)
        .collect();
    let cur = edit_item.and_then(|id| all.iter().copied().find(|i| i.id == id));
    let asm = assemblies(state);
    let cur_asm = edit_assembly.and_then(|id| asm.iter().copied().find(|a| a.id == id));
    let mut codes: Vec<_> = buildkiosk_engine::view::company_cost_codes(state);
    codes.truncate(500);
    let part_rows = cur_asm.map(|a| a.parts.len()).unwrap_or(0) + 4;
    let here = "/catalog";
    let mut suppliers: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    suppliers.sort_by_key(|v| v.name.to_lowercase());
    suppliers.truncate(500);
    let body = html! {
        div class="page-head" { div { h1 { "Price catalogue" } div class="sub" { "Unit costs and assemblies, shared by every project's estimates" } } }
        form method="get" action=(here) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search name or category" aria-label="Search the catalogue";
            button type="submit" class="btn small" { "Search" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Estimating), "Nothing in the catalogue", if write { "Add an item below, or import a list of prices." } else { "Nothing has been added yet." })) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Item" } th { "Category" } th { "Unit" } th class="num" { "Unit cost" } th { "Cost code" } th {} } }
                tbody { @for i in &shown { tr {
                    td { strong { (i.name) } @if !i.notes.is_empty() { br; span class="muted small" { (i.notes) } } }
                    td { (i.category) } td { (i.unit) } td class="num" { (i.unit_cost) }
                    td { (i.cost_code.and_then(|c| state.cost_codes.get(c)).map(|c| c.code.clone()).unwrap_or_default()) }
                    td class="right" { @if write { a class="btn small" href=(format!("{here}?edit={}", i.id)) { "Edit" } " " (button_form(req, "catalog.delete", here, &[("id", i.id.to_string())], "Delete", "small danger")) } }
                } } }
            } }
        }
        @if write {
            div class="card" {
                h2 { (if cur.is_some() { "Edit item" } else { "Add an item" }) }
                (command_form(req, "catalog.save", here, here, html! {
                    @if let Some(i) = cur { input type="hidden" name="id" value=(i.id); }
                    div class="form-grid" {
                        (field("Name", "ci_name", true, true, html! { input type="text" id="ci_name" name="name" required value=(cur.map(|i| i.name.as_str()).unwrap_or_default()); }))
                        (field("Unit cost", "ci_cost", true, false, html! { input type="text" inputmode="decimal" id="ci_cost" name="unit_cost" required value=(cur.map(|i| i.unit_cost.plain()).unwrap_or_default()); }))
                        (field("Unit", "ci_unit", false, false, html! { input type="text" id="ci_unit" name="unit" placeholder="sf" value=(cur.map(|i| i.unit.as_str()).unwrap_or_default()); }))
                        (field("Category", "ci_cat", false, false, html! { input type="text" id="ci_cat" name="category" value=(cur.map(|i| i.category.as_str()).unwrap_or_default()); }))
                        (field("Cost code", "ci_code", false, false, html! { select id="ci_code" name="cost_code" { option value="" { "None" } @for c in &codes { option value=(c.id) selected[cur.is_some_and(|i| i.cost_code == Some(c.id))] { (c.code) " " (c.description) } } } }))
                        (field("Notes", "ci_notes", false, true, html! { input type="text" id="ci_notes" name="notes" value=(cur.map(|i| i.notes.as_str()).unwrap_or_default()); }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save item" } }
                }))
            }
            @if let Some(i) = cur {
                div class="card" {
                    h2 { "Prices for " (i.name) }
                    @if i.prices.is_empty() { p class="muted" { "No prices are recorded by date yet." } } @else {
                        table class="data" {
                            thead { tr { th { "From" } th class="num" { "Unit cost" } th { "Supplier" } th { "Note" } } }
                            tbody { @for pt in i.prices.iter().rev() { tr {
                                td class="nowrap" { (pt.date.pretty()) } td class="num" { (pt.unit_cost) }
                                td { (pt.vendor.map(|v| buildkiosk_engine::view::vendor_name(state, v)).unwrap_or_default()) } td { (pt.note) }
                            } } }
                        }
                        p class="muted small" { "The newest price is the item's unit cost; an estimate line copies the price on the day it is made." }
                    }
                    (command_form(req, "catalog.price", here, here, html! {
                        input type="hidden" name="id" value=(i.id);
                        div class="form-grid" {
                            (field("Unit cost", "cp_cost", true, false, html! { input type="text" inputmode="decimal" id="cp_cost" name="unit_cost" required; }))
                            (field("From", "cp_date", false, false, html! { input type="date" id="cp_date" name="date" value=(req.today().to_string()); }))
                            (field("Supplier", "cp_vendor", false, false, html! { select id="cp_vendor" name="vendor" { option value="" { "None" } @for v in &suppliers { option value=(v.id) { (v.name) } } } }))
                            (field("Note", "cp_note", false, false, html! { input type="text" id="cp_note" name="note"; }))
                        }
                        div class="form-actions" { button type="submit" class="btn small primary" { "Record a price" } }
                    }))
                }
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a price list" }
                    p class="muted small" { "One item per line: name, unit, unit cost, category (optional), cost code (optional, from the company's standard list). An item with the same name has its price changed." }
                    (command_form(req, "catalog.import", here, here, html! {
                        div class="form-grid" {
                            (field("Supplier (if it is their price list)", "ci_vendor", false, false, html! { select id="ci_vendor" name="vendor" { option value="" { "None" } @for v in &suppliers { option value=(v.id) { (v.name) } } } }))
                            (field("Prices are from", "ci_date", false, false, html! { input type="date" id="ci_date" name="date" value=(req.today().to_string()); }))
                        }
                        (field("Lines", "ci_import", true, true, html! { textarea id="ci_import" name="text" required placeholder="Ready-mix concrete, cy, 185, Concrete, 03-300" {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
        div class="card" {
            h2 { "Assemblies" }
            @if asm.is_empty() { p class="muted" { "No assemblies yet. An assembly prices a unit of work from several catalogue items." } } @else {
                table class="data" {
                    thead { tr { th { "Assembly" } th { "Parts" } th class="num" { "Cost per unit" } th {} } }
                    tbody { @for a in &asm { tr {
                        td { strong { (a.name) } @if !a.unit.is_empty() { " " span class="muted small" { "per " (a.unit) } } }
                        td { (a.parts.iter().filter_map(|p| state.catalog_items.get(p.item).map(|i| i.name.clone())).collect::<Vec<_>>().join(", ")) }
                        td class="num" { @if let Some(c) = unit_cost(state, a) { (c) } }
                        td class="right" { @if write { a class="btn small" href=(format!("{here}?assembly={}", a.id)) { "Edit" } " " (button_form(req, "assembly.delete", here, &[("id", a.id.to_string())], "Delete", "small danger")) } }
                    } } }
                }
            }
        }
        @if write && !all.is_empty() {
            div class="card" {
                h2 { (if cur_asm.is_some() { "Edit assembly" } else { "Add an assembly" }) }
                (command_form(req, "assembly.save", here, here, html! {
                    @if let Some(a) = cur_asm { input type="hidden" name="id" value=(a.id); }
                    div class="form-grid" {
                        (field("Name", "as_name", true, true, html! { input type="text" id="as_name" name="name" required placeholder="Slab on grade" value=(cur_asm.map(|a| a.name.as_str()).unwrap_or_default()); }))
                        (field("One unit is", "as_unit", false, false, html! { input type="text" id="as_unit" name="unit" placeholder="sf" value=(cur_asm.map(|a| a.unit.as_str()).unwrap_or_default()); }))
                    }
                    table class="data" {
                        thead { tr { th { "Item" } th class="num" { "Quantity per unit" } } }
                        tbody { @for i in 0..part_rows { @let part = cur_asm.and_then(|a| a.parts.get(i)); tr {
                            td { select name=(format!("parts.{i}.item")) aria-label=(format!("Part {} item", i + 1)) { option value="" { "—" } @for it in all.iter().take(500) { option value=(it.id) selected[part.is_some_and(|x| x.item == it.id)] { (it.name) } } } }
                            td { input type="text" inputmode="decimal" name=(format!("parts.{i}.quantity")) value=(part.map(|x| format!("{}.{:03}", x.quantity_milli / 1000, x.quantity_milli % 1000)).unwrap_or_default()) aria-label=(format!("Part {} quantity", i + 1)); }
                        } } }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save assembly" } }
                }))
            }
        }
    };
    layout::app(req, "Price catalogue", Nav::Projects, &[("Projects", Some("/".into())), ("Price catalogue", None)], body)
}
