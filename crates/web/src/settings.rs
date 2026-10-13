//! Settings pages: one project's details and the company's own.

use buildkiosk_core::Id;
use buildkiosk_engine::model::Choice;
use buildkiosk_engine::model::directory::VendorKind;
use buildkiosk_engine::model::project::{Project, ProjectStage};
use maud::{Markup, html};

use crate::layout::{self, Nav, Req, command_form, field};
use crate::pages::{not_found, project_for};

/// A project's details, stage and defaults (project admins).
pub fn project(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    if !req.allowed("project.update", Some(id)) {
        return not_found(req);
    }
    let state = req.engine.state();
    let here = format!("/projects/{id}/settings");
    let mut owners: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    owners.sort_by_key(|v| (v.kind != VendorKind::Owner, v.name.to_lowercase()));
    let body = html! {
        div class="page-head" { div { h1 { "Project settings" } div class="sub" { (p.name) } } }
        div class="card" {
            (command_form(req, "project.update", &here, &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (field("Name", "name", true, false, html! { input type="text" id="name" name="name" value=(p.name) required; }))
                    (field("Job number", "number", false, false, html! { input type="text" id="number" name="number" value=(p.number); }))
                    (field("Stage", "stage", false, false, html! { select id="stage" name="stage" { @for s in ProjectStage::ALL { option value=(s.key()) selected[p.stage == *s] { (s.label()) } } } }))
                    (field("Type", "kind", false, false, html! { input type="text" id="kind" name="kind" value=(p.kind) placeholder="Commercial, healthcare…"; }))
                    (field("Owner / client", "owner", false, false, html! { select id="owner" name="owner" { option value="" { "(none)" } @for v in &owners { option value=(v.id) selected[p.owner == Some(v.id)] { (v.name) } } } }))
                    (field("Estimated value", "value", false, false, html! { input type="text" id="value" name="value" value=(p.value.map(|v| v.plain()).unwrap_or_default()); }))
                    (field("Start", "start", false, false, html! { input type="date" id="start" name="start" value=(p.start.map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Planned completion", "completion", false, false, html! { input type="date" id="completion" name="completion" value=(p.completion.map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Street", "street", false, false, html! { input type="text" id="street" name="street" value=(p.address.street); }))
                    (field("City", "city", false, false, html! { input type="text" id="city" name="city" value=(p.address.city); }))
                    (field("State or region", "region", false, false, html! { input type="text" id="region" name="region" value=(p.address.region); }))
                    (field("Postal code", "postal_code", false, false, html! { input type="text" id="postal_code" name="postal_code" value=(p.address.postal_code); }))
                    (field("Country", "country", false, false, html! { input type="text" id="country" name="country" value=(p.address.country); }))
                    (field("Days to answer an RFI", "rfi_due_days", false, false, html! { input type="number" min="1" max="90" id="rfi_due_days" name="rfi_due_days" value=(p.rfi_due_days.map(|d| d.to_string()).unwrap_or_default()) placeholder="company default"; }))
                }
                (field("Description", "description", false, true, html! { textarea id="description" name="description" { (p.description) } }))
                input type="hidden" name="active" value="false";
                label class="check" { input type="checkbox" name="active" value="true" checked[p.active]; " Active (inactive projects are hidden from the project list)" }
                div class="form-actions" { a class="btn" href=(format!("/projects/{id}")) { "Back" } button type="submit" class="btn primary" { "Save" } }
            }))
        }
    };
    layout::app(
        req,
        "Project settings",
        Nav::Home(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Settings", None)],
        body,
    )
}

/// The company's own details and defaults (company admins).
pub fn company(req: &Req<'_>) -> Markup {
    let state = req.engine.state();
    let Some(c) = state.company().filter(|_| req.allowed("company.update", None)) else { return not_found(req) };
    let hours = c.utc_offset_minutes / 60;
    let minutes = (c.utc_offset_minutes % 60).abs();
    let offset = format!("{}{:02}:{:02}", if c.utc_offset_minutes < 0 { "-" } else { "+" }, hours.abs(), minutes);
    let body = html! {
        div class="page-head" { div { h1 { "Company settings" } div class="sub" { (c.name) } } }
        div class="card" {
            (command_form(req, "company.update", "/settings", "/settings", html! {
                div class="form-grid" {
                    (field("Company name", "name", true, false, html! { input type="text" id="name" name="name" value=(c.name) required; }))
                    (field("Phone", "phone", false, false, html! { input type="text" id="phone" name="phone" value=(c.phone); }))
                    (field("Website", "website", false, false, html! { input type="text" id="website" name="website" value=(c.website); }))
                    (field("Street", "street", false, false, html! { input type="text" id="street" name="street" value=(c.address.street); }))
                    (field("City", "city", false, false, html! { input type="text" id="city" name="city" value=(c.address.city); }))
                    (field("State or region", "region", false, false, html! { input type="text" id="region" name="region" value=(c.address.region); }))
                    (field("Postal code", "postal_code", false, false, html! { input type="text" id="postal_code" name="postal_code" value=(c.address.postal_code); }))
                    (field("Country", "country", false, false, html! { input type="text" id="country" name="country" value=(c.address.country); }))
                    (field("Currency (ISO code)", "currency", false, false, html! { input type="text" id="currency" name="currency" value=(c.currency) maxlength="3"; }))
                    (field("Minutes east of UTC", "utc_offset_minutes", false, false, html! { input type="number" id="utc_offset_minutes" name="utc_offset_minutes" min="-720" max="840" step="15" value=(c.utc_offset_minutes); }))
                    (field("Days to answer an RFI", "rfi_due_days", false, false, html! { input type="number" min="1" max="90" id="rfi_due_days" name="rfi_due_days" value=(c.rfi_due_days); }))
                    (field("Days for a submittal review", "submittal_review_days", false, false, html! { input type="number" min="1" max="90" id="submittal_review_days" name="submittal_review_days" value=(c.submittal_review_days); }))
                }
                p class="muted small" { "Dates and times are shown at UTC" (offset) ". Amounts are in " (c.currency) ". Changing the currency doesn't convert existing amounts." }
                div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
            }))
        }
        div class="card" {
            h2 { "More" }
            ul class="plain" {
                li { a href="/directory" { "Directory: people, companies and permission templates" } }
                li { a href="/admin/email" { "Outgoing email" } }
            }
        }
    };
    layout::app(req, "Company settings", Nav::Directory, &[("Company settings", None)], body)
}
