//! The schedule pages: the task table with a Gantt chart drawn in plain HTML and CSS, the task form
//! and CSV export.

use std::collections::HashMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::model::Choice;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::schedule::{Constraint, ScheduleTask};
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::sched::{self, Row};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, empty, field};
use crate::pages::{not_found, project_for};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Chart width in SVG units; the SVG stretches to the column, so this only sets proportions.
const W: f64 = 600.0;

/// Where a date sits across the chart, in SVG units.
struct Scale {
    first: Date,
    days: f64,
}

impl Scale {
    fn new(rows: &[Row], today: Date) -> Option<Scale> {
        let first = rows.iter().map(|r| r.start).min()?.min(today).add_days(-2)?;
        let last = rows.iter().map(|r| r.finish).max()?.max(today).add_days(2)?;
        Some(Scale { first, days: (first.days_until(last) + 1) as f64 })
    }

    fn at(&self, d: Date) -> f64 {
        (self.first.days_until(d) as f64 / self.days * W).clamp(0.0, W)
    }

    fn width(&self, a: Date, b: Date) -> f64 {
        ((a.days_until(b) + 1) as f64 / self.days * W).max(3.0)
    }

    /// The days off in the span as (position, width) runs, from the project's calendar.
    fn days_off(&self, cal: &buildkiosk_engine::model::schedule::WorkCalendar) -> Vec<(f64, f64)> {
        let mut out = Vec::new();
        let mut run: Option<(Date, Date)> = None;
        for i in 0..(self.days as i64).min(4_000) {
            let Some(d) = self.first.add_days(i) else { break };
            if cal.is_working(d) {
                if let Some((a, b)) = run.take() {
                    out.push((self.at(a), self.width(a, b)));
                }
            } else {
                run = Some(match run {
                    Some((a, _)) => (a, d),
                    None => (d, d),
                });
            }
        }
        if let Some((a, b)) = run {
            out.push((self.at(a), self.width(a, b)));
        }
        out
    }

    /// Month labels with their positions.
    fn months(&self) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        let mut at = self.first.month_start();
        for _ in 0..60 {
            if self.first.days_until(at) as f64 >= self.days {
                break;
            }
            let (y, m, _) = at.ymd();
            out.push((format!("{} {y}", MONTHS.get(usize::try_from(m).unwrap_or(1).saturating_sub(1)).copied().unwrap_or("")), self.at(at)));
            let Some(next) = at.add_days(32) else { break };
            at = next.month_start();
        }
        out
    }
}

fn short(d: Date) -> String {
    let (_, m, day) = d.ymd();
    format!("{} {day}", MONTHS.get(usize::try_from(m).unwrap_or(1).saturating_sub(1)).copied().unwrap_or(""))
}

/// The schedule with its chart. With `drag` (the browser module is built in) a person who can edit may drag bars.
pub fn page(req: &Req<'_>, id: Id<Project>, filter: &str, drag: bool) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Schedule);
    if lvl < Level::Read {
        return not_found(req);
    }
    let today = req.today();
    let all = sched::rows(state, id, today);
    let scale = Scale::new(&all, today);
    let off = scale.as_ref().map(|s| s.days_off(&sched::calendar(state, id))).unwrap_or_default();
    let rows: Vec<&Row> = all
        .iter()
        .filter(|r| match filter {
            "critical" => r.critical,
            "late" => r.late,
            "open" => r.percent < 100,
            _ => true,
        })
        .collect();
    let base = format!("/projects/{id}/schedule");
    let can_edit = lvl >= Level::Standard;
    let drag = drag && can_edit;
    let admin = lvl == Level::Admin;
    let saved = buildkiosk_engine::cmd::schedule::saved_baselines(state, id);
    let has_baseline = state.schedule_tasks.iter().any(|t| t.project == id && t.deleted.is_none() && t.baseline_finish.is_some());
    let (n_crit, n_late, n_conflict) =
        (all.iter().filter(|r| r.critical).count(), all.iter().filter(|r| r.late).count(), all.iter().filter(|r| r.conflict).count());
    let finish = all.iter().map(|r| r.finish).max();
    let parents: Vec<&ScheduleTask> = {
        let mut v: Vec<_> = state.schedule_tasks.iter().filter(|t| t.project == id && t.deleted.is_none()).collect();
        v.sort_by_key(|t| t.number);
        v
    };
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let body = html! {
        div class="page-head" {
            div { h1 { "Schedule" } div class="sub" { "Tasks in working days (" (days_text(&sched::calendar(state, id).days)) "), linked finish to start (or start to start, finish to finish, start to finish); the critical path is the chain with no float" } }
            div class="actions" { a class="btn" href=(format!("{base}/lookahead")) { "Look ahead" } a class="btn" href=(format!("{base}/resources")) { "Resources" } a class="btn" href=(format!("{base}/calendar")) { "Calendar" } a class="btn" href=(format!("{base}.pdf")) { "PDF" } a class="btn" href=(format!("{base}.csv")) { "Export CSV" } a class="btn" href=(format!("{base}.xml")) { "Export project file (XML)" } }
        }
        @if !all.is_empty() {
            div class="stat-row" {
                div class="stat" { div class="value" { (all.len()) } div class="label" { "Tasks" } }
                div class="stat" { div class="value" { (finish.map(short).unwrap_or_default()) } div class="label" { "Last finish" } }
                div class="stat" { div class="value" { (n_crit) } div class="label" { "On the critical path" } }
                div class="stat" { div class="value" { (n_late) } div class="label" { "Late" } }
                @if n_conflict > 0 { div class="stat" { div class="value" { (n_conflict) } div class="label" { "Start before a link allows" } } }
                @let n_broken = all.iter().filter(|r| r.broken).count();
                @if n_broken > 0 { div class="stat" { div class="value" { (n_broken) } div class="label" { "Break a constraint" } } }
            }
        }
        form method="get" action=(base) class="filters" {
            select name="show" aria-label="Show" {
                @for (k, l) in [("", "All tasks"), ("open", "Not complete"), ("critical", "Critical path"), ("late", "Late")] { option value=(k) selected[k == filter] { (l) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if rows.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Schedule), if all.is_empty() { "No tasks yet" } else { "No tasks match" }, if all.is_empty() && can_edit { "Add a task below or import a schedule." } else { "Change the filter to see more." })) }
        } @else if let Some(scale) = &scale {
            div class="table-wrap" {
                table class="data gantt" id=[drag.then_some("wasm-gantt")] data-days=[drag.then(|| scale.days.to_string())] {
                    thead {
                        tr {
                            th { "#" } th { "Task" } th { "Start" } th { "Finish" } th class="right" { "Days" } th class="right" { "Done" } th class="right" { "Float" }
                            th class="timeline" { svg class="months" viewBox=(format!("0 0 {W} 20")) preserveAspectRatio="none" aria-hidden="true" { defs { g id="daysoff" { @for (x, w) in &off { rect class="dayoff" x=(format!("{x:.1}")) y="0" width=(format!("{w:.1}")) height="30" {} } } } @for (label, at) in scale.months() { line x1=(format!("{at:.1}")) x2=(format!("{at:.1}")) y1="0" y2="20" class="tick" {} text x=(format!("{:.1}", at + 3.0)) y="14" { (label) } } } }
                        }
                    }
                    tbody {
                        @for r in &rows {
                            tr class=(if r.summary { "summary" } else { "" }) {
                                td class="muted" { (r.number) }
                                td class=(format!("d{}", r.depth.min(8))) {
                                    @if can_edit { a href=(format!("{base}/{}", r.id)) { (r.name) } } @else { (r.name) }
                                    @if r.critical { " " span class="pill red" { "Critical" } }
                                    @if r.late { " " span class="pill amber" { "Late" } }
                                    @if r.conflict { " " span class="pill amber" { "Check links" } }
                                    @if r.broken { " " span class="pill amber" { "Constraint" } }
                                }
                                td class="nowrap" { (short(r.start)) }
                                td class="nowrap" { (short(r.finish)) }
                                td class="right" { @if r.milestone { "◆" } @else { (r.duration) } }
                                td class="right" { (r.percent) "%" }
                                td class="right" { (r.float.map(|f| f.to_string()).unwrap_or_default()) }
                                td class="timeline" {
                                    svg class="track" viewBox=(format!("0 0 {W} 30")) preserveAspectRatio="none" role="img" aria-label=(format!("{}: {} to {}, {}% done", r.name, short(r.start), short(r.finish), r.percent)) {
                                        @if !off.is_empty() { use href="#daysoff" {} }
                                        @if let Some((bs, bf)) = state.schedule_tasks.get(r.id).and_then(|t| t.baseline_start.zip(t.baseline_finish)) {
                                            rect class="baseline" x=(format!("{:.1}", scale.at(bs))) y="24" width=(format!("{:.1}", scale.width(bs, bf))) height="4" {}
                                        }
                                        @if (0.0..=W).contains(&scale.at(today)) { line class="today" x1=(format!("{:.1}", scale.at(today))) x2=(format!("{:.1}", scale.at(today))) y1="0" y2="30" {} }
                                        @if r.milestone {
                                            @let x = scale.at(r.start);
                                            polygon class=(if r.critical { "diamond critical" } else { "diamond" }) data-task=[(drag && !r.summary).then(|| r.id.to_string())] points=(format!("{:.1},9 {:.1},15 {:.1},21 {:.1},15", x, x + 6.0, x, x - 6.0)) {}
                                        } @else {
                                            @let (x, w) = (scale.at(r.start), scale.width(r.start, r.finish));
                                            @let cls = format!("bar{}{}", if r.critical { " critical" } else { "" }, if r.summary { " summary" } else { "" });
                                            @let (y, h) = if r.summary { (11.0, 8.0) } else { (8.0, 14.0) };
                                            @let task = (drag && !r.summary).then(|| r.id.to_string());
                                            rect class=(cls) data-task=[task.clone()] x=(format!("{x:.1}")) y=(format!("{y}")) width=(format!("{w:.1}")) height=(format!("{h}")) {}
                                            @if r.percent > 0 { rect class=(format!("done{}", if r.critical { " critical" } else { "" })) data-task=[task.clone()] x=(format!("{x:.1}")) y=(format!("{y}")) width=(format!("{:.1}", w * f64::from(r.percent) / 100.0)) height=(format!("{h}")) {} }
                                            @if let Some(t) = &task { rect class="handle" data-task=(t) data-edge="finish" x=(format!("{:.1}", x + w - 5.0)) y=(format!("{y}")) width="5" height=(format!("{h}")) {} }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            p class="muted small" { "The dashed line is today. Red bars are on the critical path. Grey columns are days off. Float is the working days a task can slip without moving the last finish." @if has_baseline { " The thin bar under a task is its baseline." } }
            @if drag {
                p id="gantt-hint" class="muted small" {}
                script type="module" src="/static/viewer/start.js" {}
                div id="gantt-move" hidden {
                    (command_form(req, "schedule.move", &base, &base, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" id="gm-id" name="id" value="";
                        input type="hidden" id="gm-shift" name="shift" value="0";
                        input type="hidden" id="gm-fshift" name="finish_shift" value="0" disabled;
                    }))
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Schedule actions" }
                div class="actions" {
                    (button_form(req, "schedule.recalculate", &base, &[("project", id.to_string())], "Move tasks that start too early", "small"))
                    @if admin {
                        (button_form(req, "schedule.baseline", &base, &[("project", id.to_string())], if has_baseline { "Replace baseline with current dates" } else { "Save baseline" }, "small"))
                        @if has_baseline { (button_form(req, "schedule.baseline", &base, &[("project", id.to_string()), ("clear", "true".into())], "Clear baseline", "small danger")) }
                    }
                }
            }
            @if admin || !saved.is_empty() {
                div class="card" {
                    h2 { "Baselines" }
                    @if saved.is_empty() { p class="muted" { "A baseline is the schedule's dates kept under a name, so progress can be measured against them. Save one when the contract is signed and another after a re-plan." } } @else {
                        table class="data" {
                            thead { tr { th { "Name" } th { "Saved" } th class="num" { "Tasks" } th { "Finish it promised" } th {} } }
                            tbody { @for b in &saved { tr {
                                td { (b.name) @if b.in_use { " " span class="pill blue" { "In use" } } }
                                td class="nowrap" { (req.when(b.saved.at)) } td class="num" { (b.tasks.len()) }
                                td class="nowrap" { @if let Some(f) = b.tasks.iter().map(|t| t.finish).max() { (f.pretty()) } }
                                td class="right" { @if admin {
                                    @if !b.in_use { (button_form(req, "schedule.baseline_use", &base, &[("project", id.to_string()), ("id", b.id.to_string())], "Use", "small")) " " }
                                    (button_form(req, "schedule.baseline_delete", &base, &[("project", id.to_string()), ("id", b.id.to_string())], "Delete", "small danger"))
                                } }
                            } } }
                        }
                    }
                    @if admin {
                        (command_form(req, "schedule.baseline_save", &base, &base, html! {
                            input type="hidden" name="project" value=(id);
                            div class="form-grid" { (field("Save the current dates as", "bl_name", true, false, html! { input type="text" id="bl_name" name="name" required placeholder="Contract" list="bl_names"; datalist id="bl_names" { @for b in &saved { option value=(b.name) {} } } })) }
                            p class="muted small" { "A name already used is replaced. The new baseline becomes the one the chart and each task's slip compare against." }
                            div class="form-actions" { button type="submit" class="btn small primary" { "Save baseline" } }
                        }))
                    }
                }
            }
            div class="card" {
                h2 { "Add a task" }
                (command_form(req, "schedule.add", &base, &base, task_fields(id, &parents, &vendors, None, state)))
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a project file (Microsoft Project XML)" }
                    p class="muted small" { "Save the schedule from your planning software as XML. Tasks, dates, percent complete, milestones, outline levels, links with their kind and lag, and start and finish constraints come across. Tick the boxes below to take more from the file. Costs are left behind. Up to 8 MB." }
                    form method="post" action=(format!("{base}/import-xml")) enctype="multipart/form-data" {
                        input type="hidden" name="_csrf" value=(req.csrf);
                        (field("Project file", "sched_xml", true, true, html! { input type="file" id="sched_xml" name="file" accept=".xml,text/xml,application/xml" required; }))
                        label class="check" { input type="checkbox" name="resources" value="true"; " Take the resources assigned to each task (100% is one a day, rounded to whole numbers)" }
                        @if lvl == Level::Admin {
                            label class="check" { input type="checkbox" name="limits" value="true"; " Set the resource limits from the file's maximum of each resource" }
                            label class="check" { input type="checkbox" name="calendar" value="true"; " Replace this project's working days and days off with the file's calendar (then recalculate)" }
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Import file" } }
                    }
                }
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import an exchange file (.xer)" }
                    p class="muted small" { "Export the schedule from your planning software as an exchange (.xer) file. The work breakdown becomes summary tasks, and tasks, dates (actual, else scheduled, else planned), percent complete, milestones, links with their kind and lag, and date constraints come across; costs are left behind. Tick the boxes below to take the resources and the calendar too. If the file holds more than one project, give the id of the one you want (the first is used otherwise). Up to 8 MB." }
                    form method="post" action=(format!("{base}/import-xer")) enctype="multipart/form-data" {
                        input type="hidden" name="_csrf" value=(req.csrf);
                        (field("Exchange file", "sched_xer", true, true, html! { input type="file" id="sched_xer" name="file" accept=".xer,text/plain" required; }))
                        (field("Project id in the file (optional)", "sched_xer_id", false, true, html! { input type="text" id="sched_xer_id" name="proj_id" maxlength="40"; }))
                        label class="check" { input type="checkbox" name="resources" value="true"; " Take the resources assigned to each task (100% is one a day, rounded to whole numbers)" }
                        @if lvl == Level::Admin {
                            label class="check" { input type="checkbox" name="limits" value="true"; " Set the resource limits from the file's maximum of each resource" }
                            label class="check" { input type="checkbox" name="calendar" value="true"; " Replace this project's working days and days off with the file's calendar (then recalculate)" }
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Import file" } }
                    }
                }
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a schedule" }
                    p class="muted small" { "One task per line: id, name, start, finish, percent, predecessors, parent. Dates are YYYY-MM-DD. Predecessors are other ids in the file separated by ;, each with an optional lag such as 4+2. Ids are renumbered when imported." }
                    p class="muted small" { "A seventh to eleventh column is optional: milestone (yes or no), then a constraint (snet, mso or fnlt) and its date, then the resources the task needs each working day, such as Carpenter 4, Laborer 2." }
                    (command_form(req, "schedule.import", &base, &base, html! {
                        input type="hidden" name="project" value=(id);
                        (field("Tasks", "sched_text", true, true, html! { textarea id="sched_text" name="text" required {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
    };
    layout::app(
        req,
        "Schedule",
        Nav::Schedule(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Schedule", None)],
        body,
    )
}

fn links_text(t: &ScheduleTask, state: &buildkiosk_engine::model::CompanyState) -> String {
    t.predecessors
        .iter()
        .filter_map(|l| state.schedule_tasks.get(l.task).filter(|x| x.deleted.is_none()).map(|x| sched::format_link(x.number, l.kind, l.lag)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn task_fields(
    project: Id<Project>,
    parents: &[&ScheduleTask],
    vendors: &[&buildkiosk_engine::model::directory::Vendor],
    current: Option<&ScheduleTask>,
    state: &buildkiosk_engine::model::CompanyState,
) -> Markup {
    let cal = sched::calendar(state, project);
    let days = current.map(|t| sched::days_inclusive_in(&cal, t.start, t.finish).max(1).to_string()).unwrap_or_default();
    let resources = current.map(|t| t.resources.iter().map(|a| format!("{} {}", a.name, a.units)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
    html! {
        input type="hidden" name="project" value=(project);
        @if let Some(t) = current { input type="hidden" name="id" value=(t.id); }
        div class="form-grid" {
            (field("Name", "t_name", true, true, html! { input type="text" id="t_name" name="name" value=(current.map(|t| t.name.as_str()).unwrap_or_default()) required; }))
            (field("Start", "t_start", true, false, html! { input type="date" id="t_start" name="start" value=(current.map(|t| t.start.to_string()).unwrap_or_default()) required; }))
            (field("Working days", "t_days", false, false, html! { input type="number" id="t_days" name="days" min="1" max="5000" value=(days); }))
            (field("Predecessors (task numbers with an optional kind and lag, e.g. 3, 5+2, 7SS, 4FF-1)", "t_preds", false, false, html! { input type="text" id="t_preds" name="predecessors" value=(current.map(|t| links_text(t, state)).unwrap_or_default()); }))
            (field("Part of (summary task)", "t_parent", false, false, html! { select id="t_parent" name="parent" {
                option value="" { "Top level" }
                @for x in parents.iter().filter(|x| Some(x.id) != current.map(|c| c.id)) { option value=(x.id) selected[current.is_some_and(|c| c.parent == Some(x.id))] { (x.number) " " (x.name) } }
            } }))
            (field("Company", "t_company", false, false, html! { select id="t_company" name="company" {
                option value="" { "—" }
                @for v in vendors { option value=(v.id) selected[current.is_some_and(|c| c.company == Some(v.id))] { (v.name) } }
            } }))
            (field("Percent complete", "t_pct", false, false, html! { input type="number" id="t_pct" name="percent" min="0" max="100" value=(current.map(|t| t.percent.to_string()).unwrap_or_default()); }))
            (field("Milestone", "t_milestone", false, false, html! { select id="t_milestone" name="milestone" {
                option value="false" selected[current.is_none_or(|t| !t.milestone)] { "No, a task with a duration" }
                option value="true" selected[current.is_some_and(|t| t.milestone)] { "Yes, a single day" }
            } }))
            (field("Constraint", "t_constraint", false, false, html! { select id="t_constraint" name="constraint" {
                @for c in Constraint::ALL { option value=(c.key()) selected[current.map(|t| t.constraint).unwrap_or_default() == *c] { (c.label()) } }
            } }))
            (field("Constraint date", "t_cdate", false, false, html! { input type="date" id="t_cdate" name="constraint_date" value=(current.and_then(|t| t.constraint_date).map(|d| d.to_string()).unwrap_or_default()); }))
            (field("Resources needed each working day (e.g. Carpenter 4, Laborer 2)", "t_resources", false, true, html! { input type="text" id="t_resources" name="resources" value=(resources); }))
            (field("Notes", "t_notes", false, true, html! { textarea id="t_notes" name="notes" { (current.map(|t| t.notes.as_str()).unwrap_or_default()) } }))
        }
        div class="form-actions" { button type="submit" class="btn primary" { (if current.is_some() { "Save" } else { "Add task" }) } }
    }
}

pub fn detail(req: &Req<'_>, id: Id<Project>, tid: Id<ScheduleTask>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Schedule);
    let Some(t) = state.schedule_tasks.get(tid).filter(|t| t.project == id && t.deleted.is_none()).filter(|_| lvl >= Level::Standard) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/schedule");
    let here = format!("{base}/{tid}");
    let mut parents: Vec<&ScheduleTask> = state.schedule_tasks.iter().filter(|x| x.project == id && x.deleted.is_none()).collect();
    parents.sort_by_key(|x| x.number);
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let successors: Vec<&ScheduleTask> = parents.iter().copied().filter(|x| x.predecessors.iter().any(|l| l.task == tid)).collect();
    let by_id: HashMap<_, _> = parents.iter().map(|x| (x.id, *x)).collect();
    let body = html! {
        div class="page-head" { div { h1 { "Task " (t.number) ": " (t.name) } div class="sub" { (short(t.start)) " to " (short(t.finish)) @if let (Some(a), Some(b)) = (t.baseline_start, t.baseline_finish) { " · baseline " (short(a)) " to " (short(b)) } } } }
        div class="card" {
            h2 { "Edit task" }
            (command_form(req, "schedule.update", &here, &here, task_fields(id, &parents, &vendors, Some(t), state)))
        }
        @if t.constraint != Constraint::None || !t.resources.is_empty() {
            div class="card" {
                h2 { "Constraint and resources" }
                @if let (true, Some(d)) = (t.constraint != Constraint::None, t.constraint_date) { p { strong { (t.constraint.label()) ": " } (d.pretty()) } }
                @if !t.resources.is_empty() { p { strong { "Needs each working day: " } (t.resources.iter().map(|a| format!("{} {}", a.name, a.units)).collect::<Vec<_>>().join(", ")) } }
            }
        }
        div class="card" {
            h2 { "Links" }
            p { "Follows: " @if t.predecessors.is_empty() { span class="muted" { "nothing" } } @for l in &t.predecessors { @if let Some(x) = by_id.get(&l.task) { a href=(format!("{base}/{}", x.id)) { (x.number) " " (x.name) } (match (l.kind, l.lag) { (buildkiosk_engine::model::schedule::LinkKind::FinishToStart, 0) => String::new(), (k, lag) => format!(" ({}{})", k.label().to_lowercase(), if lag != 0 { format!(", {lag:+} days") } else { String::new() }) }) " " } } }
            p { "Followed by: " @if successors.is_empty() { span class="muted" { "nothing" } } @for x in &successors { a href=(format!("{base}/{}", x.id)) { (x.number) " " (x.name) } " " } }
        }
        @if req.allowed("schedule.delete", Some(id)) {
            div class="card" { (button_form(req, "schedule.delete", &base, &[("project", id.to_string()), ("id", tid.to_string())], "Delete task", "small danger")) }
        }
    };
    layout::app(
        req,
        &t.name,
        Nav::Schedule(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Schedule", Some(base)), (&t.name, None)],
        body,
    )
}

pub fn csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Schedule) < Level::Read {
        return None;
    }
    let mut out =
        String::from("Number,Task,Start,Finish,Working days,Percent complete,Float,Critical,Predecessors,Baseline start,Baseline finish\r\n");
    for r in sched::rows(state, id, req.today()) {
        let t = state.schedule_tasks.get(r.id);
        let row = [
            r.number.to_string(),
            r.name.clone(),
            r.start.to_string(),
            r.finish.to_string(),
            r.duration.to_string(),
            r.percent.to_string(),
            r.float.map(|f| f.to_string()).unwrap_or_default(),
            if r.critical { "yes".into() } else { String::new() },
            t.map(|t| links_text(t, state)).unwrap_or_default(),
            t.and_then(|t| t.baseline_start).map(|d| d.to_string()).unwrap_or_default(),
            t.and_then(|t| t.baseline_finish).map(|d| d.to_string()).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

/// `mon-fri`, `mon-sat`, `mon, wed, fri`: the weekdays worked.
pub fn days_text(days: &[bool; 7]) -> String {
    const NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
    let on: Vec<usize> = (0..7).filter(|i| days.get(*i).copied().unwrap_or(false)).collect();
    match (on.first(), on.last()) {
        (Some(a), Some(b)) if on.len() >= 3 && b - a + 1 == on.len() => {
            format!("{}-{}", NAMES.get(*a).copied().unwrap_or(""), NAMES.get(*b).copied().unwrap_or(""))
        }
        _ => on.iter().filter_map(|i| NAMES.get(*i).copied()).collect::<Vec<_>>().join(", "),
    }
}

/// The days work is done on: the weekdays and the holidays.
pub fn calendar(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Schedule);
    if lvl < Level::Read {
        return not_found(req);
    }
    let cal = sched::own_calendar(state, id);
    let lists = buildkiosk_engine::cmd::schedule::holiday_lists(state);
    let company_admin = match req.actor() {
        buildkiosk_engine::Actor::System => true,
        buildkiosk_engine::Actor::Person(me) => buildkiosk_engine::model::perm::company_level(state, me, Tool::CompanyAdmin) == Level::Admin,
    };
    let base = format!("/projects/{id}/schedule");
    let here = format!("{base}/calendar");
    let holidays: String =
        cal.holidays.iter().map(|h| if h.name.is_empty() { format!("{}\n", h.date) } else { format!("{} {}\n", h.date, h.name) }).collect();
    let body = html! {
        div class="page-head" {
            div { h1 { "Working calendar" } div class="sub" { "The days work is done on. Task dates and durations are counted in these days." } }
            div class="actions" { a class="btn" href=(base.clone()) { "Back to the schedule" } }
        }
        div class="card" {
            @if lvl == Level::Admin {
                (command_form(req, "schedule.calendar_set", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    (field("Weekdays worked", "cal_days", true, false, html! { input type="text" id="cal_days" name="days" value=(days_text(&cal.days)) required; }))
                    p class="muted small" { "Such as mon-fri, mon-sat, or mon, tue, thu." }
                    (field("Days off", "cal_holidays", false, true, html! { textarea id="cal_holidays" name="holidays" rows="10" { (holidays) } }))
                    p class="muted small" { "One per line: a date as YYYY-MM-DD, then what it is (optional). Task dates don't move when this changes; use Recalculate on the schedule to push tasks that now start too early." }
                    @if !lists.is_empty() {
                        div class="field full" {
                            div class="label" { "Company holiday lists this project takes" }
                            input type="hidden" name="lists[]" value="";
                            div class="options" { @for l in &lists { label { input type="checkbox" name="lists[]" value=(l.id) checked[p.holiday_lists.contains(&l.id)]; (l.name) " (" (l.holidays.len()) " days)" } } }
                        }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save calendar" } }
                }))
            } @else {
                p { strong { "Weekdays worked: " } (days_text(&sched::calendar(state, id).days)) }
                @if cal.holidays.is_empty() { p class="muted" { "No days off are listed." } }
                @for h in &cal.holidays { p { (h.date.pretty()) @if !h.name.is_empty() { " · " (h.name) } } }
                p class="muted" { "Only a Schedule admin can change the calendar." }
            }
        }
        @if !p.holiday_lists.is_empty() {
            div class="card" {
                h2 { "Days off from company lists" }
                @for l in p.holiday_lists.iter().filter_map(|l| state.holiday_lists.get(*l)).filter(|l| l.deleted.is_none()) {
                    p { strong { (l.name) } ": " (l.holidays.iter().map(|h| format!("{} {}", h.date, h.name).trim().to_string()).collect::<Vec<_>>().join(" · ")) }
                }
            }
        }
        @if company_admin {
            div class="card" {
                h2 { "Company holiday lists" }
                p class="muted small" { "Shared by every project that takes them. Changing a list changes those projects' calendars; their task dates stay until recalculated." }
                @for l in &lists {
                    details class="disclosure" {
                        summary { (l.name) " · " (l.holidays.len()) " days" }
                        (command_form(req, "holiday_list.save", &here, &here, html! {
                            input type="hidden" name="id" value=(l.id);
                            div class="form-grid" {
                                (field("Name", &format!("hl_n{}", l.id), true, false, html! { input type="text" id=(format!("hl_n{}", l.id)) name="name" required value=(l.name); }))
                                (field("Days off", &format!("hl_d{}", l.id), false, true, html! { textarea id=(format!("hl_d{}", l.id)) name="holidays" rows="8" { (l.holidays.iter().map(|h| format!("{} {}\n", h.date, h.name)).collect::<String>()) } }))
                            }
                            div class="form-actions" { button type="submit" class="btn primary small" { "Save list" } }
                        }))
                        (button_form(req, "holiday_list.delete", &here, &[("id", l.id.to_string())], "Delete list", "small danger"))
                    }
                }
                (command_form(req, "holiday_list.save", &here, &here, html! {
                    div class="form-grid" {
                        (field("New list", "hl_new", true, false, html! { input type="text" id="hl_new" name="name" required placeholder="Federal holidays 2027"; }))
                        (field("Days off", "hl_days", false, true, html! { textarea id="hl_days" name="holidays" rows="6" placeholder="2027-01-01 New Year's Day" {} }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary small" { "Add list" } }
                }))
            }
        }
    };
    layout::app(
        req,
        "Working calendar",
        Nav::Schedule(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Schedule", Some(base)), ("Calendar", None)],
        body,
    )
}

/// The work under way or starting in each of the next weeks.
pub fn lookahead(req: &Req<'_>, id: Id<Project>, weeks: i64, from: Option<Date>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Schedule) < Level::Read {
        return not_found(req);
    }
    let today = req.today();
    let from = from.unwrap_or(today);
    let weeks = weeks.clamp(1, 12);
    let rows = sched::rows(state, id, today);
    let base = format!("/projects/{id}/schedule");
    let body = html! {
        div class="page-head" {
            div { h1 { "Look ahead" } div class="sub" { "Work that is under way or starts in each week, and isn't finished" } }
            div class="actions" { a class="btn" href=(format!("{base}/lookahead.pdf?weeks={weeks}&from={from}")) { "PDF" } a class="btn" href=(base.clone()) { "Back to the schedule" } }
        }
        div class="viewer-bar" {
            span class="muted" { "Weeks" }
            @for n in [2i64, 3, 4, 6, 12] {
                a class=(if weeks == n { "chip active" } else { "chip" }) href=(format!("{base}/lookahead?weeks={n}&from={from}")) { (n) }
            }
            span class="muted" { "from " (from.pretty()) }
        }
        @for i in 0..weeks {
            @if let (Some(start), Some(end)) = (from.add_days(i * 7), from.add_days(i * 7 + 6)) {
                @let tasks: Vec<&Row> = rows.iter().filter(|r| !r.summary && r.start <= end && r.finish >= start && (r.percent < 100 || r.milestone)).collect();
                div class="card" {
                    h2 { "Week " (i + 1) " · " (short(start)) " to " (short(end)) }
                    @if tasks.is_empty() { p class="muted" { "Nothing scheduled." } } @else {
                        div class="table-wrap" { table class="data" {
                            thead { tr { th { "Task" } th { "Dates" } th { "Done" } th { "Company" } th { "" } } }
                            tbody {
                                @for r in tasks {
                                    @let company = state.schedule_tasks.get(r.id).and_then(|t| t.company).and_then(|c| state.vendors.get(c)).map(|c| c.name.clone()).unwrap_or_default();
                                    tr {
                                        td { a href=(format!("{base}/{}", r.id)) { (r.number) " " (r.name) } }
                                        td { (short(r.start)) @if r.finish != r.start { " to " (short(r.finish)) } }
                                        td { (r.percent) "%" }
                                        td { (company) }
                                        td { @if r.critical { span class="pill red" { "Critical" } } " " @if r.late { span class="pill amber" { "Late" } } }
                                    }
                                }
                            }
                        } }
                    }
                }
            }
        }
    };
    layout::app(
        req,
        "Look ahead",
        Nav::Schedule(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Schedule", Some(base)), ("Look ahead", None)],
        body,
    )
}

/// How many of each resource are needed each week, against the project's limits.
pub fn resources(req: &Req<'_>, id: Id<Project>, weeks: i64, from: Option<Date>, level: Option<bool>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Schedule);
    if lvl < Level::Read {
        return not_found(req);
    }
    let from = from.unwrap_or_else(|| req.today());
    let weeks = weeks.clamp(1, 26);
    let loads = sched::loading(state, id, from, weeks);
    let base = format!("/projects/{id}/schedule");
    let here = format!("{base}/resources");
    let limits: String =
        state.projects.get(id).map(|p| p.resource_limits.iter().map(|a| format!("{} {}\n", a.name, a.units)).collect()).unwrap_or_default();
    let body = html! {
        div class="page-head" {
            div { h1 { "Resource loading" } div class="sub" { "The most of each resource needed on any one working day in each week, from tasks that aren't finished. Red is over the limit you set." } }
            div class="actions" { a class="btn" href=(base.clone()) { "Back to the schedule" } }
        }
        div class="viewer-bar" {
            span class="muted" { "Weeks" }
            @for n in [4i64, 8, 12, 26] {
                a class=(if weeks == n { "chip active" } else { "chip" }) href=(format!("{here}?weeks={n}&from={from}")) { (n) }
            }
            span class="muted" { "from " (from.pretty()) }
        }
        @if loads.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Schedule), "No resources yet", "Name what a task needs each working day (such as Carpenter 4) when you add or edit it.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Resource" } th class="right" { "Limit" } @for w in 0..weeks { th class="right" { (from.add_days(w * 7).map(short).unwrap_or_default()) } } } }
                tbody {
                    @for l in &loads {
                        tr {
                            td { (l.name) }
                            td class="right" { (l.limit.map(|n| n.to_string()).unwrap_or_else(|| "—".into())) }
                            @for (i, peak) in l.peaks.iter().enumerate() {
                                td class=(if l.over.get(i).copied().unwrap_or(false) { "right over" } else { "right" }) { @if *peak > 0 { (peak) } @else { span class="muted" { "·" } } }
                            }
                        }
                    }
                }
            } }
        }
        @if lvl >= Level::Standard && !limits.trim().is_empty() {
            (levelling(req, id, &base, &here, weeks, from, level))
        }
        @if lvl == Level::Admin {
            div class="card" {
                h2 { "Limits" }
                p class="muted small" { "How many of each resource you can have working on one day, one per line, such as Carpenter 6." }
                (command_form(req, "schedule.resource_limits", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    (field("Limits", "res_limits", false, true, html! { textarea id="res_limits" name="text" rows="6" { (limits) } }))
                    div class="form-actions" { button type="submit" class="btn primary" { "Save limits" } }
                }))
            }
        }
    };
    layout::app(
        req,
        "Resource loading",
        Nav::Schedule(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Schedule", Some(base)), ("Resources", None)],
        body,
    )
}

/// The "Level resources" card: a preview of what levelling would do and the button that does it.
fn levelling(req: &Req<'_>, id: Id<Project>, base: &str, here: &str, weeks: i64, from: Date, level: Option<bool>) -> Markup {
    let state = req.engine.state();
    let plan = level.map(|within| (within, buildkiosk_engine::level::plan(state, id, req.today(), within)));
    let link = |within: &str| format!("{here}?weeks={weeks}&from={from}&level={within}");
    html! {
        div class="card" {
            h2 { "Level resources" }
            p class="muted small" { "Delays tasks that need more of a resource than the limit allows on a working day, and what follows them through their links. Finished work, started work and work pinned to a date stay where they are. Tasks only move later. Look at the plan first: nothing changes until you apply it." }
            div class="actions" {
                a class="btn" href=(link("float")) { "Preview, keeping the finish date" }
                a class="btn" href=(link("all")) { "Preview, letting the finish move" }
            }
            @match &plan {
                None => {}
                Some((_, Err(msg))) => { p class="error" { (msg) } }
                Some((within, Ok(plan))) => {
                    @if plan.moves.is_empty() && plan.unresolved.is_empty() {
                        p { "No resource is over its limit, so nothing needs to move." }
                    }
                    @if !plan.moves.is_empty() {
                        div class="table-wrap" { table class="data" {
                            thead { tr { th class="right" { "#" } th { "Task" } th { "Now" } th { "Would be" } th class="right" { "Working days later" } } }
                            tbody {
                                @for m in &plan.moves {
                                    tr {
                                        td class="right" { (m.number) }
                                        td { (m.name) }
                                        td { (m.from_start.pretty()) " – " (m.from_finish.pretty()) }
                                        td { (m.start.pretty()) " – " (m.finish.pretty()) }
                                        td class="right" { (m.days) }
                                    }
                                }
                            }
                        } }
                        @if let (Some(a), Some(b)) = (plan.finish_before, plan.finish_after) {
                            p { "Project finish: " (a.pretty()) @if plan.finish_slip > 0 { " becomes " strong { (b.pretty()) } " (" (plan.finish_slip) " working days later)" } @else { ", unchanged" } "." }
                        }
                    }
                    @if !plan.unresolved.is_empty() {
                        h3 { "Still over a limit" }
                        ul {
                            @for u in &plan.unresolved { li { "#" (u.number) " " (u.name) ": " (u.resource) " " (u.reason) } }
                        }
                    }
                    @if !plan.moves.is_empty() {
                        (command_form(req, "schedule.level", base, base, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="within_float" value=(if *within { "true" } else { "false" });
                            div class="form-actions" { button type="submit" class="btn primary" { "Apply this plan" } }
                        }))
                    }
                }
            }
        }
    }
}
