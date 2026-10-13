//! The drawings pages: the sheet log, each sheet's revisions, import and CSV export.

use std::collections::HashMap;

use buildkiosk_core::Id;
use buildkiosk_engine::model::drawing::{Discipline, Drawing};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, empty, field};
use crate::pages::{dash, human_size, not_found, project_for};

fn discipline_select(selected: Option<Discipline>, any: bool) -> Markup {
    html! {
        @if any { option value="" selected[selected.is_none()] { "All disciplines" } }
        @for d in Discipline::ALL { option value=(d.key()) selected[selected == Some(*d)] { (d.label()) } }
    }
}

pub fn page(req: &Req<'_>, id: Id<Project>, q: &HashMap<String, String>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Drawings);
    if lvl < Level::Read {
        return not_found(req);
    }
    let text = q.get("q").map(|s| s.trim().to_lowercase()).unwrap_or_default();
    let disc = q.get("discipline").and_then(|d| Discipline::parse(d));
    let area = q.get("area").map(|a| a.trim().to_string()).filter(|a| !a.is_empty());
    let all: Vec<&Drawing> = state.drawings.iter().filter(|d| d.project == id && d.deleted.is_none()).collect();
    let mut areas: Vec<&str> = all.iter().map(|d| d.area.as_str()).filter(|a| !a.is_empty()).collect();
    areas.sort_by_key(|a| a.to_lowercase());
    areas.dedup();
    let mut shown: Vec<&Drawing> = all
        .iter()
        .copied()
        .filter(|d| disc.is_none_or(|x| d.discipline == x))
        .filter(|d| area.as_ref().is_none_or(|a| d.area.eq_ignore_ascii_case(a)))
        .filter(|d| text.is_empty() || d.number.to_lowercase().contains(&text) || d.title.to_lowercase().contains(&text))
        .collect();
    shown.sort_by_key(|d| d.number.to_lowercase());
    let base = format!("/projects/{id}/drawings");
    let can_add = lvl >= Level::Standard;
    let body = html! {
        div class="page-head" {
            div { h1 { "Drawings" } div class="sub" { "The sheet log: each sheet with its revisions; the newest revision is current" } }
            div class="actions" { a class="btn" href=(format!("{base}.csv")) { "Export CSV" } }
        }
        form method="get" action=(base) class="toolbar" {
            input type="search" name="q" value=(text) placeholder="Search sheet number or title" aria-label="Search drawings";
            select name="discipline" aria-label="Discipline" { (discipline_select(disc, true)) }
            select name="area" aria-label="Area" {
                option value="" { "All areas" }
                @for a in &areas { option value=(a) selected[area.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(a))] { (a) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Drawings), if all.is_empty() { "No drawings yet" } else { "No sheets match" }, if all.is_empty() && can_add { "Add a sheet below, or import your drawing log." } else { "Change the filters to see more." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Sheet" } th { "Title" } th { "Discipline" } th { "Area" } th { "Revision" } th { "Date" } th { "Set" } th { "File" } } }
                    tbody {
                        @for d in &shown {
                            @let cur = d.current();
                            tr {
                                td class="nowrap" { a href=(format!("{base}/{}", d.id)) { (d.number) } }
                                td { (d.title) }
                                td { (d.discipline.label()) }
                                td { (dash(&d.area)) }
                                td { @if let Some(c) = cur { (c.label) } @else { span class="muted" { "none yet" } } }
                                td class="nowrap" { (cur.and_then(|c| c.date).map(|x| x.to_string()).unwrap_or_default()) }
                                td { (cur.map(|c| dash(&c.set)).unwrap_or("")) }
                                td { @if let Some(f) = cur.and_then(|c| c.file.as_ref()) { a href=(format!("{base}/{}/files/{}", d.id, f.sha256)) { (f.name) } } }
                            }
                        }
                    }
                }
            }
        }
        @if can_add {
            div class="card" {
                h2 { "Add a sheet" }
                form method="post" action=(format!("{base}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-grid" {
                        (field("Sheet number", "number", true, false, html! { input type="text" id="number" name="number" required; }))
                        (field("Title", "title", false, false, html! { input type="text" id="title" name="title"; }))
                        (field("Discipline", "discipline", false, false, html! { select id="discipline" name="discipline" { (discipline_select(Some(Discipline::General), false)) } }))
                        (field("Area", "area", false, false, html! { input type="text" id="area" name="area" list="areas"; datalist id="areas" { @for a in &areas { option value=(a) {} } } }))
                        (field("Revision (leave blank to log the sheet only)", "revision", false, false, html! { input type="text" id="revision" name="revision"; }))
                        (field("Issue date", "date", false, false, html! { input type="date" id="date" name="date"; }))
                        (field("Set", "set", false, false, html! { input type="text" id="set" name="set"; }))
                        (field("Sheet file", "file", false, false, html! { input type="file" id="file" name="file"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add sheet" } }
                }
                p class="muted small" { "Files can be at most 25 MB." }
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a drawing log" }
                    p class="muted small" { "One sheet per line: number, title, discipline, area, separated by commas or tabs. A header line is skipped, and sheets already in the log are left alone. Add each sheet's file later as a revision." }
                    (command_form(req, "drawing.import", &base, &base, html! {
                        input type="hidden" name="project" value=(id);
                        (field("Sheets", "drawing_text", true, true, html! { textarea id="drawing_text" name="text" required {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
    };
    layout::app(
        req,
        "Drawings",
        Nav::Drawings(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Drawings", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, did: Id<Drawing>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Drawings);
    let Some(d) = state.drawings.get(did).filter(|d| d.project == id && d.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/drawings");
    let here = format!("{base}/{did}");
    let can_edit = lvl >= Level::Standard;
    let body = html! {
        div class="page-head" { div { h1 { (d.number) @if !d.title.is_empty() { " · " (d.title) } } div class="sub" { (d.discipline.label()) @if !d.area.is_empty() { " · " (d.area) } } } }
        div class="card" {
            h2 { "Revisions" }
            @if d.revisions.is_empty() {
                (empty(tool_icon(Tool::Drawings), "No revisions yet", "Add the first revision below."))
            } @else {
                div class="table-wrap" {
                    table class="data" {
                        thead { tr { th { "Revision" } th { "Date" } th { "Set" } th { "File" } th class="right" { "Size" } th { "Added" } th { "By" } th { "Note" } } }
                        tbody {
                            @for (i, r) in d.revisions.iter().enumerate().rev() {
                                tr {
                                    td class="nowrap" { (r.label) @if i + 1 == d.revisions.len() { " " span class="chip" { "Current" } } }
                                    td class="nowrap" { (r.date.map(|x| x.to_string()).unwrap_or_default()) }
                                    td { (dash(&r.set)) }
                                    td { @if let Some(f) = &r.file { a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } } @else { span class="muted" { "no file" } } }
                                    td class="right nowrap" { (r.file.as_ref().map(|f| human_size(f.size)).unwrap_or_default()) }
                                    td class="nowrap" { (req.when(r.at)) }
                                    td { (r.by.map(|b| view::person_name(state, b)).unwrap_or_default()) }
                                    td { (dash(&r.note)) }
                                }
                            }
                        }
                    }
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Add a revision" }
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-grid" {
                        (field("Revision", "revision", true, false, html! { input type="text" id="revision" name="revision" required; }))
                        (field("Issue date", "date", false, false, html! { input type="date" id="date" name="date"; }))
                        (field("Set", "set", false, false, html! { input type="text" id="set" name="set"; }))
                        (field("Sheet file", "file", false, false, html! { input type="file" id="file" name="file"; }))
                        (field("What changed", "note", false, true, html! { input type="text" id="note" name="note"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add revision" } }
                }
            }
            div class="card" {
                h2 { "Details" }
                (command_form(req, "drawing.update", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(did);
                    div class="form-grid" {
                        (field("Sheet number", "e_number", true, false, html! { input type="text" id="e_number" name="number" value=(d.number) required; }))
                        (field("Title", "e_title", false, false, html! { input type="text" id="e_title" name="title" value=(d.title); }))
                        (field("Discipline", "e_discipline", false, false, html! { select id="e_discipline" name="discipline" { (discipline_select(Some(d.discipline), false)) } }))
                        (field("Area", "e_area", false, false, html! { input type="text" id="e_area" name="area" value=(d.area); }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                }))
                @if req.allowed("drawing.delete", Some(id)) {
                    (button_form(req, "drawing.delete", &base, &[("project", id.to_string()), ("id", did.to_string())], "Delete sheet", "small danger"))
                }
            }
        }
    };
    layout::app(
        req,
        &d.number,
        Nav::Drawings(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Drawings", Some(base)), (&d.number, None)],
        body,
    )
}

/// The drawing log as CSV.
pub fn csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Drawings) < Level::Read {
        return None;
    }
    let mut out = String::from("Sheet,Title,Discipline,Area,Current revision,Revision date,Set,File,Revisions\r\n");
    let mut list: Vec<&Drawing> = state.drawings.iter().filter(|d| d.project == id && d.deleted.is_none()).collect();
    list.sort_by_key(|d| d.number.to_lowercase());
    for d in list {
        let cur = d.current();
        let row = [
            d.number.clone(),
            d.title.clone(),
            d.discipline.label().to_string(),
            d.area.clone(),
            cur.map(|c| c.label.clone()).unwrap_or_default(),
            cur.and_then(|c| c.date).map(|x| x.to_string()).unwrap_or_default(),
            cur.map(|c| c.set.clone()).unwrap_or_default(),
            cur.and_then(|c| c.file.as_ref()).map(|f| f.name.clone()).unwrap_or_default(),
            d.revisions.len().to_string(),
        ];
        out.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
