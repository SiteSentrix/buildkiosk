//! The schedule pages: the task table with a Gantt chart drawn in plain HTML and CSS, the task form
//! and CSV export.

use std::collections::HashMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::model::Choice;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::schedule::ScheduleTask;
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

pub fn page(req: &Req<'_>, id: Id<Project>, filter: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Schedule);
    if lvl < Level::Read {
        return not_found(req);
    }
    let today = req.today();
    let all = sched::rows(state, id, today);
    let scale = Scale::new(&all, today);
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
    let admin = lvl == Level::Admin;
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
            div class="actions" { a class="btn" href=(format!("{base}/lookahead")) { "Look ahead" } a class="btn" href=(format!("{base}/calendar")) { "Calendar" } a class="btn" href=(format!("{base}.pdf")) { "PDF" } a class="btn" href=(format!("{base}.csv")) { "Export CSV" } }
        }
        @if !all.is_empty() {
            div class="stat-row" {
                div class="stat" { div class="value" { (all.len()) } div class="label" { "Tasks" } }
                div class="stat" { div class="value" { (finish.map(short).unwrap_or_default()) } div class="label" { "Last finish" } }
                div class="stat" { div class="value" { (n_crit) } div class="label" { "On the critical path" } }
                div class="stat" { div class="value" { (n_late) } div class="label" { "Late" } }
                @if n_conflict > 0 { div class="stat" { div class="value" { (n_conflict) } div class="label" { "Start before a link allows" } } }
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
                table class="data gantt" {
                    thead {
                        tr {
                            th { "#" } th { "Task" } th { "Start" } th { "Finish" } th class="right" { "Days" } th class="right" { "Done" } th class="right" { "Float" }
                            th class="timeline" { svg class="months" viewBox=(format!("0 0 {W} 20")) preserveAspectRatio="none" aria-hidden="true" { @for (label, at) in scale.months() { line x1=(format!("{at:.1}")) x2=(format!("{at:.1}")) y1="0" y2="20" class="tick" {} text x=(format!("{:.1}", at + 3.0)) y="14" { (label) } } } }
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
                                }
                                td class="nowrap" { (short(r.start)) }
                                td class="nowrap" { (short(r.finish)) }
                                td class="right" { @if r.milestone { "◆" } @else { (r.duration) } }
                                td class="right" { (r.percent) "%" }
                                td class="right" { (r.float.map(|f| f.to_string()).unwrap_or_default()) }
                                td class="timeline" {
                                    svg class="track" viewBox=(format!("0 0 {W} 30")) preserveAspectRatio="none" role="img" aria-label=(format!("{}: {} to {}, {}% done", r.name, short(r.start), short(r.finish), r.percent)) {
                                        @if (0.0..=W).contains(&scale.at(today)) { line class="today" x1=(format!("{:.1}", scale.at(today))) x2=(format!("{:.1}", scale.at(today))) y1="0" y2="30" {} }
                                        @if r.milestone {
                                            @let x = scale.at(r.start);
                                            polygon class=(if r.critical { "diamond critical" } else { "diamond" }) points=(format!("{:.1},9 {:.1},15 {:.1},21 {:.1},15", x, x + 6.0, x, x - 6.0)) {}
                                        } @else {
                                            @let (x, w) = (scale.at(r.start), scale.width(r.start, r.finish));
                                            @let cls = format!("bar{}{}", if r.critical { " critical" } else { "" }, if r.summary { " summary" } else { "" });
                                            @let (y, h) = if r.summary { (11.0, 8.0) } else { (8.0, 14.0) };
                                            rect class=(cls) x=(format!("{x:.1}")) y=(format!("{y}")) width=(format!("{w:.1}")) height=(format!("{h}")) {}
                                            @if r.percent > 0 { rect class=(format!("done{}", if r.critical { " critical" } else { "" })) x=(format!("{x:.1}")) y=(format!("{y}")) width=(format!("{:.1}", w * f64::from(r.percent) / 100.0)) height=(format!("{h}")) {} }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            p class="muted small" { "The dashed line is today. Red bars are on the critical path. Float is the working days a task can slip without moving the last finish." @if has_baseline { " Baseline saved." } }
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
            div class="card" {
                h2 { "Add a task" }
                (command_form(req, "schedule.add", &base, &base, task_fields(id, &parents, &vendors, None, state)))
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a schedule" }
                    p class="muted small" { "One task per line: id, name, start, finish, percent, predecessors, parent. Dates are YYYY-MM-DD. Predecessors are other ids in the file separated by ;, each with an optional lag such as 4+2. Ids are renumbered when imported." }
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
    let days = current.map(|t| sched::days_inclusive(t.start, t.finish).max(1).to_string()).unwrap_or_default();
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
    let cal = sched::calendar(state, id);
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
                    div class="form-actions" { button type="submit" class="btn primary" { "Save calendar" } }
                }))
            } @else {
                p { strong { "Weekdays worked: " } (days_text(&cal.days)) }
                @if cal.holidays.is_empty() { p class="muted" { "No days off are listed." } }
                @for h in &cal.holidays { p { (h.date.pretty()) @if !h.name.is_empty() { " · " (h.name) } } }
                p class="muted" { "Only a Schedule admin can change the calendar." }
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
            div class="actions" { a class="btn" href=(base.clone()) { "Back to the schedule" } }
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
