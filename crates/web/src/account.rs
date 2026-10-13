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

/// What the account page needs to know about email.
pub struct MailPref {
    /// The company has set up outgoing email.
    pub configured: bool,
    /// This person receives notification emails.
    pub on: bool,
}

/// Outgoing email settings as the admin page shows them (the password is never sent back).
#[derive(Default)]
pub struct MailForm {
    pub host: String,
    pub port: String,
    pub security: String,
    pub username: String,
    pub has_password: bool,
    pub from: String,
    pub base_url: String,
    pub configured: bool,
}

/// The company admin's page for outgoing email.
pub fn mail_page(req: &Req<'_>, m: &MailForm) -> Markup {
    let sel = |v: &str| m.security == v || (m.security.is_empty() && v == "starttls");
    let body = html! {
        div class="page-head" { div { h1 { "Email" } div class="sub" { "Send notifications to people's inboxes" } } }
        div class="card" {
            p class="muted" { "BuildKiosk sends each notification as an email through your own mail server (SMTP). People can turn it off on their account page. Nothing is sent until this is filled in." }
            form method="post" action="/admin/email" class="stack" {
                input type="hidden" name="_csrf" value=(req.csrf);
                div class="form-grid" {
                    (field("Mail server", "host", true, false, html! { input type="text" id="host" name="host" value=(m.host) required placeholder="smtp.example.com"; }))
                    (field("Port", "port", true, false, html! { input type="text" id="port" name="port" value=(m.port) required placeholder="587"; }))
                    (field("Encryption", "security", false, false, html! { select id="security" name="security" {
                        option value="starttls" selected[sel("starttls")] { "STARTTLS (usual, port 587)" }
                        option value="tls" selected[sel("tls")] { "TLS (port 465)" }
                        option value="none" selected[sel("none")] { "None (trusted local relay only)" }
                    } }))
                    (field("User name", "username", false, false, html! { input type="text" id="username" name="username" value=(m.username) autocomplete="off"; }))
                    (field("Password", "password", false, false, html! { input type="password" id="password" name="password" autocomplete="new-password" placeholder=(if m.has_password { "(unchanged)" } else { "" }); }))
                    (field("Send from", "from", true, false, html! { input type="text" id="from" name="from" value=(m.from) required placeholder="BuildKiosk <noreply@example.com>"; }))
                    (field("Address of this server", "base_url", false, false, html! { input type="text" id="base_url" name="base_url" value=(m.base_url) placeholder="https://buildkiosk.example.com"; }))
                }
                p class="muted small" { "The address of this server is used for the links in messages." }
                div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
            }
        }
        @if m.configured {
            div class="card" {
                h2 { "Test it" }
                form method="post" action="/admin/email/test" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    p class="muted" { "Sends a short message to your own address." }
                    div class="form-actions" { button type="submit" class="btn" { "Send a test email" } }
                }
                form method="post" action="/admin/email/clear" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-actions" { button type="submit" class="btn danger" { "Turn email off" } }
                }
            }
        }
    };
    layout::app(req, "Email", Nav::Account, &[("Email", None)], body)
}

/// `new_token` is shown once, right after it is made.
pub fn page(req: &Req<'_>, tokens: &[TokenRow], new_token: Option<&str>, mail: &MailPref) -> Markup {
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
        @if mail.configured {
            div class="card" {
                h2 { "Email" }
                form method="post" action="/account/email" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    label class="check" { input type="checkbox" name="on" value="1" checked[mail.on]; " Email me my notifications" }
                    div class="form-actions" { button type="submit" class="btn" { "Save" } }
                }
            }
        }
        @if me.is_some_and(|p| p.company_admin) {
            p class="muted" { a href="/settings" { "Company settings" } " · " a href="/admin/email" { "Set up outgoing email" } " · " a href="/admin/backups" { "Backups" } }
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

/// One finished backup in the backup folder.
pub struct BackupRow {
    pub name: String,
    /// When it was taken, in unix seconds.
    pub taken: i64,
    pub bytes: u64,
    /// Whether it is also in the second folder.
    pub copied: bool,
}

/// What the backups page shows: whether scheduled backups are on, where they go and what is there.
#[derive(Default)]
pub struct BackupInfo {
    pub configured: bool,
    pub dir: String,
    pub copy_to: Option<String>,
    pub every_hours: u64,
    pub keep: usize,
    pub rows: Vec<BackupRow>,
}

/// Backups taken while the server runs: the list, and a button to take one now.
pub fn backups_page(req: &Req<'_>, info: &BackupInfo) -> Markup {
    let body = html! {
        div class="page-head" { div { h1 { "Backups" } div class="sub" { "Copies of all the company data, each checked by loading it" } } }
        @if !info.configured {
            div class="card" {
                p { "Scheduled backups are not turned on. Start the server with " code { "--backup-dir /path/to/backups" } " to take one every day and keep the last seven. See the deployment notes for restoring one." }
            }
        } @else {
            div class="card" {
                dl class="facts" {
                    dt { "Folder" } dd { code { (info.dir) } }
                    dt { "Schedule" } dd { "Every " (info.every_hours) " hours, keeping the newest " (info.keep) }
                    dt { "Second copy" } dd { @if let Some(c) = &info.copy_to { code { (c) } } @else { span class="muted" { "None. Add --backup-copy-to FOLDER to copy each backup to another disk or a mounted share." } } }
                }
                form method="post" action="/admin/backups/run" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-actions" { button type="submit" class="btn primary" { "Back up now" } }
                }
                p class="muted small" { "Making a backup takes a moment and holds other requests up only while the small files are copied." }
            }
            div class="card" {
                h2 { "Backups kept" }
                @if info.rows.is_empty() { p class="muted" { "None yet." } } @else {
                    div class="table-wrap" { table class="data" {
                        thead { tr { th { "Backup" } th { "Taken" } th class="num" { "Size" } th { "Second copy" } } }
                        tbody { @for r in &info.rows { tr {
                            td { code { (r.name) } }
                            td class="nowrap" { (req.when(buildkiosk_core::Timestamp::from_millis(r.taken.saturating_mul(1000)))) }
                            td class="num" { (crate::pages::human_size(r.bytes)) }
                            td { @if info.copy_to.is_none() { span class="muted" { "—" } } @else if r.copied { span class="pill green" { "Copied" } } @else { span class="pill amber" { "Not copied" } } }
                        } } }
                    } }
                }
            }
        }
    };
    layout::app(req, "Backups", Nav::Account, &[("My account", Some("/account".into())), ("Backups", None)], body)
}
