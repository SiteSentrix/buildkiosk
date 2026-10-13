//! The signed-in person's own account page: password and API tokens.

use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::layout::{self, Nav, Req, field};

/// One API token as the page lists it (never its secret).
pub struct TokenRow {
    pub id: String,
    pub name: String,
    pub created: buildkiosk_core::Timestamp,
}

/// `new_token` is shown once, right after it is made.
pub fn page(req: &Req<'_>, tokens: &[TokenRow], new_token: Option<&str>) -> Markup {
    let state = req.engine.state();
    let me = state.people.get(req.person);
    let name = view::person_name(state, req.person);
    let body = html! {
        div class="page-head" { div { h1 { "My account" } div class="sub" { (name) @if let Some(e) = me.and_then(|p| p.email.as_ref()) { " · " (e) } } } }
        div class="card" {
            h2 { "Change password" }
            form method="post" action="/account/password" class="stack" {
                input type="hidden" name="_csrf" value=(req.csrf);
                div class="form-grid" {
                    (field("Current password", "current", true, false, html! { input type="password" id="current" name="current" required autocomplete="current-password"; }))
                    (field("New password", "password", true, false, html! { input type="password" id="password" name="password" required autocomplete="new-password"; }))
                    (field("New password again", "confirm", true, false, html! { input type="password" id="confirm" name="confirm" required autocomplete="new-password"; }))
                }
                p class="muted small" { "Changing it signs you out everywhere else." }
                div class="form-actions" { button type="submit" class="btn primary" { "Change password" } }
            }
        }
        div class="card" {
            h2 { "API tokens" }
            p class="muted" { "A token lets a script or an AI agent act as you over the HTTP API (" code { "/api/v1/commands/…" } ") and MCP (" code { "/mcp" } "), with exactly your permissions. Treat it like a password." }
            @if let Some(t) = new_token {
                div class="flash ok" role="status" { "New token (copy it now; it is not shown again):" br; code { (t) } }
            }
            @if tokens.is_empty() { p class="muted" { "You have no tokens." } } @else {
                table class="data" {
                    thead { tr { th { "Name" } th { "Made" } th {} } }
                    tbody { @for t in tokens { tr {
                        td { (t.name) } td class="nowrap" { (req.when(t.created)) }
                        td class="right" {
                            form method="post" action="/account/token/revoke" {
                                input type="hidden" name="_csrf" value=(req.csrf);
                                input type="hidden" name="id" value=(t.id);
                                button type="submit" class="btn small danger" { "Revoke" }
                            }
                        }
                    } } }
                }
            }
            form method="post" action="/account/token" class="stack" {
                input type="hidden" name="_csrf" value=(req.csrf);
                div class="form-grid" { (field("Name (what it is for)", "name", true, false, html! { input type="text" id="name" name="name" required placeholder="Claude on my laptop"; })) }
                div class="form-actions" { button type="submit" class="btn" { "Create a token" } }
            }
        }
    };
    layout::app(req, "My account", Nav::Account, &[("My account", None)], body)
}
