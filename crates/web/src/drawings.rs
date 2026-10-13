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
use crate::pages::{dash, human_size, not_found, project_for, user_options};

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
            div class="actions" { a class="btn" href=(format!("{base}.pdf")) { "PDF" } a class="btn" href=(format!("{base}.csv")) { "Export CSV" } }
        }
        form method="get" action=(base) class="filters" {
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
                h2 { "Upload a set of sheets" }
                p class="muted" { "Name each file with its sheet number first, such as “E-101 Power plan.pdf”. New sheet numbers are added to the log (the discipline is guessed from the letters); sheets already logged get a new revision." }
                form method="post" action=(format!("{base}/upload-set")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-grid" {
                        (field("Revision", "set_revision", true, false, html! { input type="text" id="set_revision" name="revision" required placeholder="A, 2, Addendum 3"; }))
                        (field("Set", "set_name", false, false, html! { input type="text" id="set_name" name="set" placeholder="Permit set"; }))
                        (field("Issue date", "set_date", false, false, html! { input type="date" id="set_date" name="date"; }))
                        (field("Sheet files", "set_files", true, false, html! { input type="file" id="set_files" name="file" multiple required; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Upload the set" } }
                }
                p class="muted small" { "Up to 50 files, 25 MB each and 100 MB in all." }
            }
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
            div class="card-head" {
                h2 { "Revisions" }
                @if d.revisions.iter().any(|r| r.file.is_some()) { a class="btn small" href=(format!("{here}/view")) { "Open in viewer" } }
            }
            @if d.revisions.is_empty() {
                (empty(tool_icon(Tool::Drawings), "No revisions yet", "Add the first revision below."))
            } @else {
                div class="table-wrap" {
                    table class="data" {
                        thead { tr { th { "Revision" } th { "Date" } th { "Set" } th { "File" } th class="right" { "Size" } th { "Added" } th { "By" } th { "Note" } } }
                        tbody {
                            @for (i, r) in d.revisions.iter().enumerate().rev() {
                                tr {
                                    td class="nowrap" { (r.label) @if i + 1 == d.revisions.len() { " " span class="pill blue" { "Current" } } }
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
        @let linked_rfis: Vec<_> = view::rfis_for(state, req.actor(), id).into_iter().filter(|r| !r.drawing_number.is_empty() && r.drawing_number.eq_ignore_ascii_case(&d.number)).collect();
        @let linked_punch: Vec<_> = buildkiosk_engine::workitem::items_for::<buildkiosk_engine::model::punch::PunchItem>(state, req.actor(), id).into_iter().filter(|i| i.drawing_number.eq_ignore_ascii_case(&d.number)).collect();
        @let linked_obs: Vec<_> = buildkiosk_engine::workitem::items_for::<buildkiosk_engine::model::observation::Observation>(state, req.actor(), id).into_iter().filter(|i| i.drawing_number.eq_ignore_ascii_case(&d.number)).collect();
        @if !linked_rfis.is_empty() || !linked_punch.is_empty() || !linked_obs.is_empty() {
            div class="card" {
                h2 { "Linked to this sheet" }
                ul class="plain" {
                    @for r in &linked_rfis { li { a href=(format!("/projects/{id}/rfis/{}", r.id)) { "RFI #" (r.number) ": " (r.subject) } } }
                    @for i in &linked_punch { li { a href=(format!("/projects/{id}/punch-list/{}", i.id)) { "Punch item #" (i.number) ": " (i.title) } } }
                    @for i in &linked_obs { li { a href=(format!("/projects/{id}/observations/{}", i.id)) { "Observation #" (i.number) ": " (i.title) } } }
                }
            }
        }
        @if can_edit && !d.revisions.is_empty() {
            @let groups: Vec<_> = state.groups.iter().filter(|g| g.deleted.is_none() && g.project.is_none_or(|p| p == id)).collect();
            div class="card" {
                h2 { "Tell people the current revision was issued" }
                (command_form(req, "drawing.publish", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(did);
                    input type="hidden" name="people[]" value="";
                    details class="multi" {
                        summary { "Choose people" }
                        div class="options" { @for x in user_options(req, id, true) { label { input type="checkbox" name="people[]" value=(x.id); (x.name()) } } }
                    }
                    @if !groups.is_empty() {
                        (field("Or a group", "publish_group", false, false, html! { select id="publish_group" name="groups[]" { option value="" { "—" } @for g in &groups { option value=(g.id) { (g.name) " (" (g.members.len()) ")" } } } }))
                    }
                    (field("Message", "publish_message", false, false, html! { input type="text" id="publish_message" name="message" maxlength="300" placeholder="What changed, or what to do about it"; }))
                    div class="form-actions" { button type="submit" class="btn" { "Send notifications" } }
                }))
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

// -- the sheet viewer

/// What the viewer was asked to show, from the query string.
#[derive(Clone, Default)]
pub struct View {
    /// The revision label ("" for the current one).
    pub rev: String,
    /// The page, from 1.
    pub page: u32,
    /// Display width in pixels.
    pub width: u32,
    /// `pin`, `rect`, `arrow`, `text`, or "" to just look.
    pub tool: String,
    /// The first corner of a box or arrow, when the first click has been made.
    pub first: Option<(u16, u16)>,
    /// The point just clicked.
    pub click: Option<(u16, u16)>,
    /// Another revision to lay over this one ("" for none).
    pub compare: String,
}

/// The size of the page being shown, in any unit (only the shape matters).
#[derive(Clone, Copy)]
pub struct PageShape {
    pub pages: usize,
    pub width: f32,
    pub height: f32,
}

fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

impl View {
    /// The query string for this view (without a click).
    pub fn query(&self) -> String {
        let mut q = vec![format!("page={}", self.page.max(1)), format!("w={}", self.width)];
        if !self.rev.is_empty() {
            q.push(format!("rev={}", enc(&self.rev)));
        }
        if !self.tool.is_empty() {
            q.push(format!("tool={}", enc(&self.tool)));
        }
        if let Some((x, y)) = self.first {
            q.push(format!("x1={x}&y1={y}"));
        }
        if !self.compare.is_empty() {
            q.push(format!("compare={}", enc(&self.compare)));
        }
        q.join("&")
    }
}

/// The viewer for one revision of a sheet: the page as a picture with the markups drawn over it,
/// tools that place new ones by clicking the picture (no scripts: the click is sent as a form),
/// and a comparison with another revision.
pub fn viewer(req: &Req<'_>, id: Id<Project>, did: Id<Drawing>, v: &View, shape: Option<PageShape>) -> Markup {
    use buildkiosk_engine::model::drawing::{MARKUP_SCALE, MarkupKind};
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Drawings);
    let Some(d) = state.drawings.get(did).filter(|d| d.project == id && d.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let sheet = format!("/projects/{id}/drawings/{did}");
    let here = format!("{sheet}/view");
    let rev = if v.rev.is_empty() { d.revisions.last() } else { d.revisions.iter().find(|r| r.label.eq_ignore_ascii_case(&v.rev)) };
    let can_mark = lvl >= Level::Standard;
    let link = |f: &dyn Fn(&mut View)| {
        let mut n = v.clone();
        n.click = None;
        f(&mut n);
        format!("{here}?{}", n.query())
    };
    let body = html! {
        div class="page-head" {
            div { h1 { (d.number) @if !d.title.is_empty() { " · " (d.title) } } div class="sub" { "Sheet viewer" @if let Some(r) = rev { " · revision " (r.label) } } }
            div class="actions" { a class="btn" href=(sheet.clone()) { "Back to the sheet" } }
        }
        @if let (Some(r), Some(shape)) = (rev, shape) {
            @let page = v.page.clamp(1, u32::try_from(shape.pages).unwrap_or(1));
            @let width = v.width;
            @let height = ((width as f32) * shape.height / shape.width).round().max(1.0) as u32;
            @let rev_q = if v.rev.is_empty() { String::new() } else { format!("&rev={}", enc(&v.rev)) };
            div class="viewer-bar" {
                span class="muted" { "Revision" }
                @for (i, x) in d.revisions.iter().enumerate().rev() {
                    a class=(if x.label == r.label { "chip active" } else { "chip" }) href=(link(&|n| { n.rev = if i + 1 == d.revisions.len() { String::new() } else { x.label.clone() }; n.first = None; n.compare = String::new(); })) { (x.label) }
                }
                @if shape.pages > 1 {
                    span class="muted" { "Page" }
                    @if page > 1 { a class="chip" href=(link(&|n| { n.page = page - 1; n.first = None; })) { "‹ Prev" } }
                    span { (page) " of " (shape.pages) }
                    @if (page as usize) < shape.pages { a class="chip" href=(link(&|n| { n.page = page + 1; n.first = None; })) { "Next ›" } }
                }
                span class="muted" { "Zoom" }
                @for (label, w) in [("Fit", 900u32), ("Large", 1400), ("Detail", 2000)] {
                    a class=(if width == w { "chip active" } else { "chip" }) href=(link(&|n| { n.width = w; })) { (label) }
                }
            }
            div class="viewer-bar" {
                span class="muted" { "Mark up" }
                a class=(if v.tool.is_empty() { "chip active" } else { "chip" }) href=(link(&|n| { n.tool = String::new(); n.first = None; })) { "View only" }
                @if can_mark && v.compare.is_empty() {
                    @for (key, label) in [("pin", "Pin"), ("rect", "Box"), ("arrow", "Arrow"), ("text", "Text")] {
                        a class=(if v.tool == key { "chip active" } else { "chip" }) href=(link(&|n| { n.tool = key.to_string(); n.first = None; })) { (label) }
                    }
                }
                @if d.revisions.len() > 1 {
                    span class="muted" { "Compare with" }
                    @if v.compare.is_empty() {
                        @for x in d.revisions.iter().filter(|x| x.label != r.label && x.file.is_some()) {
                            a class="chip" href=(link(&|n| { n.compare = x.label.clone(); n.tool = String::new(); n.first = None; })) { (x.label) }
                        }
                    } @else {
                        a class="chip active" href=(link(&|n| { n.compare = String::new(); })) { (v.compare) " ✕" }
                    }
                }
            }
            @if !v.compare.is_empty() {
                p class="muted" { span class="swatch red" {} " only on revision " (r.label) " · " span class="swatch blue" {} " only on revision " (v.compare) " · " span class="swatch gray" {} " on both. Revisions are laid on top of each other at the page's size; a sheet that moved or was rescaled shows as changed everywhere." }
                div class="sheet-scroll" { img src=(format!("{here}/compare.png?a={}&b={}&page={page}&w={width}", enc(&v.compare), enc(&r.label))) width=(width) height=(height) alt="Differences between the two revisions"; }
            } @else {
                @let tool_hint = match (v.tool.as_str(), v.first) {
                    ("pin", _) => "Click the picture where the pin goes.",
                    ("text", _) => "Click where the words go.",
                    ("rect", None) => "Click one corner of the box.",
                    ("rect", Some(_)) => "Click the opposite corner.",
                    ("arrow", None) => "Click where the arrow starts.",
                    ("arrow", Some(_)) => "Click where the arrow points.",
                    _ => "",
                };
                @if !tool_hint.is_empty() { p class="notice" { (tool_hint) } }
                @let marks: Vec<_> = r.markups.iter().filter(|m| m.page == page).collect();
                @let scale = |a: u16, total: u32| (f64::from(a) * f64::from(total) / f64::from(MARKUP_SCALE)).round();
                @let drawing = html! {
                    svg class="sheet-overlay" width=(width) height=(height) viewBox=(format!("0 0 {width} {height}")) aria-hidden="true" {
                        defs { marker id="head" markerWidth="10" markerHeight="10" refX="8" refY="5" orient="auto" { path d="M0,0 L10,5 L0,10 z" fill="#c62828" {} } }
                        @for m in &marks {
                            @let (x, y, x2, y2) = (scale(m.x, width), scale(m.y, height), scale(m.x2, width), scale(m.y2, height));
                            @let color = if m.resolved { "#7b8794" } else { "#c62828" };
                            @match m.kind {
                                MarkupKind::Pin => { circle cx=(x) cy=(y) r="11" fill="white" stroke=(color) stroke-width="2" {} text x=(x) y=(y + 4.0) text-anchor="middle" font-size="12" font-weight="700" fill=(color) { (m.id) } }
                                MarkupKind::Rectangle => { rect x=(x.min(x2)) y=(y.min(y2)) width=((x - x2).abs()) height=((y - y2).abs()) fill="none" stroke=(color) stroke-width="2" {} text x=(x.min(x2) + 3.0) y=(y.min(y2) - 4.0) font-size="12" font-weight="700" fill=(color) { (m.id) } }
                                MarkupKind::Arrow => { line x1=(x) y1=(y) x2=(x2) y2=(y2) stroke=(color) stroke-width="2" marker-end="url(#head)" {} text x=(x + 4.0) y=(y - 4.0) font-size="12" font-weight="700" fill=(color) { (m.id) } }
                                MarkupKind::Text => { text x=(x) y=(y) font-size="16" font-weight="600" fill=(color) { (m.id) ". " (m.text) } }
                            }
                        }
                        @if let Some((fx, fy)) = v.first { circle cx=(scale(fx, width)) cy=(scale(fy, height)) r="5" fill="#1565c0" {} }
                        @if let Some((cx, cy)) = v.click { circle cx=(scale(cx, width)) cy=(scale(cy, height)) r="7" fill="none" stroke="#1565c0" stroke-width="3" {} }
                    }
                };
                div class="sheet-scroll" {
                    div class="sheet-view" {
                        @if v.tool.is_empty() || !can_mark {
                            img src=(format!("{here}.png?page={page}&w={width}{rev_q}")) width=(width) height=(height) alt=(format!("Sheet {} page {page}", d.number));
                        } @else {
                            form method="get" action=(here.clone()) {
                                input type="hidden" name="page" value=(page);
                                input type="hidden" name="w" value=(width);
                                input type="hidden" name="rev" value=(v.rev);
                                input type="hidden" name="tool" value=(v.tool);
                                @if let Some((x, y)) = v.first { input type="hidden" name="x1" value=(x); input type="hidden" name="y1" value=(y); }
                                input type="image" name="at" src=(format!("{here}.png?page={page}&w={width}{rev_q}")) width=(width) height=(height) alt="Click to place the markup";
                            }
                        }
                        (drawing)
                    }
                }
                @if let Some((cx, cy)) = v.click {
                    @let two = matches!(v.tool.as_str(), "rect" | "arrow");
                    div class="card" {
                        h2 { "Add " (match v.tool.as_str() { "pin" => "a pin", "rect" => "a box", "arrow" => "an arrow", _ => "text" }) }
                        (command_form(req, "drawing.markup_add", &format!("{here}?{}", { let mut n = v.clone(); n.first = None; n.query() }), &format!("{here}?{}", v.query()), html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(did);
                            input type="hidden" name="revision" value=(r.label);
                            input type="hidden" name="page" value=(page);
                            input type="hidden" name="kind" value=(v.tool);
                            @if two {
                                @let (fx, fy) = v.first.unwrap_or((cx, cy));
                                input type="hidden" name="x" value=(fx); input type="hidden" name="y" value=(fy);
                                input type="hidden" name="x2" value=(cx); input type="hidden" name="y2" value=(cy);
                            } @else {
                                input type="hidden" name="x" value=(cx); input type="hidden" name="y" value=(cy);
                            }
                            (field(if v.tool == "text" { "Words" } else { "Note (optional)" }, "markup_text", v.tool == "text", true, html! { input type="text" id="markup_text" name="text" maxlength="1000" required[v.tool == "text"]; }))
                            div class="form-actions" {
                                a class="btn" href=(link(&|n| { n.first = None; })) { "Cancel" }
                                button type="submit" class="btn primary" { "Save markup" }
                            }
                        }))
                    }
                }
                div class="card" {
                    h2 { "Markups on this page" }
                    @if marks.is_empty() { p class="muted" { "None yet." @if can_mark { " Pick a tool above and click the sheet." } } }
                    @for m in &marks {
                        div class="response" {
                            div class="meta" {
                                span { strong { "#" (m.id) " " (m.kind.label()) } " · " @if let Some(b) = m.by { (view::person_name(state, b)) } " · " (req.when(m.at)) }
                                @if m.resolved { span class="pill green" { "Resolved" } }
                            }
                            @if !m.text.is_empty() { p class="prose" { (m.text) } }
                            @if can_mark {
                                div class="row" {
                                    @let ids = [("project", id.to_string()), ("id", did.to_string()), ("revision", r.label.clone()), ("markup", m.id.to_string())];
                                    @let back = format!("{here}?{}", v.query());
                                    (button_form(req, "drawing.markup_resolve", &back, &[ids[0].clone(), ids[1].clone(), ids[2].clone(), ids[3].clone(), ("resolved", (!m.resolved).to_string())], if m.resolved { "Reopen" } else { "Resolve" }, "small"))
                                    (button_form(req, "drawing.markup_delete", &back, &ids, "Remove", "small danger"))
                                }
                            }
                        }
                    }
                }
            }
        } @else {
            div class="card" { (empty(tool_icon(Tool::Drawings), "This sheet can't be shown", "It has no file yet, or the file is not a PDF or a picture (PNG, JPEG, GIF or WebP) the viewer can draw. Download it from the sheet's page instead.")) }
        }
    };
    layout::app(
        req,
        &d.number,
        Nav::Drawings(id),
        &[
            ("Projects", Some("/".into())),
            (&p.name, Some(format!("/projects/{id}"))),
            ("Drawings", Some(format!("/projects/{id}/drawings"))),
            (&d.number, Some(sheet)),
            ("Viewer", None),
        ],
        body,
    )
}
