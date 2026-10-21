//! The signing page: what the other party sees through the link they were emailed. No navigation, no account; the
//! page shows only what `contract.portal_view` returned for that link.

use buildkiosk_core::Money;
use maud::{DOCTYPE, Markup, html};
use serde_json::Value;

use buildkiosk_core::Id;
use buildkiosk_engine::model::project::Project;

use crate::icons::Icon;
use crate::layout::{self, Flash, Nav, Req, field, head, language_links};
use crate::pages::{not_found, project_for};

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

/// The signer's page. `view` is the result of `contract.portal_view`; `base` is the link's path (`/sign/<link>`).
pub fn page(view: &Value, base: &str, csrf: &str, flash: Option<&Flash>) -> Markup {
    let title = format!("{}: {}", text(view, "label"), text(view, "title"));
    let status = text(view, "status");
    let open = view.get("open").and_then(Value::as_bool).unwrap_or(false);
    let lines = view.get("lines").and_then(Value::as_array).cloned().unwrap_or_default();
    let terms = view.get("terms").and_then(Value::as_i64).unwrap_or(0);
    let signers = view.get("signers").and_then(Value::as_array).cloned().unwrap_or_default();
    let ordered = view.get("ordered").and_then(Value::as_bool).unwrap_or(false);
    let you_signed = view.get("you_signed").and_then(Value::as_bool).unwrap_or(false);
    let your_turn = view.get("your_turn").and_then(Value::as_bool).unwrap_or(true);
    let can_sign = open && status == "sent" && !lines.is_empty() && !you_signed && your_turn;
    shell(
        &title,
        flash,
        html! {
            div class="card" {
                p class="muted" { (text(view, "company")) @if !text(view, "project").is_empty() { " · " (text(view, "project")) } }
                h1 { (title) }
                @if !text(view, "party").is_empty() { p { "For " strong { (text(view, "party")) } } }
                @if !text(view, "description").is_empty() { p class="prose" { (text(view, "description")) } }
                @match status {
                    "signed" => {
                        div class="flash ok" {
                            "Signed by " strong { (text(view, "signer")) } @if !text(view, "signed_on").is_empty() { " on " (text(view, "signed_on")) } ". "
                            "Keep this page or the email as your record; the team has been told."
                        }
                    }
                    "declined" => {
                        div class="flash info" { "You declined to sign this contract." @if !text(view, "note").is_empty() { " " (text(view, "note")) } " Ask the team if you want to look at it again." }
                    }
                    "sent" => {
                        @if !open { div class="flash info" { "This contract can't be signed now. Ask the team." } }
                        @else if you_signed { div class="flash ok" { "You have signed. The contract is complete when everyone has." } }
                        @else if !your_turn { div class="flash info" { "The people before you sign first. You will get an email when it is your turn." } }
                    }
                    _ => { div class="flash info" { "This contract is not out for signature. Ask the team." } }
                }
            }
            @if signers.len() > 1 {
                div class="card" {
                    h2 { "Who signs" }
                    @if ordered { p class="muted small" { "They sign one after the other, in this order." } }
                    ul class="plain" { @for (i, who) in signers.iter().enumerate() { li {
                        (i + 1) ". "
                        @if who.get("you").and_then(Value::as_bool).unwrap_or(false) { strong { "You" } } @else if !text(who, "name").is_empty() { (text(who, "name")) } @else { "Someone else" }
                        " · "
                        @if who.get("signed").and_then(Value::as_bool).unwrap_or(false) { span class="pill green" { "Signed" } } @else { span class="pill" { "Not yet" } }
                    } } }
                }
            }
            div class="card" {
                h2 { "The contract" }
                @if lines.is_empty() { p class="muted" { "There are no lines on this contract yet." } }
                @else {
                    table class="data" {
                        thead { tr { th { "#" } th { "Cost code" } th { "Description" } th class="num" { "Amount" } } }
                        tbody { @for (n, l) in lines.iter().enumerate() { tr {
                            td { (n + 1) } td { (text(l, "cost_code")) } td { (text(l, "description")) }
                            td class="num" { (money(l.get("amount")).map(|m| m.to_string()).unwrap_or_default()) }
                        } } }
                        tfoot { tr { td colspan="3" { "Total" } td class="num" { (money(view.get("total")).map(|m| m.to_string()).unwrap_or_default()) } } }
                    }
                }
                @if let Some(r) = view.get("retainage").and_then(Value::as_str) { p class="muted small" { "Retainage: " (r) } }
            }
            @if can_sign {
                div class="card" {
                    h2 { "Sign" }
                    form method="post" action=(format!("{base}/sign")) class="stack" {
                        input type="hidden" name="_csrf" value=(csrf);
                        input type="hidden" name="terms" value=(terms);
                        (field("Type your full name", "name", true, false, html! { input type="text" id="name" name="name" required autocomplete="name" maxlength="120"; }))
                        label class="check" { input type="checkbox" name="agree" value="true" required; " I have read this contract and agree to it, and I am authorised to sign for " (if text(view, "party").is_empty() { "the other party" } else { text(view, "party") }) "." }
                        p class="muted small" { "Typing your name here is your signature. We record your name, the date and time, and the version of the contract you saw." }
                        button type="submit" class="btn primary" { "Sign" }
                    }
                }
                div class="card" {
                    details class="disclosure" {
                        summary { "I can't sign this" }
                        form method="post" action=(format!("{base}/decline")) class="stack" {
                            input type="hidden" name="_csrf" value=(csrf);
                            (field("Why (optional)", "note", false, true, html! { input type="text" id="note" name="note" maxlength="200"; }))
                            button type="submit" class="btn danger" { "Decline to sign" }
                        }
                    }
                }
            }
        },
    )
}

/// The team's page that shows one signing link so it can be copied when email isn't set up. `path` is the contract's
/// page under the project (`commitments/4`), `nav` the tool it belongs to.
pub fn link_page(req: &Req<'_>, id: Id<Project>, nav: Nav, (tool, path): (&str, &str), label: &str, email: &str, url: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let here = format!("/projects/{id}/{path}");
    layout::app(
        req,
        "Signing link",
        nav,
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), (tool, None), ("Signing link", None)],
        html! {
            div class="page-head" { div { h1 { "Signing link for " (label) } div class="sub" { "Sent to " (email) } } div class="actions" { a class="btn" href=(here) { "Back" } } }
            div class="card" {
                p { "Anyone with this link can read the contract and sign it for the other party. Send it only to them." }
                p { input type="text" readonly value=(url) size="80" aria-label="Signing link"; }
                @if !url.starts_with("http") { p class="muted small" { "Add the server's public address under Email settings to get a full link; until then, put your site's address in front of the path above." } }
            }
        },
    )
}
