//! The prequalification link: the page a company's contact sees to fill in the questionnaire, dates and files without an
//! account. It shows only what `prequal.portal_view` returned for that link, and never the team's notes or score.

use maud::{Markup, html};
use serde_json::Value;

use crate::layout::{Flash, field};
use crate::pages::human_size;
use crate::portal::shell;

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

/// The company's page. `view` is the result of `prequal.portal_view`; `base` is the link's path (`/prequal/<link>`).
pub fn page(view: &Value, base: &str, csrf: &str, flash: Option<&Flash>) -> Markup {
    let company = text(view, "company");
    let status = text(view, "status");
    let editable = view.get("editable").and_then(Value::as_bool).unwrap_or(false);
    let sections = view.get("sections").and_then(Value::as_array).cloned().unwrap_or_default();
    let files = view.get("files").and_then(Value::as_array).cloned().unwrap_or_default();
    let title = format!("Prequalification: {company}");
    shell(
        &title,
        flash,
        html! {
            div class="card" {
                h1 { "Prequalification" }
                p { strong { (company) } @if let Some(f) = view.get("form").and_then(Value::as_str) { " · " (f) } }
                @match status {
                    "submitted" => { p { "Your record was sent and is waiting for review. You can still attach files and leave a message below." } }
                    "approved" => {
                        p { "Your record is approved" @if let Some(d) = view.get("valid_until").and_then(Value::as_str) { " until " (d) } "." }
                        p class="muted small" { "To send a renewed insurance certificate or licence, attach it below and leave a message. The team updates the dates." }
                    }
                    "rejected" => { p { "Your record was not approved this time. Correct what the reviewers asked for, and send it again." } }
                    _ => { p { "Answer the questions, give your dates, attach your documents and send your record for review." } }
                }
            }
            @if editable {
                div class="card" {
                    form method="post" action=(format!("{base}/save")) class="stack" {
                        input type="hidden" name="_csrf" value=(csrf);
                        @for (si, s) in sections.iter().enumerate() {
                            h2 { (text(s, "title")) }
                            @for (qi, q) in s.get("questions").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
                                (field(text(q, "text"), &format!("reply_{si}_{qi}"), false, true, html! {
                                    textarea id=(format!("reply_{si}_{qi}")) name=(format!("reply_{si}_{qi}")) class="short" { (text(q, "answer")) }
                                }))
                            }
                        }
                        h2 { "Your details" }
                        div class="form-grid" {
                            (field("Insurance expires", "insurance_expires", false, false, html! { input type="date" id="insurance_expires" name="insurance_expires" value=(text(view, "insurance_expires")); }))
                            (field("Licence expires", "license_expires", false, false, html! { input type="date" id="license_expires" name="license_expires" value=(text(view, "license_expires")); }))
                            (field("Experience modification rate (such as 0.85)", "emr", false, false, html! { input type="text" id="emr" name="emr" inputmode="decimal" value=(text(view, "emr")); }))
                            (field("Bonding capacity", "bonding_capacity", false, false, html! { input type="text" id="bonding_capacity" name="bonding_capacity" inputmode="decimal" value=(view.get("bonding_capacity").and_then(Value::as_str).map(|m| m.replace(",", "")).unwrap_or_default()); }))
                        }
                        (field("A message for the reviewers (optional)", "message", false, true, html! { textarea id="message" name="message" class="short" { (text(view, "message")) } }))
                        div class="form-actions" {
                            button type="submit" class="btn" { "Save and come back later" }
                            button type="submit" class="btn primary" formaction=(format!("{base}/submit")) { "Send for review" }
                        }
                        p class="muted small" { "Sending needs every question answered. Once sent, your answers are locked until the reviewers return the record." }
                    }
                }
            }
            div class="card" {
                h2 { "Documents" }
                @if files.is_empty() { p class="muted" { "No documents yet. Attach your insurance certificate, financial statements and the like." } }
                @else {
                    ul class="plain" { @for f in &files { li {
                        (text(f, "name")) " " span class="muted small" { (human_size(f.get("size").and_then(Value::as_u64).unwrap_or(0))) }
                        @if f.get("removable").and_then(Value::as_bool).unwrap_or(false) { " " form method="post" action=(format!("{base}/remove")) class="inline" {
                            input type="hidden" name="_csrf" value=(csrf); input type="hidden" name="sha256" value=(text(f, "sha256"));
                            button type="submit" class="btn small danger" { "Remove" }
                        } }
                    } } }
                }
                form method="post" action=(format!("{base}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(csrf);
                    input type="file" name="file" required aria-label="Document to attach";
                    p class="muted small" { "Up to 25 MB each." }
                    button type="submit" class="btn small" { "Attach document" }
                }
            }
            div class="card" {
                h2 { "Message the reviewers" }
                form method="post" action=(format!("{base}/message")) class="stack" {
                    input type="hidden" name="_csrf" value=(csrf);
                    (field("Message", "note", true, true, html! { textarea id="note" name="message" class="short" required {} }))
                    button type="submit" class="btn small" { "Send message" }
                }
            }
        },
    )
}
