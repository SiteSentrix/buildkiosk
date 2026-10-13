//! Bidding pages: packages, bid leveling and the form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::bidding::{spread, visible};
use buildkiosk_engine::model::bidding::{BidPackage, BidStatus, ScopeAnswer};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{human_size, not_found, project_for};

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
    let leveled = standing.iter().any(|x| !x.scope.is_empty());
    let body = html! {
        div class="page-head" {
            div { h1 { (b.label()) ": " (b.title) " " (status_pill(b.status)) } div class="sub" { @if let Some(d) = b.due { "Bids due " (d.pretty()) " · " } @if let Some(c) = b.cost_code.and_then(|c| state.cost_codes.get(c)) { "Cost code " (c.code) " " (c.description) } } }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                @if can_edit && b.status != BidStatus::Awarded { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if can_edit && b.status == BidStatus::Draft { (button_form(req, "bid_package.open", &here, &ids, "Send out for bid", "primary")) }
                @if can_edit && b.status == BidStatus::Open { (button_form(req, "bid_package.close", &here, &ids, "Close bidding", "")) }
                @if can_edit && b.status == BidStatus::Closed { (button_form(req, "bid_package.reopen", &here, &ids, "Reopen bidding", "")) }
                @if admin { (button_form(req, "bid_package.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if !b.scope.is_empty() { div class="card" { h2 { "Scope" } p class="prose" { (b.scope) } } }
        @if b.bond_required.is_some() || !b.addenda.is_empty() {
            div class="card" { dl class="facts" {
                @if let Some(x) = b.bond_required { dt { "Bid bond" } dd { (x) "% of the bid, required with each bid" } }
                @if !b.addenda.is_empty() { dt { "Addenda" } dd { (b.addenda.len()) " issued" } }
            } }
        }
        div class="card" {
            h2 { "Bids" }
            @if let Some(x) = budget { p class="muted" { "Revised budget for this cost code: " strong { (x) } } }
            @if b.bids.is_empty() { p class="muted" { "No bids yet." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Company" } th class="num" { "Bid" } @if leveled { th class="num" { "Leveled" } } th class="num" { "Over low bid" } @if budget.is_some() { th class="num" { "Against budget" } } th { "Received" } th { "Notes" } th {} } }
                    tbody {
                        @for x in &standing { tr class=(if b.awarded_to == Some(x.vendor) { "awarded" } else { "" }) {
                            td { strong { (vname(x.vendor)) } @if b.awarded_to == Some(x.vendor) { " " span class="pill green" { "Awarded" } } @if Some(x.amount) == low { " " span class="pill blue" { "Low" } } }
                            td class="num" { (x.amount) }
                            @if leveled { td class="num" { @if let Some(l) = x.leveled() { (l) @if !x.adjustments().unwrap_or_default().is_zero() { " " span class="muted small" { "(+" (x.adjustments().unwrap_or_default()) ")" } } } } }
                            td class="num" { @if let (Some(l), true) = (low, Some(x.amount) != low) { (x.amount.checked_sub(l).unwrap_or_default()) } }
                            @if let Some(bg) = budget { td class="num" { (x.amount.checked_sub(bg).map(|d| html! { span class=(if d.is_negative() { "" } else { "overdue" }) { (d) } }).unwrap_or_default()) } }
                            td class="nowrap" { (x.received.to_string()) }
                            td {
                                (x.notes)
                                @if b.bond_required.is_some() { div { @if x.bond.received { span class="pill green" { "Bond in" } } @else { span class="pill amber" { "No bond" } } } }
                                @let missing = b.unacknowledged(x.vendor);
                                @if !missing.is_empty() { div { span class="pill amber" { "Addendum " (missing.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")) " not acknowledged" } } }
                                @for f in &x.files { div { a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) } @if can_edit && b.status != BidStatus::Awarded { " " (button_form(req, "bid_package.detach", &here, &[("project", id.to_string()), ("id", bid.to_string()), ("vendor", x.vendor.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) } } }
                                @if can_edit && b.status != BidStatus::Awarded {
                                    form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                                        input type="hidden" name="_csrf" value=(req.csrf); input type="hidden" name="vendor" value=(x.vendor);
                                        input type="file" name="file" required aria-label=(format!("A file from {}", vname(x.vendor)));
                                        button type="submit" class="btn small" { "Attach to this bid" }
                                    }
                                }
                            }
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
        @if !standing.is_empty() && (!b.scope_items.is_empty() || !b.form_items.is_empty() || !b.alternates.is_empty()) {
            div class="card" {
                h2 { "Leveling" }
                p class="muted small" { "What each bid covers, side by side. An adjustment is what it would cost to bring a bid up to the full scope; the leveled amount adds it." }
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "" } @for x in &standing { th class="num" { (vname(x.vendor)) } } } }
                    tbody {
                        @if !b.scope_items.is_empty() {
                            tr class="group-row" { th colspan=(standing.len() + 1) { "Scope" } }
                            @for (i, item) in b.scope_items.iter().enumerate() { tr {
                                td { (item) }
                                @for x in &standing { @let r = x.scope.iter().find(|s| s.item as usize == i); td class="num" {
                                    @match r.map(|r| r.answer) {
                                        Some(ScopeAnswer::Included) => { span class="pill green" { "Included" } }
                                        Some(ScopeAnswer::Excluded) => { span class="pill red" { "Excluded" } @if let Some(a) = r.and_then(|r| r.adjustment) { " +" (a) } }
                                        Some(ScopeAnswer::Unclear) => { span class="pill amber" { "Not clear" } @if let Some(a) = r.and_then(|r| r.adjustment) { " +" (a) } }
                                        None => { span class="muted" { "—" } }
                                    }
                                } }
                            } }
                        }
                        @if !b.form_items.is_empty() {
                            tr class="group-row" { th colspan=(standing.len() + 1) { "Unit prices" } }
                            @for (i, item) in b.form_items.iter().enumerate() { tr {
                                td { (item.description) " " span class="muted small" { (item.quantity_milli / 1000) @if !item.unit.is_empty() { " " (item.unit) } } }
                                @for x in &standing { td class="num" { @if let Some(l) = x.lines.iter().find(|l| l.item as usize == i) { (l.unit_price) } @else { span class="muted" { "—" } } } }
                            } }
                        }
                        @if !b.alternates.is_empty() {
                            tr class="group-row" { th colspan=(standing.len() + 1) { "Alternates" } }
                            @for (i, name) in b.alternates.iter().enumerate() { tr {
                                td { (name) }
                                @for x in &standing { td class="num" { @if let Some(a) = x.alternates.iter().find(|a| a.index as usize == i) { (a.amount) } @else { span class="muted" { "—" } } } }
                            } }
                        }
                    }
                    tfoot { tr { td { "Bid · leveled" } @for x in &standing { td class="num" { (x.amount) " · " strong { (x.leveled().unwrap_or(x.amount)) } } } } }
                } }
            }
        }
        @if !b.criteria.is_empty() || (can_edit && b.status != BidStatus::Awarded) {
            div class="card" {
                h2 { "Scoring" }
                @if b.criteria.is_empty() {
                    p class="muted" { "Bids are compared on price alone. Add criteria to weigh other things, such as safety record or schedule." }
                } @else {
                    @let ranked = { let mut v: Vec<(&buildkiosk_engine::model::bidding::Bid, Option<u32>)> = standing.iter().map(|x| (*x, b.weighted(x))).collect(); v.sort_by_key(|(_, t)| std::cmp::Reverse(t.unwrap_or(0))); v };
                    div class="table-wrap" { table class="data" {
                        thead { tr { th { "Company" } @for c in &b.criteria { th class="num" { (c.name) " " span class="muted small" { (c.weight) "%" } } } th class="num" { "Total (of 10)" } } }
                        tbody { @for (x, total) in &ranked { tr {
                            td { strong { (vname(x.vendor)) } }
                            @for (i, c) in b.criteria.iter().enumerate() { td class="num" {
                                @if c.price {
                                    @let low_c = standing.iter().map(|y| y.leveled().unwrap_or(y.amount).cents()).filter(|v| *v > 0).min().unwrap_or(0);
                                    @let mine = x.leveled().unwrap_or(x.amount).cents();
                                    @if mine > 0 { (format!("{:.2}", (1000 * low_c / mine) as f64 / 100.0)) } @else { "—" }
                                } @else { @if let Some(sc) = x.scores.iter().find(|s| s.criterion as usize == i) { (sc.score) } @else { span class="muted" { "—" } } }
                            } }
                            td class="num" { @if let Some(t) = total { strong { (format!("{:.2}", f64::from(*t) / 100.0)) } } @else { span class="muted" { "not complete" } } }
                        } } }
                    } }
                }
                @if can_edit && b.status != BidStatus::Awarded {
                    @for x in &standing { @if !b.criteria.is_empty() {
                        details class="disclosure" {
                            summary { "Score " (vname(x.vendor)) }
                            (command_form(req, "bid_package.score", &here, &here, html! {
                                input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(bid); input type="hidden" name="vendor" value=(x.vendor);
                                div class="form-grid" {
                                    @for (i, c) in b.criteria.iter().enumerate() { @if !c.price {
                                        (field(&format!("{} (0 to 10)", c.name), &format!("sc_{}_{i}", x.vendor), false, false, html! { input type="number" min="0" max="10" id=(format!("sc_{}_{i}", x.vendor)) name=(format!("scores.{}", i + 1)) value=(x.scores.iter().find(|s| s.criterion as usize == i).map(|s| s.score.to_string()).unwrap_or_default()); }))
                                    } }
                                }
                                div class="form-actions" { button type="submit" class="btn small primary" { "Save scores" } }
                            }))
                        }
                    } }
                    details class="disclosure" {
                        summary { "Set the criteria" }
                        (command_form(req, "bid_package.set_criteria", &here, &here, html! {
                            input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(bid);
                            (field("One per line: name, weight (adding up to 100), then price for the one worked out from the bids", "crit_text", false, true, html! { textarea id="crit_text" name="text" rows="5" placeholder="Price, 50, price\nSafety record, 30\nSchedule, 20" { (b.criteria.iter().map(|c| format!("{}, {}{}\n", c.name, c.weight, if c.price { ", price" } else { "" })).collect::<String>()) } }))
                            p class="muted small" { "Changing the criteria clears the scores already given." }
                            div class="form-actions" { button type="submit" class="btn small" { "Save criteria" } }
                        }))
                    }
                }
            }
        }
        div class="card" {
            h2 { "Addenda" }
            @if b.addenda.is_empty() { p class="muted" { "None issued." } } @else {
                @for a in &b.addenda {
                    div class="response" {
                        div class="meta" { span { strong { "Addendum " (a.number) ": " (a.title) } " · " (req.when(a.issued.at)) } }
                        @if !a.body.is_empty() { p class="prose" { (a.body) } }
                        div class="small" {
                            @for v in &b.invited {
                                @if b.acknowledgements.iter().any(|k| k.vendor == *v && k.number == a.number) { span class="pill green" { (vname(*v)) " ✓" } " " }
                                @else if can_edit && b.status != BidStatus::Awarded { (button_form(req, "bid_package.acknowledge", &here, &[("project", id.to_string()), ("id", bid.to_string()), ("vendor", v.to_string()), ("number", a.number.to_string())], &format!("{} acknowledged", vname(*v)), "small")) " " }
                                @else { span class="pill amber" { (vname(*v)) " not yet" } " " }
                            }
                        }
                    }
                }
            }
            @if can_edit && b.status != BidStatus::Awarded {
                (command_form(req, "bid_package.addendum", &here, &here, html! {
                    input type="hidden" name="project" value=(id); input type="hidden" name="id" value=(bid);
                    div class="form-grid" {
                        (field("Title", "add_title", true, false, html! { input type="text" id="add_title" name="title" required; }))
                        (field("What changed", "add_body", false, true, html! { textarea id="add_body" name="body" class="short" {} }))
                    }
                    div class="form-actions" { button type="submit" class="btn" { "Issue addendum" } }
                }))
            }
        }
        div class="card" {
            h2 { "Bid documents" }
            @if b.attachments.is_empty() { p class="muted" { "No files yet. Attach the scope, drawing lists and the form of proposal." } } @else {
                ul class="plain" { @for f in &b.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if can_edit && b.status != BidStatus::Awarded { " " (button_form(req, "bid_package.detach", &here, &[("project", id.to_string()), ("id", bid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if can_edit && b.status != BidStatus::Awarded {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
        div class="card" {
            h2 { "Invited" }
            @if b.invited.is_empty() { p class="muted" { "No companies invited yet. Edit the package to invite some." } } @else {
                @let see_prequal = buildkiosk_engine::model::perm::company_level(req.engine.state(), req.person, Tool::Prequalification) >= Level::Read;
                @let today = req.today();
                ul class="plain" { @for v in &b.invited { li { (vname(*v)) " " @if see_prequal { a href=(format!("/prequalification/{v}")) { (crate::prequal::standing_pill(buildkiosk_engine::cmd::prequal::vendor_standing(req.engine.state(), *v, today))) } " " } @if b.bids.iter().any(|x| x.vendor == *v && !x.withdrawn) { span class="pill green" { "Bid in" } } @else { span class="pill" { "No bid yet" } } } } }
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
                        (field(if b.form_items.is_empty() { "Bid amount" } else { "Bid amount (or price the lines below)" }, "amount", b.form_items.is_empty(), false, html! { input type="text" id="amount" name="amount" required[b.form_items.is_empty()] inputmode="decimal" placeholder="0.00"; }))
                        (field("Received", "received", false, false, html! { input type="date" id="received" name="received" value=(req.today().to_string()); }))
                        (field("Notes (inclusions, exclusions, alternates)", "notes", false, true, html! { textarea id="notes" name="notes" class="short" {} }))
                    }
                    @if !b.form_items.is_empty() {
                        h3 { "Unit prices" }
                        table class="data" {
                            thead { tr { th { "Line" } th class="num" { "Quantity" } th class="num" { "Unit price" } } }
                            tbody { @for (i, item) in b.form_items.iter().enumerate() { tr {
                                td { (item.description) } td class="num" { (item.quantity_milli / 1000) " " (item.unit) }
                                td class="num" { input type="hidden" name=(format!("lines.{i}.item")) value=(i); input type="text" inputmode="decimal" name=(format!("lines.{i}.unit_price")) aria-label=(format!("Unit price of {}", item.description)); }
                            } } }
                        }
                    }
                    @if !b.scope_items.is_empty() {
                        h3 { "What the bid covers" }
                        table class="data" {
                            thead { tr { th { "Scope item" } th { "Covered?" } th class="num" { "Add to level it" } } }
                            tbody { @for (i, item) in b.scope_items.iter().enumerate() { tr {
                                td { (item) }
                                td { input type="hidden" name=(format!("scope.{i}.item")) value=(i); select name=(format!("scope.{i}.answer")) aria-label=(format!("Does the bid cover {item}")) { @for a in ScopeAnswer::ALL { option value=(a.key()) { (a.label()) } } } }
                                td class="num" { input type="text" inputmode="decimal" name=(format!("scope.{i}.adjustment")) aria-label=(format!("Amount to add for {item}")); }
                            } } }
                        }
                    }
                    @if !b.alternates.is_empty() {
                        h3 { "Alternates" }
                        table class="data" {
                            thead { tr { th { "Alternate" } th class="num" { "Amount (negative for a deduct)" } } }
                            tbody { @for (i, name) in b.alternates.iter().enumerate() { tr {
                                td { (name) }
                                td class="num" { input type="hidden" name=(format!("alternates.{i}.index")) value=(i); input type="text" inputmode="decimal" name=(format!("alternates.{i}.amount")) aria-label=(format!("Amount for {name}")); }
                            } } }
                        }
                    }
                    @if b.bond_required.is_some() {
                        div class="form-grid" {
                            (field("Bid bond", "bond_received", false, false, html! { select id="bond_received" name="bond_received" { option value="false" { "Not received" } option value="true" { "Received" } } }))
                            (field("Bond amount", "bond_amount", false, false, html! { input type="text" id="bond_amount" name="bond_amount" inputmode="decimal"; }))
                        }
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
    let see_prequal = buildkiosk_engine::model::perm::company_level(state, req.person, Tool::Prequalification) >= Level::Read;
    let today = req.today();
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
                    @if cur.is_none_or(|b| !b.bids.iter().any(|x| !x.withdrawn)) {
                        div class="field full" {
                            details class="disclosure" {
                                summary { "Bid form, scope checklist, alternates and bond" }
                                p class="muted small" { "Optional. Bidders price the lines of the bid form; the scope checklist lets you compare bids on the same scope; alternates are options priced apart. They can't change once a bid is in." }
                                h3 { "Scope checklist" }
                                @for i in 0..cur.map(|b| b.scope_items.len()).unwrap_or(0) + 3 { input type="text" name=(format!("scope_items.{i}.text")) value=(cur.and_then(|b| b.scope_items.get(i)).map(String::as_str).unwrap_or_default()) aria-label=(format!("Scope item {}", i + 1)); }
                                h3 { "Bid form lines" }
                                table class="data" {
                                    thead { tr { th { "Description" } th { "Unit" } th class="num" { "Quantity" } } }
                                    tbody { @for i in 0..cur.map(|b| b.form_items.len()).unwrap_or(0) + 3 { @let it = cur.and_then(|b| b.form_items.get(i)); tr {
                                        td { input type="text" name=(format!("form_items.{i}.description")) value=(it.map(|x| x.description.as_str()).unwrap_or_default()) aria-label=(format!("Line {} description", i + 1)); }
                                        td { input type="text" name=(format!("form_items.{i}.unit")) value=(it.map(|x| x.unit.as_str()).unwrap_or_default()) aria-label=(format!("Line {} unit", i + 1)); }
                                        td class="num" { input type="text" inputmode="decimal" name=(format!("form_items.{i}.quantity")) value=(it.map(|x| format!("{}", x.quantity_milli / 1000)).unwrap_or_default()) aria-label=(format!("Line {} quantity", i + 1)); }
                                    } } }
                                }
                                h3 { "Alternates" }
                                @for i in 0..cur.map(|b| b.alternates.len()).unwrap_or(0) + 2 { input type="text" name=(format!("alternates.{i}.name")) value=(cur.and_then(|b| b.alternates.get(i)).map(String::as_str).unwrap_or_default()) aria-label=(format!("Alternate {}", i + 1)); }
                                (field("Bid bond required (percent of the bid)", "bond_percent", false, false, html! { input type="text" id="bond_percent" name="bond_percent" inputmode="decimal" placeholder="5" value=(cur.and_then(|b| b.bond_required).map(|x| x.plain()).unwrap_or_default()); }))
                            }
                        }
                    }
                    div class="field full" {
                        div class="label" { "Invite" }
                        input type="hidden" name="invite[]" value="";
                        details class="multi" {
                            summary { "Choose companies" }
                            div class="options" { @for v in &all { label { input type="checkbox" name="invite[]" value=(v.id) checked[cur.is_some_and(|b| b.invited.contains(&v.id))]; (v.name) @if !v.trade.is_empty() { " (" (v.trade) ")" } @if see_prequal && !matches!(buildkiosk_engine::cmd::prequal::vendor_standing(state, v.id, today), buildkiosk_engine::model::prequal::Standing::Current | buildkiosk_engine::model::prequal::Standing::ExpiringSoon) { " " span class="muted small" { "(not prequalified)" } } } } }
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
