//! The project report: every tool the person can read, with how many items are open or late.

use buildkiosk_core::Id;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::report::project_summary;
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::layout::{self, Nav, Req, csv_cell};
use crate::pages::{not_found, project_for};

pub fn page(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Reports) < Level::Read {
        return not_found(req);
    }
    let rows = project_summary(state, req.actor(), id, req.today());
    let base = format!("/projects/{id}");
    let (open, overdue): (usize, usize) = rows.iter().fold((0, 0), |a, r| (a.0 + r.open, a.1 + r.overdue));
    let body = html! {
        div class="page-head" {
            div { h1 { "Project report" } div class="sub" { "Where each tool stands today, for the tools you can see" } }
            div class="actions" { a class="btn" href=(format!("{base}/reports.pdf")) { "PDF" } a class="btn" href=(format!("{base}/reports.csv")) { "Export CSV" } }
        }
        div class="stat-row" {
            div class="stat" { div class="label" { "Items needing action" } div class="value" { (open) } }
            div class=(if overdue > 0 { "stat alert" } else { "stat good" }) { div class="label" { "Past due" } div class="value" { (overdue) } }
        }
        div class="table-wrap" { table class="data" {
            thead { tr { th { "Tool" } th class="num" { "Total" } th class="num" { "Open" } th class="num" { "Past due" } } }
            tbody { @for r in &rows { tr {
                td { a href=(format!("{base}/{}", r.path)) { (r.tool) } }
                td class="num" { (r.total) } td class="num" { (r.open) }
                td class="num" { @if r.overdue > 0 { span class="overdue" { (r.overdue) } } @else { (r.overdue) } }
            } } }
        } }
    };
    layout::app(req, "Project report", Nav::Reports(id), &[("Projects", Some("/".into())), (&p.name, Some(base)), ("Report", None)], body)
}

pub fn csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Reports) < Level::Read {
        return None;
    }
    let mut out = String::from("Tool,Total,Open,Past due\r\n");
    for r in project_summary(state, req.actor(), id, req.today()) {
        out.push_str(&format!("{},{},{},{}\r\n", csv_cell(r.tool), r.total, r.open, r.overdue));
    }
    Some(out)
}
