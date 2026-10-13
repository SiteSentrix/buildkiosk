//! Daily log pages: list, detail and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::daily_log::can_edit;
use buildkiosk_engine::model::daily_log::{DailyLog, LogStatus, Sky, hours_text};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, empty, field};
use crate::pages::{dash, not_found, project_for};

fn status_pill(s: LogStatus) -> Markup {
    html! { span class=(if s == LogStatus::Submitted { "pill green" } else { "pill violet" }) { (s.label()) } }
}

fn weather_text(l: &DailyLog) -> String {
    let w = &l.weather;
    match (w.high, w.low) {
        (Some(h), Some(lo)) => format!("{}, {h}/{lo}", w.sky.label()),
        (Some(h), None) => format!("{}, high {h}", w.sky.label()),
        (None, Some(lo)) => format!("{}, low {lo}", w.sky.label()),
        (None, None) => w.sky.label().to_string(),
    }
}

fn crumbs<'a>(p: &'a Project, last: Option<&'a str>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some(l) => {
            c.push(("Daily log", Some(format!("/projects/{}/daily-logs", p.id))));
            c.push((l, None));
        }
        None => c.push(("Daily log", None)),
    }
    c
}

/// `month` is "YYYY-MM" (or empty for everything).
pub fn list(req: &Req<'_>, id: Id<Project>, month: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::DailyLog) < Level::Read {
        return not_found(req);
    }
    let all = view::daily_logs_for(state, req.actor(), id);
    let shown: Vec<&DailyLog> = all.iter().copied().filter(|l| month.is_empty() || l.date.to_string().starts_with(month)).take(200).collect();
    let base = format!("/projects/{id}/daily-logs");
    let today = req.today();
    let todays = all.iter().find(|l| l.date == today);
    let body = html! {
        div class="page-head" {
            div { h1 { "Daily log" } div class="sub" { "What happened on site, day by day" } }
            div class="actions" {
                a class="btn" href=(format!("{base}/week")) { "Week" }
                @if let Some(l) = todays { a class="btn primary" href=(format!("{base}/{}", l.id)) { "Today's log" } }
                @else if req.allowed("daily_log.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "Start today's log" } }
                @if todays.is_some() && req.allowed("daily_log.create", Some(id)) { a class="btn" href=(format!("{base}/new")) { (Icon::Plus.svg()) "Another day" } }
            }
        }
        form method="get" action=(base) class="filters" {
            input type="month" name="month" value=(month) aria-label="Month";
            button type="submit" class="btn small" { (Icon::Search.svg()) "Show month" }
            @if !month.is_empty() { a class="btn small" href=(base) { "All days" } }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::DailyLog), "No logs here", "Start a log for the day to record weather, crews, deliveries and notes.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Date" } th { "Status" } th { "Weather" } th class="num" { "Workers" } th class="num" { "Worker-hours" } th { "Author" } } }
                    tbody {
                        @for l in &shown {
                            @let (workers, tenths) = l.headcount();
                            tr {
                                td class="nowrap" { a class="row-link" href=(format!("{base}/{}", l.id)) { (l.date.pretty()) } }
                                td { (status_pill(l.status)) }
                                td { (weather_text(l)) }
                                td class="num" { (workers) }
                                td class="num" { (hours_text(tenths)) }
                                td { @if let Some(b) = l.created.by { (view::person_name(state, b)) } }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, "Daily log", Nav::DailyLogs(id), &crumbs(p, None), body)
}

fn text_card(title: &str, text: &str) -> Markup {
    html! { @if !text.is_empty() { div class="card" { h2 { (title) } p class="prose" { (text) } } } }
}

pub fn detail(req: &Req<'_>, id: Id<Project>, lid: Id<DailyLog>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(l) = state.daily_logs.get(lid).filter(|l| l.project == id && view::can_see_daily_log(state, req.actor(), l)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/daily-logs");
    let here = format!("{base}/{}", l.id);
    let ids = [("project", id.to_string()), ("id", l.id.to_string())];
    let editable = can_edit(state, req.actor(), l);
    let level = view::level(state, req.actor(), id, Tool::DailyLog);
    let (workers, tenths) = l.headcount();
    let body = html! {
        div class="page-head" {
            div {
                h1 { (l.date.pretty()) " " (status_pill(l.status)) }
                div class="sub" { "Started " (req.when(l.created.at)) @if let Some(b) = l.created.by { " by " (view::person_name(state, b)) } @if let Some(s) = l.submitted { " · submitted " (req.when(s.at)) } }
            }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if editable && l.status == LogStatus::Draft { (button_form(req, "daily_log.submit", &here, &ids, "Submit log", "primary")) }
                @if level == Level::Admin && l.status == LogStatus::Submitted { (button_form(req, "daily_log.reopen", &here, &ids, "Reopen as draft", "")) }
                @if level >= Level::Standard && (level == Level::Admin || (l.status == LogStatus::Draft && l.created.by == Some(req.person))) {
                    (button_form(req, "daily_log.delete", &base, &ids, "Delete", "danger"))
                }
            }
        }
        div class="stat-row" {
            div class="stat" { div class="label" { "Weather" } div class="value" { (l.weather.sky.label()) } @if l.weather.high.is_some() || l.weather.low.is_some() { div class="muted small" { "High " (l.weather.high.map(|h| h.to_string()).unwrap_or_else(|| "—".into())) " · low " (l.weather.low.map(|h| h.to_string()).unwrap_or_else(|| "—".into())) } } }
            div class="stat" { div class="label" { "Workers on site" } div class="value" { (workers) } }
            div class="stat" { div class="label" { "Worker-hours" } div class="value" { (hours_text(tenths)) } }
        }
        (text_card("Weather notes", &l.weather.notes))
        (text_card("Work performed", &l.work_performed))
        (text_card("Delays and issues", &l.delays))
        (text_card("Safety", &l.safety))
        (text_card("Notes", &l.notes))
        @if !l.manpower.is_empty() {
            div class="card" {
                h2 { "Manpower" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Company" } th { "Trade" } th class="num" { "Workers" } th class="num" { "Hours each" } th class="num" { "Worker-hours" } th { "Notes" } } }
                    tbody { @for m in &l.manpower { tr { td { (m.company) } td { (m.trade) } td class="num" { (m.workers) } td class="num" { (hours_text(u64::from(m.hours_tenths))) } td class="num" { (hours_text(m.total_tenths())) } td { (m.notes) } } } }
                    tfoot { tr { td colspan="2" { "Total" } td class="num" { (workers) } td {} td class="num" { (hours_text(tenths)) } td {} } }
                } }
            }
        }
        @if !l.equipment.is_empty() {
            div class="card" {
                h2 { "Equipment" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Equipment" } th class="num" { "Hours" } th { "Notes" } } }
                    tbody { @for e in &l.equipment { tr { td { (e.name) } td class="num" { (hours_text(u64::from(e.hours_tenths))) } td { (e.notes) } } } }
                } }
            }
        }
        @if !l.deliveries.is_empty() {
            div class="card" {
                h2 { "Deliveries" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Delivery" } th { "From" } th { "Notes" } } }
                    tbody { @for d in &l.deliveries { tr { td { (d.description) } td { (dash(&d.company)) } td { (d.notes) } } } }
                } }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if l.attachments.is_empty() { p class="muted" { "No files attached." } }
            @for a in &l.attachments {
                div class="row" {
                    a href=(format!("{here}/files/{}", a.sha256)) { (a.name) } span class="muted small" { (crate::pages::human_size(a.size)) }
                    @if editable { (button_form(req, "daily_log.detach", &here, &[("project", id.to_string()), ("id", l.id.to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
                }
            }
            @if editable {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
        @if !l.visitors.is_empty() {
            div class="card" {
                h2 { "Visitors" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Name" } th { "Company" } th { "Purpose" } } }
                    tbody { @for v in &l.visitors { tr { td { (v.name) } td { (dash(&v.company)) } td { (v.purpose) } } } }
                } }
            }
        }
    };
    layout::app(req, &l.label(), Nav::DailyLogs(id), &crumbs(p, Some(&l.date.pretty())), body)
}

/// Spare blank rows offered under the existing ones.
fn row_count(have: usize, min: usize) -> usize {
    (have + 2).max(min)
}

fn input(name: String, label: &str, value: String) -> Markup {
    html! { input type="text" name=(name) value=(value) aria-label=(label); }
}

/// The new/edit form. On a new log, `copy` fills the crews and equipment from the latest earlier
/// log the person can see.
pub fn form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<DailyLog>>, copy: bool) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(lid) => match state
            .daily_logs
            .get(lid)
            .filter(|l| l.project == id && view::can_see_daily_log(state, req.actor(), l) && can_edit(state, req.actor(), l))
        {
            Some(l) => Some(l),
            None => return not_found(req),
        },
        None => {
            if !req.allowed("daily_log.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let base = format!("/projects/{id}/daily-logs");
    let action = match current {
        Some(l) => format!("{base}/{}/save", l.id),
        None => format!("{base}/save"),
    };
    let text = |f: fn(&DailyLog) -> String| current.map(f).unwrap_or_default();
    let date = current.map(|l| l.date).unwrap_or_else(|| req.today());
    let sky = current.map(|l| l.weather.sky).unwrap_or_default();
    let num = |v: Option<i32>| v.map(|n| n.to_string()).unwrap_or_default();
    let previous = if current.is_none() {
        view::daily_logs_for(state, req.actor(), id).into_iter().filter(|l| l.date < date).max_by_key(|l| l.date)
    } else {
        None
    };
    let copied = previous.filter(|_| copy);
    let source = current.or(copied);
    let manpower = source.map(|l| l.manpower.as_slice()).unwrap_or(&[]);
    let equipment = source.map(|l| l.equipment.as_slice()).unwrap_or(&[]);
    let deliveries = current.map(|l| l.deliveries.as_slice()).unwrap_or(&[]);
    let visitors = current.map(|l| l.visitors.as_slice()).unwrap_or(&[]);
    let title = if current.is_some() { "Edit daily log" } else { "New daily log" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        @if let Some(prev) = previous {
            @if copied.is_some() { p class="muted" { "Crews and equipment copied from " (prev.date.pretty()) ". Change or remove what's different today." } }
            @else { p { a class="btn small" href=(format!("{base}/new?copy=1")) { "Copy crews and equipment from " (prev.date.pretty()) } } }
        }
        form method="post" action=(action) {
            input type="hidden" name="_csrf" value=(req.csrf);
            input type="hidden" name="_project" value=(id);
            div class="card" {
                h2 { "Day and weather" }
                div class="form-grid" {
                    (field("Date", "date", true, false, html! { input type="date" id="date" name="date" required value=(date.to_string()); }))
                    (field("Sky", "sky", false, false, html! { select id="sky" name="sky" { @for s in [Sky::Clear, Sky::PartlyCloudy, Sky::Overcast, Sky::Rain, Sky::Snow, Sky::Fog, Sky::Windy, Sky::Storm] { option value=(s.key()) selected[s == sky] { (s.label()) } } } }))
                    (field("High", "high", false, false, html! { input type="number" id="high" name="high" value=(num(current.and_then(|l| l.weather.high))); }))
                    (field("Low", "low", false, false, html! { input type="number" id="low" name="low" value=(num(current.and_then(|l| l.weather.low))); }))
                    (field("Weather notes", "weather_notes", false, true, html! { input type="text" id="weather_notes" name="weather_notes" value=(text(|l| l.weather.notes.clone())); }))
                }
            }
            div class="card" {
                h2 { "Notes" }
                div class="form-grid" {
                    (field("Work performed", "work_performed", false, true, html! { textarea id="work_performed" name="work_performed" { (text(|l| l.work_performed.clone())) } }))
                    (field("Delays and issues", "delays", false, false, html! { textarea id="delays" name="delays" class="short" { (text(|l| l.delays.clone())) } }))
                    (field("Safety", "safety", false, false, html! { textarea id="safety" name="safety" class="short" { (text(|l| l.safety.clone())) } }))
                    (field("Other notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (text(|l| l.notes.clone())) } }))
                }
            }
            div class="card" {
                h2 { "Manpower" }
                p class="muted small" { "One row per crew. Hours are per worker, like 7.5. Leave spare rows blank." }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Company" } th { "Trade" } th { "Workers" } th { "Hours each" } th { "Notes" } } }
                    tbody { @for i in 0..row_count(manpower.len(), 5) {
                        @let m = manpower.get(i);
                        tr {
                            td { (input(format!("manpower.{i}.company"), "Company", m.map(|m| m.company.clone()).unwrap_or_default())) }
                            td { (input(format!("manpower.{i}.trade"), "Trade", m.map(|m| m.trade.clone()).unwrap_or_default())) }
                            td { (input(format!("manpower.{i}.workers"), "Workers", m.map(|m| m.workers.to_string()).unwrap_or_default())) }
                            td { (input(format!("manpower.{i}.hours"), "Hours each", m.map(|m| hours_text(u64::from(m.hours_tenths))).unwrap_or_default())) }
                            td { (input(format!("manpower.{i}.notes"), "Notes", m.map(|m| m.notes.clone()).unwrap_or_default())) }
                        }
                    } }
                } }
            }
            div class="card" {
                h2 { "Equipment" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Equipment" } th { "Hours" } th { "Notes" } } }
                    tbody { @for i in 0..row_count(equipment.len(), 3) {
                        @let e = equipment.get(i);
                        tr {
                            td { (input(format!("equipment.{i}.name"), "Equipment", e.map(|e| e.name.clone()).unwrap_or_default())) }
                            td { (input(format!("equipment.{i}.hours"), "Hours", e.map(|e| hours_text(u64::from(e.hours_tenths))).unwrap_or_default())) }
                            td { (input(format!("equipment.{i}.notes"), "Notes", e.map(|e| e.notes.clone()).unwrap_or_default())) }
                        }
                    } }
                } }
            }
            div class="card" {
                h2 { "Deliveries" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Delivery" } th { "From" } th { "Notes" } } }
                    tbody { @for i in 0..row_count(deliveries.len(), 3) {
                        @let d = deliveries.get(i);
                        tr {
                            td { (input(format!("deliveries.{i}.description"), "Delivery", d.map(|d| d.description.clone()).unwrap_or_default())) }
                            td { (input(format!("deliveries.{i}.company"), "From", d.map(|d| d.company.clone()).unwrap_or_default())) }
                            td { (input(format!("deliveries.{i}.notes"), "Notes", d.map(|d| d.notes.clone()).unwrap_or_default())) }
                        }
                    } }
                } }
            }
            div class="card" {
                h2 { "Visitors" }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Name" } th { "Company" } th { "Purpose" } } }
                    tbody { @for i in 0..row_count(visitors.len(), 3) {
                        @let v = visitors.get(i);
                        tr {
                            td { (input(format!("visitors.{i}.name"), "Name", v.map(|v| v.name.clone()).unwrap_or_default())) }
                            td { (input(format!("visitors.{i}.company"), "Company", v.map(|v| v.company.clone()).unwrap_or_default())) }
                            td { (input(format!("visitors.{i}.purpose"), "Purpose", v.map(|v| v.purpose.clone()).unwrap_or_default())) }
                        }
                    } }
                } }
            }
            div class="form-actions" {
                a class="btn" href=(current.map(|l| format!("{base}/{}", l.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                button type="submit" name="_submit" value="1" class="btn" { "Save and submit" }
                button type="submit" class="btn primary" { "Save draft" }
            }
        }
    };
    layout::app(req, title, Nav::DailyLogs(id), &crumbs(p, Some(title)), body)
}

// -- weekly summary

/// One day of the weekly summary.
pub struct WeekDay {
    pub date: buildkiosk_core::Date,
    pub log: Option<u64>,
    pub status: Option<LogStatus>,
    pub weather: String,
    pub workers: u64,
    pub tenths: u64,
    pub work: String,
    pub delays: String,
    pub safety: String,
}

pub struct Week {
    pub start: buildkiosk_core::Date,
    pub days: Vec<WeekDay>,
    pub worker_days: u64,
    pub tenths: u64,
    pub logged: usize,
}

/// The Monday-to-Sunday week that holds `any_day`, from the logs the person can see.
pub fn week_data(req: &Req<'_>, id: Id<Project>, any_day: buildkiosk_core::Date) -> Option<Week> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::DailyLog) < Level::Read {
        return None;
    }
    let back = i64::try_from(buildkiosk_core::date::Weekday::ALL.iter().position(|w| *w == any_day.weekday()).unwrap_or(0)).unwrap_or(0);
    let start = any_day.add_days(-back)?;
    let logs = view::daily_logs_for(state, req.actor(), id);
    let mut days = Vec::new();
    for i in 0..7 {
        let date = start.add_days(i)?;
        let log = logs.iter().copied().find(|l| l.date == date);
        let (workers, tenths) = log.map(|l| l.headcount()).unwrap_or((0, 0));
        days.push(WeekDay {
            date,
            log: log.map(|l| l.id.raw()),
            status: log.map(|l| l.status),
            weather: log.map(weather_text).unwrap_or_default(),
            workers,
            tenths,
            work: log.map(|l| l.work_performed.clone()).unwrap_or_default(),
            delays: log.map(|l| l.delays.clone()).unwrap_or_default(),
            safety: log.map(|l| l.safety.clone()).unwrap_or_default(),
        });
    }
    Some(Week {
        start,
        worker_days: days.iter().map(|d| d.workers).sum(),
        tenths: days.iter().map(|d| d.tenths).sum(),
        logged: days.iter().filter(|d| d.log.is_some()).count(),
        days,
    })
}

/// A week at a glance: weather, crews, work, delays and safety for each day, with totals.
pub fn week(req: &Req<'_>, id: Id<Project>, any_day: buildkiosk_core::Date) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let Some(w) = week_data(req, id, any_day) else { return not_found(req) };
    let base = format!("/projects/{id}/daily-logs");
    let prev = w.start.add_days(-7).map(|d| d.to_string()).unwrap_or_default();
    let next = w.start.add_days(7).map(|d| d.to_string()).unwrap_or_default();
    let end = w.start.add_days(6).unwrap_or(w.start);
    let body = html! {
        div class="page-head" {
            div { h1 { "Week of " (w.start.pretty()) } div class="sub" { (p.name) " · " (w.logged) " of 7 days logged · " (w.worker_days) " worker-days · " (hours_text(w.tenths)) " worker-hours" } }
            div class="actions" {
                a class="btn" href=(format!("{base}/week?date={prev}")) { "Previous week" }
                a class="btn" href=(format!("{base}/week?date={next}")) { "Next week" }
                a class="btn" href=(format!("{base}/week/pdf?date={}", w.start)) { "PDF" }
                a class="btn" href=(base.clone()) { "All logs" }
            }
        }
        div class="table-wrap" {
            table class="data" {
                thead { tr { th { "Day" } th { "Weather" } th class="num" { "Workers" } th class="num" { "Worker-hours" } th { "Work performed" } th { "Delays" } th { "Safety" } } }
                tbody { @for d in &w.days { tr {
                    td class="nowrap" {
                        @if let Some(l) = d.log { a class="row-link" href=(format!("{base}/{l}")) { (d.date.weekday().short()) " " (d.date.pretty()) } }
                        @else { (d.date.weekday().short()) " " (d.date.pretty()) }
                        @if d.status == Some(LogStatus::Draft) { " " span class="tag" { "Draft" } }
                    }
                    td { @if d.log.is_some() { (d.weather) } @else { span class="muted" { "No log" } } }
                    td class="num" { @if d.log.is_some() { (d.workers) } }
                    td class="num" { @if d.log.is_some() { (hours_text(d.tenths)) } }
                    td { (d.work) } td { (d.delays) } td { (d.safety) }
                } } }
            }
        }
        p class="muted small" { "Week ends " (end.pretty()) "." }
    };
    layout::app(req, "Weekly summary", Nav::DailyLogs(id), &crumbs(p, Some("Week")), body)
}
