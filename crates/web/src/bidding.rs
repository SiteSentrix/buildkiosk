//! Bidding pages: packages, bid leveling and the form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::bidding::{spread, visible};
use buildkiosk_engine::model::bidding::{BidPackage, BidStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{not_found, project_for};

fn status_pill(s: BidStatus) -> Markup {
    let class = match s {
        BidStatus::Draft => "violet",
        BidStatus::Open => "blue",
        BidStatus::Closed => "amber",
        BidStatus::Awarded => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn vendors<'a>(req: &'a Req<'_>) -> Vec<&'a buildkiosk_engine::model::directory::Vendor> {
    let mut v: Vec<_> = req.engine.state().vendors.iter().filter(|v| v.deleted.is_none()).collect();
    v.sort_by_key(|v| v.name.to_lowercase());
    v
}

pub fn list(req: &Req<'_>, id: Id<Project>, status: Option<BidStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Bidding);
    if lvl < Level::Read {
        return not_found(req);
    }
    let needle = q.trim().to_lowercase();
    let shown: Vec<&BidPackage> = visible(state, id)
        .into_iter()
        .filter(|b| status.is_none_or(|s| b.status == s))
        .filter(|b| needle.is_empty() || b.title.to_lowercase().contains(&needle) || b.scope.to_lowercase().contains(&needle))
        .collect();
    let base = format!("/projects/{id}/bidding");
    let body = html! {
        div class="page-head" {
            div { h1 { "Bidding" } div class="sub" { "Packages of work out to bid, the bids received and the award" } }
            div class="actions" { @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New bid package" } } }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search title or scope" aria-label="Search bid packages";
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in BidStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Bidding), "No bid packages", "Create a package, invite companies and record the bids as they arrive.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Package" } th { "Bids due" } th class="num" { "Invited" } th class="num" { "Bids" } th class="num" { "Low bid" } th { "Status" } } }
                tbody { @for b in &shown { tr {
                    td { (b.number) }
                    td { a href=(format!("{base}/{}", b.id)) { (b.title) } }
                    td class="nowrap" { (b.due.map(|d| d.to_string()).unwrap_or_default()) }
                    td class="num" { (b.invited.len()) }
                    td class="num" { (b.standing().len()) }
                    td class="num" { @if let Some((low, _, _)) = spread(b) { (low) } }
                    td { (status_pill(b.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Bidding",
        Nav::Bidding(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Bidding", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, bid: Id<BidPackage>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Bidding);
    let Some(b) = state.bid_packages.get(bid).filter(|b| b.project == id && b.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/bidding");
    let here = format!("{base}/{bid}");
    let ids = [("project", id.to_string()), ("id", bid.to_string())];
    let admin = lvl == Level::Admin;
    let can_edit = lvl >= Level::Standard;
    let standing = b.standing();
    let low = standing.first().map(|x| x.amount);
    let budget = b.cost_code.and_then(|c| {
        if view::level(state, req.actor(), id, Tool::Budget) >= Level::Read {
            view::budget_rows(state, id).ok().and_then(|rows| rows.into_iter().find(|r| r.cost_code == c)).map(|r| r.revised())
        } else {
            None
        }
    });
    let vname = |v| view::vendor_name(state, v);
    let all = vendors(req);
    let body = html! {
        div class="page-head" {
            div { h1 { (b.label()) ": " (b.title) " " (status_pill(b.status)) } div class="sub" { @if let Some(d) = b.due { "Bids due " (d.pretty()) " · " } @if let Some(c) = b.cost_code.and_then(|c| state.cost_codes.get(c)) { "Cost code " (c.code) " " (c.description) } } }
            div class="actions" {
                @if can_edit && b.status != BidStatus::Awarded { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if can_edit && b.status == BidStatus::Draft { (button_form(req, "bid_package.open", &here, &ids, "Send out for bid", "primary")) }
                @if can_edit && b.status == BidStatus::Open { (button_form(req, "bid_package.close", &here, &ids, "Close bidding", "")) }
                @if can_edit && b.status == BidStatus::Closed { (button_form(req, "bid_package.reopen", &here, &ids, "Reopen bidding", "")) }
                @if admin { (button_form(req, "bid_package.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if !b.scope.is_empty() { div class="card" { h2 { "Scope" } p class="prose" { (b.scope) } } }
        div class="card" {
            h2 { "Bids" }
            @if let Some(x) = budget { p class="muted" { "Revised budget for this cost code: " strong { (x) } } }
            @if b.bids.is_empty() { p class="muted" { "No bids yet." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Company" } th class="num" { "Bid" } th class="num" { "Over low bid" } @if budget.is_some() { th class="num" { "Against budget" } } th { "Received" } th { "Notes" } th {} } }
                    tbody {
                        @for x in &standing { tr class=(if b.awarded_to == Some(x.vendor) { "awarded" } else { "" }) {
                            td { strong { (vname(x.vendor)) } @if b.awarded_to == Some(x.vendor) { " " span class="pill green" { "Awarded" } } @if Some(x.amount) == low { " " span class="pill blue" { "Low" } } }
                            td class="num" { (x.amount) }
                            td class="num" { @if let (Some(l), true) = (low, Some(x.amount) != low) { (x.amount.checked_sub(l).unwrap_or_default()) } }
                            @if let Some(bg) = budget { td class="num" { (x.amount.checked_sub(bg).map(|d| html! { span class=(if d.is_negative() { "" } else { "overdue" }) { (d) } }).unwrap_or_default()) } }
                            td class="nowrap" { (x.received.to_string()) }
                            td { (x.notes) }
                            td class="right" {
                                @if admin && matches!(b.status, BidStatus::Open | BidStatus::Closed) { (button_form(req, "bid_package.award", &here, &[("project", id.to_string()), ("id", bid.to_string()), ("vendor", x.vendor.to_string())], "Award", "small primary")) " " }
                                @if can_edit && b.status != BidStatus::Awarded { (button_form(req, "bid_package.withdraw_bid", &here, &[("project", id.to_string()), ("id", bid.to_string()), ("vendor", x.vendor.to_string())], "Withdraw", "small")) }
                            }
                        } }
                        @for x in b.bids.iter().filter(|x| x.withdrawn) { tr { td { span class="muted" { (vname(x.vendor)) } } td class="num muted" { (x.amount) } td colspan="4" { span class="pill" { "Withdrawn" } } } }
                    }
                    @if let Some((l, a, h)) = spread(b) { tfoot { tr { td { "Low · average · high" } td class="num" { (l) " · " (a) " · " (h) } td colspan="5" {} } } }
                } }
            }
            @if let Some(c) = b.commitment { p { "A draft commitment was started: " a href=(format!("/projects/{id}/commitments/{c}")) { "open it" } "." } }
        }
        div class="card" {
            h2 { "Invited" }
            @if b.invited.is_empty() { p class="muted" { "No companies invited yet. Edit the package to invite some." } } @else {
                ul class="plain" { @for v in &b.invited { li { (vname(*v)) " " @if b.bids.iter().any(|x| x.vendor == *v && !x.withdrawn) { span class="pill green" { "Bid in" } } @else { span class="pill" { "No bid yet" } } } } }
            }
        }
        @if can_edit && matches!(b.status, BidStatus::Open | BidStatus::Closed) {
            div class="card" {
                h2 { "Record a bid" }
                p class="muted small" { "Enter what the company sent. A new bid from the same company replaces its earlier one." }
                (command_form(req, "bid_package.record_bid", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(bid);
                    div class="form-grid" {
                        (field("Company", "vendor", true, false, html! { select id="vendor" name="vendor" required { @for v in &all { option value=(v.id) { (v.name) } } } }))
                        (field("Bid amount", "amount", true, false, html! { input type="text" id="amount" name="amount" required inputmode="decimal" placeholder="0.00"; }))
                        (field("Received", "received", false, false, html! { input type="date" id="received" name="received" value=(req.today().to_string()); }))
                        (field("Notes (inclusions, exclusions, alternates)", "notes", false, true, html! { textarea id="notes" name="notes" class="short" {} }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Record bid" } }
                }))
            }
        }
        @if admin && matches!(b.status, BidStatus::Open | BidStatus::Closed) && !standing.is_empty() && b.cost_code.is_some() {
            div class="card" {
                h2 { "Award and start a commitment" }
                p class="muted small" { "Awards the package to a company and starts a draft commitment for its bid amount against " (b.cost_code.and_then(|c| state.cost_codes.get(c)).map(|c| c.code.clone()).unwrap_or_default()) "." }
                (command_form(req, "bid_package.award", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(bid);
                    input type="hidden" name="commitment" value="true";
                    div class="form-grid" { (field("Award to", "award_vendor", true, false, html! { select id="award_vendor" name="vendor" required { @for x in &standing { option value=(x.vendor) { (vname(x.vendor)) " — " (x.amount) } } } })) }
                    div class="form-actions" { button type="submit" class="btn primary" { "Award and start commitment" } }
                }))
            }
        }
    };
    layout::app(
        req,
        &b.label(),
        Nav::Bidding(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Bidding", Some(base)), (&b.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<BidPackage>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.bid_packages.get(c).filter(|b| b.project == id && b.deleted.is_none() && b.status != BidStatus::Awarded) {
            Some(b) => Some(b),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "bid_package.update" } else { "bid_package.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/bidding");
    let back = cur.map(|b| format!("{base}/{}/edit", b.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|b| format!("{base}/{}", b.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let all = vendors(req);
    let codes = view::project_cost_codes(state, id);
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit bid package" } else { "New bid package" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(b) = cur { input type="hidden" name="id" value=(b.id); }
                div class="form-grid" {
                    (field("Title", "title", true, false, html! { input type="text" id="title" name="title" required value=(cur.map(|b| b.title.as_str()).unwrap_or_default()); }))
                    (field("Bids due", "due", false, false, html! { input type="date" id="due" name="due" value=(cur.and_then(|b| b.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Cost code (to compare with the budget)", "cost_code", false, false, html! { select id="cost_code" name="cost_code" {
                        option value="" { "—" }
                        @for c in &codes { option value=(c.id) selected[cur.is_some_and(|b| b.cost_code == Some(c.id))] { (c.code) " " (c.description) } }
                    } }))
                    (field("Scope of work", "scope", false, true, html! { textarea id="scope" name="scope" { (cur.map(|b| b.scope.as_str()).unwrap_or_default()) } }))
                    div class="field full" {
                        div class="label" { "Invite" }
                        input type="hidden" name="invite[]" value="";
                        details class="multi" {
                            summary { "Choose companies" }
                            div class="options" { @for v in &all { label { input type="checkbox" name="invite[]" value=(v.id) checked[cur.is_some_and(|b| b.invited.contains(&v.id))]; (v.name) @if !v.trade.is_empty() { " (" (v.trade) ")" } } } }
                        }
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|b| format!("{base}/{}", b.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Bid package",
        Nav::Bidding(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Bidding", Some(base)), ("Bid package", None)],
        body,
    )
}
