//! The timesheets page: a person's week with hours by day, and for admins the week's approvals.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::cmd::timesheet::visible;
use buildkiosk_engine::model::daily_log::hours_text;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::timesheet::{TimeEntry, TimeStatus};
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, field};
use crate::pages::{not_found, project_for};

/// The Monday on or before `d`.
pub fn week_start(d: Date) -> Date {
    let back = d.weekday() as i64;
    d.add_days(-back).unwrap_or(d)
}

fn pill(s: TimeStatus) -> Markup {
    let class = match s {
        TimeStatus::Draft => "violet",
        TimeStatus::Submitted => "amber",
        TimeStatus::Approved => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

pub fn page(req: &Req<'_>, id: Id<Project>, week: Option<Date>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Timesheets);
    if lvl < Level::Standard {
        return not_found(req);
    }
    let admin = lvl == Level::Admin;
    let start = week_start(week.unwrap_or_else(|| req.today()));
    let end = start.add_days(6).unwrap_or(start);
    let (prev, next) = (start.add_days(-7).unwrap_or(start), start.add_days(7).unwrap_or(start));
    let base = format!("/projects/{id}/timesheets");
    let here = format!("{base}?week={start}");
    let me = req.actor().person();
    let all: Vec<&TimeEntry> = visible(state, req.actor(), id).into_iter().filter(|e| e.date >= start && e.date <= end).collect();
    let mine: Vec<&TimeEntry> = all.iter().copied().filter(|e| Some(e.person) == me).collect();
    let total = |v: &[&TimeEntry]| hours_text(v.iter().map(|e| u64::from(e.tenths)).sum());
    let drafts = mine.iter().filter(|e| e.status == TimeStatus::Draft).count();
    let codes = view::project_cost_codes(state, id);
    let days: Vec<Date> = (0..7).filter_map(|i| start.add_days(i)).collect();
    // Admin: one row per person for the week.
    let mut by_person: BTreeMap<String, (buildkiosk_core::Id<buildkiosk_engine::model::directory::Person>, u64, u64, u64)> = BTreeMap::new();
    if admin {
        for e in &all {
            let row = by_person.entry(view::person_name(state, e.person)).or_insert((e.person, 0, 0, 0));
            let t = u64::from(e.tenths);
            match e.status {
                TimeStatus::Draft => row.1 += t,
                TimeStatus::Submitted => row.2 += t,
                TimeStatus::Approved => row.3 += t,
            }
        }
    }
    let body = html! {
        div class="page-head" {
            div { h1 { "Timesheets" } div class="sub" { "Week of " (start.pretty()) " to " (end.pretty()) } }
            div class="actions" {
                a class="btn" href=(format!("{base}?week={prev}")) { "← Previous week" }
                a class="btn" href=(format!("{base}?week={next}")) { "Next week →" }
                @if admin { a class="btn" href=(format!("{base}.csv?week={start}")) { "Export CSV" } }
            }
        }
        div class="card" {
            h2 { "My hours" }
            @if mine.is_empty() { p class="muted" { "No hours logged this week." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Day" } th class="num" { "Hours" } th { "Cost code" } th { "What" } th { "Status" } th {} } }
                    tbody { @for e in &mine { tr {
                        td class="nowrap" { (e.date.weekday().short()) " " (e.date.to_string()) }
                        td class="num" { (hours_text(u64::from(e.tenths))) }
                        td { (e.cost_code.and_then(|c| state.cost_codes.get(c)).map(|c| c.code.clone()).unwrap_or_default()) }
                        td { (e.description) @if !e.returned_note.is_empty() { " " span class="pill amber" { "Sent back: " (e.returned_note) } } }
                        td { (pill(e.status)) }
                        td class="right" { @if e.status == TimeStatus::Draft { (button_form(req, "timesheet.delete", &here, &[("project", id.to_string()), ("id", e.id.to_string())], "Delete", "small danger")) } }
                    } } }
                    tfoot { tr { td { "Total" } td class="num" { (total(&mine)) } td colspan="4" {} } }
                } }
            }
            @if drafts > 0 {
                (button_form(req, "timesheet.submit", &here, &[("project", id.to_string()), ("from", start.to_string()), ("to", end.to_string())], "Submit the week for approval", "primary"))
            }
        }
        div class="card" {
            h2 { "Log hours" }
            (command_form(req, "timesheet.add", &here, &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (field("Day", "date", true, false, html! { select id="date" name="date" { @for d in &days { option value=(d.to_string()) selected[*d == req.today()] { (d.weekday().short()) " " (d.to_string()) } } } }))
                    (field("Hours", "hours", true, false, html! { input type="text" id="hours" name="hours" required inputmode="decimal" placeholder="7.5"; }))
                    (field("Cost code", "cost_code", false, false, html! { select id="cost_code" name="cost_code" { option value="" { "—" } @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } }))
                    (field("What you did", "description", false, false, html! { input type="text" id="description" name="description"; }))
                }
                div class="form-actions" { button type="submit" class="btn primary" { "Add hours" } }
            }))
        }
        @if admin {
            div class="card" {
                h2 { "Everyone this week" }
                @if by_person.is_empty() { p class="muted" { "Nobody has logged hours this week." } } @else {
                    div class="table-wrap" { table class="data" {
                        thead { tr { th { "Person" } th class="num" { "Draft" } th class="num" { "Waiting for approval" } th class="num" { "Approved" } th {} } }
                        tbody { @for (name, (pid, d, s, a)) in &by_person { tr {
                            td { (name) }
                            td class="num" { (hours_text(*d)) } td class="num" { (hours_text(*s)) } td class="num" { (hours_text(*a)) }
                            td class="right" { @if *s > 0 {
                                (button_form(req, "timesheet.approve", &here, &[("project", id.to_string()), ("person", pid.to_string()), ("from", start.to_string()), ("to", end.to_string())], "Approve", "small primary"))
                                " "
                                (button_form(req, "timesheet.return", &here, &[("project", id.to_string()), ("person", pid.to_string()), ("from", start.to_string()), ("to", end.to_string())], "Send back", "small"))
                            } }
                        } } }
                    } }
                }
            }
        }
    };
    layout::app(
        req,
        "Timesheets",
        Nav::Timesheets(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Timesheets", None)],
        body,
    )
}

/// The week's entries (everyone's for an admin) as CSV.
pub fn csv(req: &Req<'_>, id: Id<Project>, week: Option<Date>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Timesheets) < Level::Admin {
        return None;
    }
    let start = week_start(week.unwrap_or_else(|| req.today()));
    let end = start.add_days(6).unwrap_or(start);
    let mut out = String::from("Date,Person,Hours,Cost code,Description,Status\r\n");
    let mut rows: Vec<&TimeEntry> = visible(state, req.actor(), id).into_iter().filter(|e| e.date >= start && e.date <= end).collect();
    rows.sort_by_key(|e| (e.date, e.person));
    for e in rows {
        let cells = [
            e.date.to_string(),
            view::person_name(state, e.person),
            hours_text(u64::from(e.tenths)),
            e.cost_code.and_then(|c| state.cost_codes.get(c)).map(|c| c.code.clone()).unwrap_or_default(),
            e.description.clone(),
            e.status.label().to_string(),
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
