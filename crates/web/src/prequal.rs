//! The prequalification page: every company's record, who needs attention, and review.

use buildkiosk_core::Date;
use buildkiosk_engine::cmd::prequal::visible;
use buildkiosk_engine::model::prequal::{PrequalStatus, Prequalification, Standing};
use buildkiosk_engine::model::{Choice, Level, Tool, perm};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::not_found;

fn status_pill(s: PrequalStatus) -> Markup {
    let class = match s {
        PrequalStatus::Pending => "violet",
        PrequalStatus::Submitted => "amber",
        PrequalStatus::Approved => "green",
        PrequalStatus::Rejected => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn expiry(d: Option<Date>, today: Date) -> Markup {
    html! {
        @if let Some(d) = d {
            @if d < today { span class="overdue" { (d.to_string()) " (lapsed)" } }
            @else if today.days_until(d) <= Prequalification::WARN_DAYS { span class="pill amber" { (d.to_string()) } }
            @else { (d.to_string()) }
        } @else { span class="muted" { "—" } }
    }
}

pub fn page(
    req: &Req<'_>,
    status: Option<PrequalStatus>,
    attention: bool,
    edit: Option<buildkiosk_core::Id<buildkiosk_engine::model::directory::Vendor>>,
) -> Markup {
    let state = req.engine.state();
    let lvl = perm::company_level(state, req.person, Tool::Prequalification);
    if lvl < Level::Read {
        return not_found(req);
    }
    let today = req.today();
    let all = visible(state);
    let lapsing = all.iter().filter(|r| matches!(r.standing(today), Standing::ExpiringSoon | Standing::Lapsed)).count();
    let shown: Vec<&Prequalification> = all
        .iter()
        .copied()
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| !attention || matches!(r.standing(today), Standing::ExpiringSoon | Standing::Lapsed))
        .collect();
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let cur = edit.and_then(|v| all.iter().copied().find(|r| r.vendor == v));
    let can_edit = lvl >= Level::Standard;
    let admin = lvl == Level::Admin;
    let here = "/prequalification";
    let vname = |v| buildkiosk_engine::view::vendor_name(state, v);
    let body = html! {
        div class="page-head" { div { h1 { "Prequalification" } div class="sub" { "Which companies have been vetted to work for you, and when their approval, insurance or licence lapses" } } }
        @if lapsing > 0 { div class="flash error" role="status" { (lapsing) " approved compan" @if lapsing == 1 { "y has" } @else { "ies have" } " something that has lapsed or lapses within " (Prequalification::WARN_DAYS) " days. " a href="/prequalification?attention=1" { "Show them" } } }
        form method="get" action=(here) class="filters" {
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in PrequalStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            label { input type="checkbox" name="attention" value="1" checked[attention]; " Needs attention" }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Prequalification), "No records", if can_edit { "Add a company's prequalification below." } else { "Nothing recorded yet." })) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Company" } th { "Status" } th class="num" { "Score" } th { "Approved until" } th { "Insurance" } th { "Licence" } th class="num" { "EMR" } th class="num" { "Bonding" } th {} } }
                tbody { @for r in &shown { tr {
                    td { strong { (vname(r.vendor)) } @if !r.notes.is_empty() { br; span class="muted small" { (r.notes) } } }
                    td { (status_pill(r.status)) }
                    td class="num" { (r.score.map(|s| s.to_string()).unwrap_or_default()) }
                    td class="nowrap" { (expiry(r.valid_until, today)) }
                    td class="nowrap" { (expiry(r.insurance_expires, today)) }
                    td class="nowrap" { (expiry(r.license_expires, today)) }
                    td class="num" { (r.emr_hundredths.map(|e| format!("{}.{:02}", e / 100, e % 100)).unwrap_or_default()) }
                    td class="num" { (r.bonding_capacity.map(|m| m.to_string()).unwrap_or_default()) }
                    td class="right" {
                        @if can_edit { a class="btn small" href=(format!("{here}?edit={}", r.vendor)) { "Edit" } " " }
                        @if can_edit && r.status == PrequalStatus::Pending { (button_form(req, "prequal.submit", here, &[("vendor", r.vendor.to_string())], "Put forward", "small")) " " }
                        @if admin && matches!(r.status, PrequalStatus::Pending | PrequalStatus::Submitted) {
                            (button_form(req, "prequal.review", here, &[("vendor", r.vendor.to_string()), ("decision", "approved".into())], "Approve", "small primary")) " "
                            (button_form(req, "prequal.review", here, &[("vendor", r.vendor.to_string()), ("decision", "rejected".into())], "Reject", "small danger")) " "
                        }
                        @if admin && matches!(r.status, PrequalStatus::Approved | PrequalStatus::Rejected) { (button_form(req, "prequal.reset", here, &[("vendor", r.vendor.to_string())], "Reset", "small")) }
                    }
                } } }
            } }
        }
        @if can_edit {
            div class="card" {
                h2 { (if cur.is_some() { "Edit record" } else { "Add or update a record" }) }
                (command_form(req, "prequal.save", here, here, html! {
                    div class="form-grid" {
                        (field("Company", "vendor", true, false, html! { select id="vendor" name="vendor" required { @for v in &vendors { option value=(v.id) selected[cur.is_some_and(|r| r.vendor == v.id)] { (v.name) } } } }))
                        (field("Approved until", "valid_until", false, false, html! { input type="date" id="valid_until" name="valid_until" value=(cur.and_then(|r| r.valid_until).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Insurance expires", "insurance_expires", false, false, html! { input type="date" id="insurance_expires" name="insurance_expires" value=(cur.and_then(|r| r.insurance_expires).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Licence expires", "license_expires", false, false, html! { input type="date" id="license_expires" name="license_expires" value=(cur.and_then(|r| r.license_expires).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Experience modification rate", "emr", false, false, html! { input type="text" id="emr" name="emr" inputmode="decimal" placeholder="0.85" value=(cur.and_then(|r| r.emr_hundredths).map(|e| format!("{}.{:02}", e / 100, e % 100)).unwrap_or_default()); }))
                        (field("Bonding capacity", "bonding_capacity", false, false, html! { input type="text" id="bonding_capacity" name="bonding_capacity" inputmode="decimal" value=(cur.and_then(|r| r.bonding_capacity).map(|m| m.plain()).unwrap_or_default()); }))
                        (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (cur.map(|r| r.notes.as_str()).unwrap_or_default()) } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                }))
            }
        }
    };
    layout::app(req, "Prequalification", Nav::Prequalification, &[("Prequalification", None)], body)
}
