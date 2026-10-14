//! The bidder portal: the pages a company sees through the link it was emailed. No navigation, no account; the
//! page shows only what `bid_package.portal_view` returned for that link.

use buildkiosk_core::Money;
use maud::{DOCTYPE, Markup, html};
use serde_json::Value;

use crate::icons::Icon;
use crate::layout::{Flash, field, head, language_links};
use crate::pages::human_size;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn money(v: Option<&Value>) -> Option<Money> {
    v.and_then(Value::as_str).and_then(|s| Money::parse(s).ok())
}

fn shell(title: &str, flash: Option<&Flash>, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head { (head(title)) meta name="robots" content="noindex, nofollow"; }
            body {
                main class="portal" {
                    div class="brand" { (Icon::Logo.svg()) "BuildKiosk" }
                    @if let Some(f) = flash { div class=(if f.ok { "flash ok" } else { "flash error" }) role="alert" { (f.msg) } }
                    (body)
                    (language_links())
                }
            }
        }
    }
}

/// The answer to a link that doesn't work (wrong, cancelled, replaced or past its last day): the same page every time.
pub fn bad_link() -> Markup {
    shell(
        "Link not valid",
        None,
        html! {
            div class="card" {
                h1 { "This link doesn't work" }
                p { "The link may have been replaced by a newer one, cancelled, or passed its last day. Ask the person who invited you to send it again." }
            }
        },
    )
}

/// The bidder's page. `view` is the result of `bid_package.portal_view`; `base` is the link's path (`/bid/<link>`).
pub fn page(view: &Value, base: &str, csrf: &str, flash: Option<&Flash>) -> Markup {
    let package = view.get("package").cloned().unwrap_or(Value::Null);
    let title = format!("Bid package #{}: {}", package.get("number").and_then(Value::as_u64).unwrap_or(0), text(&package, "title"));
    let open = view.get("taking_bids").and_then(Value::as_bool).unwrap_or(false);
    let list = |k: &str| view.get(k).and_then(Value::as_array).cloned().unwrap_or_default();
    let (form_items, alternates, scope_items, documents, addenda) =
        (list("form_items"), list("alternates"), list("scope_items"), list("documents"), list("addenda"));
    let bid = view.get("bid").filter(|b| !b.is_null());
    let bid_lines: Vec<(u64, Option<Money>)> = bid
        .and_then(|b| b.get("lines"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|l| (l.get("item").and_then(Value::as_u64).unwrap_or(u64::MAX), money(l.get("unit_price")))).collect())
        .unwrap_or_default();
    let bid_alts: Vec<(u64, Option<Money>)> = bid
        .and_then(|b| b.get("alternates"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|l| (l.get("index").and_then(Value::as_u64).unwrap_or(u64::MAX), money(l.get("amount")))).collect())
        .unwrap_or_default();
    let bid_scope = |i: usize| -> (String, String) {
        bid.and_then(|b| b.get("scope"))
            .and_then(Value::as_array)
            .and_then(|a| a.iter().find(|s| s.get("item").and_then(Value::as_u64) == u64::try_from(i).ok()))
            .map(|s| (text(s, "answer").to_string(), money(s.get("adjustment")).map(|m| m.plain()).unwrap_or_default()))
            .unwrap_or_default()
    };
    let status = text(&package, "status");
    let due = package.get("due").and_then(Value::as_str);
    shell(
        &title,
        flash,
        html! {
            div class="card" {
                p class="muted" { (text(view, "company")) }
                h1 { (title) }
                @if let Some(d) = due { p { strong { "Bids are due " (d) "." } } }
                @if !open {
                    div class="flash info" {
                        @if status == "open" { "The due date for this package has passed, so it is no longer taking bids. Ask the team if you need more time." }
                        @else if status == "draft" { "This package is not out for bid yet." }
                        @else { "This package is not taking bids." }
                    }
                }
                @if !text(&package, "scope").is_empty() { p class="prose" { (text(&package, "scope")) } }
            }
            @if !documents.is_empty() {
                div class="card" {
                    h2 { "Bid documents" }
                    ul class="plain" { @for f in &documents { li {
                        a href=(format!("{base}/files/{}", text(f, "sha256"))) { (text(f, "name")) } " "
                        span class="muted small" { (human_size(f.get("size").and_then(Value::as_u64).unwrap_or(0))) }
                    } } }
                }
            }
            @if !addenda.is_empty() {
                div class="card" {
                    h2 { "Addenda" }
                    @for a in &addenda {
                        div class="response" {
                            p { strong { "Addendum " (a.get("number").and_then(Value::as_u64).unwrap_or(0)) ": " (text(a, "title")) } }
                            @if !text(a, "body").is_empty() { p class="prose" { (text(a, "body")) } }
                            @if a.get("acknowledged").and_then(Value::as_bool).unwrap_or(false) { span class="pill green" { "You have read this" } }
                            @else if open {
                                form method="post" action=(format!("{base}/ack")) {
                                    input type="hidden" name="_csrf" value=(csrf);
                                    input type="hidden" name="number" value=(a.get("number").and_then(Value::as_u64).unwrap_or(0));
                                    button type="submit" class="btn small" { "I have read this addendum" }
                                }
                            }
                        }
                    }
                }
            }
            @let asked = list("questions");
            @let published = list("published_answers");
            @if open || !asked.is_empty() || !published.is_empty() {
                div class="card" {
                    h2 { "Questions" }
                    @for q in &published {
                        div class="response" {
                            p { strong { "Question " (q.get("number").and_then(Value::as_u64).unwrap_or(0)) } ": " (text(q, "text")) }
                            p class="prose" { strong { "Answer: " } (text(q, "answer")) }
                        }
                    }
                    @for q in &asked {
                        div class="response" {
                            p { strong { "Your question" } ": " (text(q, "text")) }
                            @if q.get("answered").and_then(Value::as_bool).unwrap_or(false) { p class="prose" { strong { "Answer: " } (text(q, "answer")) } }
                            @else { p class="muted small" { "Not answered yet." } }
                        }
                    }
                    @if open {
                        form method="post" action=(format!("{base}/ask")) class="stack" {
                            input type="hidden" name="_csrf" value=(csrf);
                            (field("Ask the team a question", "question", false, true, html! { textarea id="question" name="question" class="short" required {} }))
                            p class="muted small" { "The team answers you here. They may share a question and its answer with the other bidders, without your name." }
                            button type="submit" class="btn small" { "Send question" }
                        }
                    }
                }
            }
            @if let Some(b) = bid {
                div class="card" {
                    h2 { "Your bid" }
                    p { strong { (money(b.get("amount")).map(|m| m.plain()).unwrap_or_default()) } " · received " (text(b, "received")) }
                    @if !text(b, "notes").is_empty() { p class="prose" { (text(b, "notes")) } }
                    p class="muted small" { "You can change your bid below until the due date." }
                    @if open {
                        form method="post" action=(format!("{base}/withdraw")) {
                            input type="hidden" name="_csrf" value=(csrf);
                            button type="submit" class="btn small danger" { "Withdraw my bid" }
                        }
                    }
                }
            }
            @if open {
                div class="card" {
                    h2 { @if bid.is_some() { "Change your bid" } @else { "Send your bid" } }
                    @if let Some(p) = view.get("bond_required").and_then(Value::as_str) { p class="muted small" { "A bid bond of " (p) "% of the bid is required. Send it to the team separately; they record when it arrives." } }
                    form method="post" action=(format!("{base}/submit")) class="stack" {
                        input type="hidden" name="_csrf" value=(csrf);
                        @if form_items.is_empty() {
                            (field("Bid amount", "amount", true, false, html! {
                                input type="text" id="amount" name="amount" required inputmode="decimal" placeholder="0.00"
                                    value=(bid.and_then(|b| money(b.get("amount"))).map(|m| m.plain()).unwrap_or_default());
                            }))
                        } @else {
                            p class="muted small" { "Price every line. Your bid is the total." }
                            table class="data" {
                                thead { tr { th { "Line" } th class="num" { "Quantity" } th class="num" { "Unit price" } } }
                                tbody { @for (i, it) in form_items.iter().enumerate() { tr {
                                    td { (text(it, "description")) }
                                    td class="num" { (it.get("quantity_milli").and_then(Value::as_i64).unwrap_or(0) / 1000) " " (text(it, "unit")) }
                                    td class="num" { input type="text" inputmode="decimal" name=(format!("line_{i}")) required aria-label=(format!("Unit price for line {}", i + 1))
                                        value=(bid_lines.iter().find(|(n, _)| *n == i as u64).and_then(|(_, m)| *m).map(|m| m.plain()).unwrap_or_default()); }
                                } } }
                            }
                        }
                        @if !alternates.is_empty() {
                            h3 { "Alternates" }
                            p class="muted small" { "Enter an amount to add (or a negative number to deduct), or leave blank to not price it." }
                            @for (i, a) in alternates.iter().enumerate() {
                                (field(a.as_str().unwrap_or(""), &format!("alt_{i}"), false, false, html! {
                                    input type="text" id=(format!("alt_{i}")) name=(format!("alt_{i}")) inputmode="decimal"
                                        value=(bid_alts.iter().find(|(n, _)| *n == i as u64).and_then(|(_, m)| *m).map(|m| m.plain()).unwrap_or_default());
                                }))
                            }
                        }
                        @if !scope_items.is_empty() {
                            h3 { "What your bid covers" }
                            @for (i, s) in scope_items.iter().enumerate() {
                                @let (answer, adj) = bid_scope(i);
                                div class="scope-row" {
                                    span { (s.as_str().unwrap_or("")) }
                                    @for (v, l) in [("included", "Included"), ("excluded", "Excluded"), ("unclear", "Not clear")] {
                                        label { input type="radio" name=(format!("scope_{i}")) value=(v) checked[answer == v]; " " (l) }
                                    }
                                    label { "Cost to add if excluded " input type="text" inputmode="decimal" name=(format!("adj_{i}")) value=(adj) size="10"; }
                                }
                            }
                        }
                        (field("Notes (inclusions, exclusions, conditions)", "notes", false, true, html! {
                            textarea id="notes" name="notes" class="short" { (bid.map(|b| text(b, "notes")).unwrap_or("")) }
                        }))
                        button type="submit" class="btn primary" { "Send bid" }
                    }
                }
            }
        },
    )
}
