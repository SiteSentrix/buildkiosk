//! The timesheets page: a person's week with hours by day, and for admins the week's approvals.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::cmd::timesheet::{crews, rate_list, report, visible};
use buildkiosk_engine::model::daily_log::hours_text;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::timesheet::{TimeEntry, TimeStatus};
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, field};
use crate::pages::{not_found, project_for, user_options};

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
    let people = if admin { user_options(req, id, true) } else { Vec::new() };
    let saved_crews = if admin { crews(state, id) } else { Vec::new() };
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
                @if admin { a class="btn" href=(format!("{base}/cost")) { "Labour cost" } }
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
                @if admin {
                    details class="disclosure" {
                        summary { "Log the same hours for a crew" }
                        @if !saved_crews.is_empty() {
                            (field("A saved crew", "crew", false, false, html! { select id="crew" name="crew" { option value="" { "None" } @for c in &saved_crews { option value=(c.id) { (c.name) " (" (c.members.len()) ")" } } } }))
                        }
                        input type="hidden" name="people[]" value="";
                        div class="options" { @for x in &people { label { input type="checkbox" name="people[]" value=(x.id); (x.name()) } } }
                        p class="muted small" { "Each ticked person gets their own entry for this day. With nobody ticked, the hours are yours." }
                    }
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
    let body = html! {
        (body)
        @if admin {
            div class="card" {
                h2 { "Crews" }
                @if saved_crews.is_empty() { p class="muted" { "A crew is a named group you log hours for together." } }
                @for c in &saved_crews {
                    details class="disclosure" {
                        summary { (c.name) " · " (c.members.len()) " people" }
                        (command_form(req, "crew.save", &here, &here, html! {
                            input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(c.id);
                            div class="form-grid" { (field("Name", &format!("cr_n{}", c.id), true, false, html! { input type="text" id=(format!("cr_n{}", c.id)) name="name" required value=(c.name); })) }
                            input type="hidden" name="members[]" value="";
                            div class="options" { @for x in &people { label { input type="checkbox" name="members[]" value=(x.id) checked[c.members.contains(&x.id)]; (x.name()) } } }
                            div class="form-actions" { button type="submit" class="btn small primary" { "Save crew" } }
                        }))
                        (button_form(req, "crew.delete", &here, &[("project", id.to_string()), ("id", c.id.to_string())], "Delete crew", "small danger"))
                    }
                }
                (command_form(req, "crew.save", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" { (field("New crew", "cr_new", true, false, html! { input type="text" id="cr_new" name="name" required placeholder="Concrete crew"; })) }
                    input type="hidden" name="members[]" value="";
                    div class="options" { @for x in &people { label { input type="checkbox" name="members[]" value=(x.id); (x.name()) } } }
                    div class="form-actions" { button type="submit" class="btn small primary" { "Add crew" } }
                }))
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
    let mut out = String::from("Date,Person,Hours,Cost code,Description,Status,Submitted,Approved by,Approved\r\n");
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
            e.submitted.map(|x| x.at.date_at(state.utc_offset()).to_string()).unwrap_or_default(),
            e.approved.and_then(|x| x.by).map(|x| view::person_name(state, x)).unwrap_or_default(),
            e.approved.map(|x| x.at.date_at(state.utc_offset()).to_string()).unwrap_or_default(),
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

fn code_label(state: &buildkiosk_engine::model::CompanyState, c: Option<Id<buildkiosk_engine::model::project::CostCode>>) -> String {
    c.and_then(|c| state.cost_codes.get(c)).map(|c| format!("{} {}", c.code, c.description)).unwrap_or_else(|| "No cost code".into())
}

/// Labour cost: hours split into regular and overtime and priced, by cost code and by person, with the
/// pay rates, the overtime rules and the button that posts approved hours to direct costs.
pub fn cost_page(req: &Req<'_>, id: Id<Project>, from: Option<Date>, to: Option<Date>, all: bool) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Timesheets) < Level::Admin {
        return not_found(req);
    }
    let base = format!("/projects/{id}/timesheets");
    let here = format!("{base}/cost");
    let r = report(state, id, from, to, !all);
    let rates = rate_list(state, id);
    let rules = p.overtime;
    let people = user_options(req, id, true);
    let codes = view::project_cost_codes(state, id);
    let (f, t) = (from.map(|d| d.to_string()).unwrap_or_default(), to.map(|d| d.to_string()).unwrap_or_default());
    let unposted = r.lines.iter().any(|l| l.status == TimeStatus::Approved && !l.posted);
    let post_from = from.unwrap_or_else(|| week_start(req.today()));
    let body = html! {
        div class="page-head" {
            div { h1 { "Labour cost" } div class="sub" { "Hours priced at each person's rate, with overtime under the project's rules" } }
            div class="actions" {
                a class="btn" href=(format!("{base}/cost.csv?from={f}&to={t}&include={}", if all { "all" } else { "approved" })) { "Export CSV" }
                a class="btn" href=(base) { "Back to timesheets" }
            }
        }
        form method="get" action=(here) class="filters" {
            input type="date" name="from" value=(f) aria-label="From";
            input type="date" name="to" value=(t) aria-label="To";
            select name="include" aria-label="Hours counted" {
                option value="approved" selected[!all] { "Approved hours only" }
                option value="all" selected[all] { "All hours, including waiting and draft" }
            }
            button type="submit" class="btn" { "Show" }
        }
        @if !r.no_rate.is_empty() {
            div class="card" { p { span class="pill amber" { "No pay rate" } " " (r.no_rate.iter().map(|x| view::person_name(state, *x)).collect::<Vec<_>>().join(", ")) " — their hours count as zero cost until a rate is set below." } }
        }
        div class="card" {
            h2 { "By cost code" }
            @if r.by_code.is_empty() { p class="muted" { "No hours in this range." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Cost code" } th class="num" { "Regular hours" } th class="num" { "Overtime hours" } th class="num" { "Cost" } } }
                    tbody { @for (c, tot) in &r.by_code { tr { td { (code_label(state, *c)) } td class="num" { (hours_text(u64::from(tot.regular))) } td class="num" { (hours_text(u64::from(tot.overtime))) } td class="num" { (tot.cost) } } } }
                    tfoot { tr { td { "Total" } td class="num" { (hours_text(u64::from(r.total.regular))) } td class="num" { (hours_text(u64::from(r.total.overtime))) } td class="num" { (r.total.cost) } } }
                } }
            }
        }
        div class="card" {
            h2 { "By person" }
            @if r.by_person.is_empty() { p class="muted" { "No hours in this range." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Person" } th class="num" { "Regular hours" } th class="num" { "Overtime hours" } th class="num" { "Cost" } } }
                    tbody { @for (x, tot) in &r.by_person { tr { td { (view::person_name(state, *x)) } td class="num" { (hours_text(u64::from(tot.regular))) } td class="num" { (hours_text(u64::from(tot.overtime))) } td class="num" { (tot.cost) } } } }
                } }
            }
        }
        @if unposted && !all {
            div class="card" {
                h2 { "Post to direct costs" }
                p class="muted" { "Approved hours in the range that were not posted before become one labour direct cost for each cost code, so they count in the budget. Everyone needs a pay rate first." }
                (command_form(req, "timesheet.post_costs", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("From", "post_from", true, false, html! { input type="date" id="post_from" name="from" required value=(from.map(|d| d.to_string()).unwrap_or_else(|| post_from.to_string())); }))
                        (field("To", "post_to", false, false, html! { input type="date" id="post_to" name="to" value=(t); }))
                        (field("Cost code for hours without one", "post_code", false, false, html! { select id="post_code" name="default_cost_code" { option value="" { "—" } @for c in &codes { option value=(c.id) { (c.code) " " (c.description) } } } }))
                    }
                    p class="muted small" { "Leave To empty to post the seven days from From." }
                    div class="form-actions" { button type="submit" class="btn primary" { "Post labour" } }
                }))
            }
        }
        div class="card" {
            h2 { "Overtime rules" }
            (command_form(req, "timesheet.set_rules", &here, &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (field("Daily hours before overtime", "ot_day", false, false, html! { input type="text" id="ot_day" name="daily_hours" inputmode="decimal" value=(hours_text(u64::from(rules.daily_tenths))) placeholder="8"; }))
                    (field("Weekly hours before overtime", "ot_week", false, false, html! { input type="text" id="ot_week" name="weekly_hours" inputmode="decimal" value=(hours_text(u64::from(rules.weekly_tenths))) placeholder="40"; }))
                    (field("Overtime pays (percent of the rate)", "ot_mult", false, false, html! { input type="text" id="ot_mult" name="multiplier" inputmode="decimal" value=(rules.pay_multiplier().plain()); }))
                }
                p class="muted small" { "0 switches a rule off. Weeks run Monday to Sunday and hours already paid as daily overtime are not counted again." }
                div class="form-actions" { button type="submit" class="btn primary small" { "Save rules" } }
            }))
        }
        div class="card" {
            h2 { "Pay rates" }
            @if rates.is_empty() { p class="muted" { "No rates set." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Person" } th { "From" } th class="num" { "Per hour" } th {} } }
                    tbody { @for rt in &rates { tr {
                        td { (view::person_name(state, rt.person)) } td { (rt.from.to_string()) } td class="num" { (rt.hourly) }
                        td class="right" { (button_form(req, "timesheet.delete_rate", &here, &[("project", id.to_string()), ("id", rt.id.to_string())], "Remove", "small danger")) }
                    } } }
                } }
            }
            (command_form(req, "timesheet.set_rate", &here, &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (field("Person", "rate_person", true, false, html! { select id="rate_person" name="person" required { @for x in &people { option value=(x.id) { (x.name()) } } } }))
                    (field("Per hour", "rate_amount", true, false, html! { input type="text" id="rate_amount" name="hourly" required inputmode="decimal" placeholder="38.50"; }))
                    (field("From", "rate_from", false, false, html! { input type="date" id="rate_from" name="from" value=(req.today().to_string()); }))
                }
                div class="form-actions" { button type="submit" class="btn primary small" { "Set rate" } }
            }))
        }
    };
    layout::app(
        req,
        "Labour cost",
        Nav::Timesheets(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Timesheets", Some(base)), ("Labour cost", None)],
        body,
    )
}

/// The cost report's lines (one per entry) as CSV.
pub fn cost_csv(req: &Req<'_>, id: Id<Project>, from: Option<Date>, to: Option<Date>, all: bool) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Timesheets) < Level::Admin {
        return None;
    }
    let r = report(state, id, from, to, !all);
    let mut out = String::from("Date,Person,Cost code,Regular hours,Overtime hours,Rate,Cost,Status,Posted\r\n");
    for l in &r.lines {
        let cells = [
            l.date.to_string(),
            view::person_name(state, l.person),
            code_label(state, l.cost_code),
            hours_text(u64::from(l.regular)),
            hours_text(u64::from(l.overtime)),
            l.rate.map(|m| m.plain()).unwrap_or_default(),
            l.cost.plain(),
            l.status.label().to_string(),
            if l.posted { "yes".into() } else { "no".into() },
        ];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
