//! Transmittal pages: list, detail and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::transmittal::{can_see, visible};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::transmittal::{Purpose, Transmittal, TransmittalStatus};
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

fn status_pill(s: TransmittalStatus) -> Markup {
    html! { span class=(format!("pill {}", if s == TransmittalStatus::Sent { "green" } else { "violet" })) { (s.label()) } }
}

pub fn list(req: &Req<'_>, id: Id<Project>, status: Option<TransmittalStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Transmittals);
    if lvl < Level::Read {
        return not_found(req);
    }
    let needle = q.trim().to_lowercase();
    let shown: Vec<&Transmittal> = visible(state, req.actor(), id)
        .into_iter()
        .filter(|t| status.is_none_or(|s| t.status == s))
        .filter(|t| {
            needle.is_empty() || t.subject.to_lowercase().contains(&needle) || t.items.iter().any(|i| i.description.to_lowercase().contains(&needle))
        })
        .collect();
    let base = format!("/projects/{id}/transmittals");
    let body = html! {
        div class="page-head" {
            div { h1 { "Transmittals" } div class="sub" { "A formal record of what was sent, to whom and why" } }
            div class="actions" { @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New transmittal" } } }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search subject or items" aria-label="Search transmittals";
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in TransmittalStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Transmittals), "No transmittals", "Create one to record what you send to the team.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Subject" } th { "Purpose" } th { "To" } th { "Sent" } th { "Response due" } th { "Status" } } }
                tbody { @for t in &shown { tr {
                    td { (t.number) }
                    td { a href=(format!("{base}/{}", t.id)) { (t.subject) } @if t.private { " " span class="pill blue" { "Private" } } }
                    td { (t.purpose.label()) }
                    td { (view::people_names(state, &t.to)) }
                    td class="nowrap" { (t.sent.map(|s| req.when(s.at)).unwrap_or_default()) }
                    td class="nowrap" { (t.due.map(|d| d.to_string()).unwrap_or_default()) }
                    td { (status_pill(t.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Transmittals",
        Nav::Transmittals(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Transmittals", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, tid: Id<Transmittal>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(t) = state.transmittals.get(tid).filter(|t| t.project == id && t.deleted.is_none() && can_see(state, req.actor(), t)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/transmittals");
    let here = format!("{base}/{tid}");
    let ids = [("project", id.to_string()), ("id", tid.to_string())];
    let draft = t.status == TransmittalStatus::Draft;
    let can_edit = req.allowed("transmittal.update", Some(id)) && (draft || view::level(state, req.actor(), id, Tool::Transmittals) == Level::Admin);
    let body = html! {
        div class="page-head" {
            div { h1 { (t.label()) ": " (t.subject) " " (status_pill(t.status)) } div class="sub" { (t.purpose.label()) @if let Some(s) = t.sent { " · sent " (req.when(s.at)) " by " (s.by.map(|b| view::person_name(state, b)).unwrap_or_default()) } } }
            div class="actions" {
                @if can_edit { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if draft && can_edit { (button_form(req, "transmittal.send", &here, &ids, "Send", "primary")) }
                @if req.allowed("transmittal.delete", Some(id)) && (draft || view::level(state, req.actor(), id, Tool::Transmittals) == Level::Admin) { (button_form(req, "transmittal.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="card" {
            dl class="facts" {
                dt { "To" } dd { (dash(&view::people_names(state, &t.to))) }
                dt { "Copied" } dd { (dash(&view::people_names(state, &t.cc))) }
                dt { "Response wanted by" } dd { (t.due.map(|d| d.pretty()).unwrap_or_else(|| "—".into())) }
            }
            @if !t.message.is_empty() { p class="prose" { (t.message) } }
        }
        div class="card" {
            h2 { "Items" }
            @if t.items.is_empty() { p class="muted" { "No items listed." } } @else {
                table class="data" { thead { tr { th { "Description" } th class="num" { "Copies" } } } tbody { @for i in &t.items { tr { td { (i.description) } td class="num" { (i.copies) } } } } }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if t.attachments.is_empty() { p class="muted" { "No files attached." } } @else {
                ul class="plain" { @for f in &t.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if can_edit { " " (button_form(req, "transmittal.detach", &here, &[("project", id.to_string()), ("id", tid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if can_edit {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
    };
    layout::app(
        req,
        &t.label(),
        Nav::Transmittals(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Transmittals", Some(base)), (&t.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<Transmittal>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.transmittals.get(c).filter(|t| t.project == id && t.deleted.is_none() && can_see(state, req.actor(), t)) {
            Some(t) => Some(t),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "transmittal.update" } else { "transmittal.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/transmittals");
    let back = cur.map(|t| format!("{base}/{}/edit", t.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|t| format!("{base}/{}", t.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let people = user_options(req, id, true);
    let picker = |name: &str, label: &str, chosen: &[Id<buildkiosk_engine::model::directory::Person>]| {
        html! {
            div class="field full" {
                div class="label" { (label) }
                input type="hidden" name=(format!("{name}[]")) value="";
                details class="multi" {
                    summary { "Choose people" }
                    div class="options" { @for x in &people { label { input type="checkbox" name=(format!("{name}[]")) value=(x.id) checked[chosen.contains(&x.id)]; (x.name()) } } }
                }
            }
        }
    };
    let rows = cur.map(|t| t.items.len()).unwrap_or(0) + 3;
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit transmittal" } else { "New transmittal" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(t) = cur { input type="hidden" name="id" value=(t.id); }
                div class="form-grid" {
                    (field("Subject", "subject", true, true, html! { input type="text" id="subject" name="subject" required value=(cur.map(|t| t.subject.as_str()).unwrap_or_default()); }))
                    (field("Purpose", "purpose", false, false, html! { select id="purpose" name="purpose" { @for x in Purpose::ALL { option value=(x.key()) selected[cur.map(|t| t.purpose).unwrap_or_default() == *x] { (x.label()) } } } }))
                    (field("Response wanted by", "due", false, false, html! { input type="date" id="due" name="due" value=(cur.and_then(|t| t.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    (picker("to", "To", cur.map(|t| t.to.as_slice()).unwrap_or_default()))
                    (picker("cc", "Copy", cur.map(|t| t.cc.as_slice()).unwrap_or_default()))
                    (field("Visible to", "private", false, false, html! { select id="private" name="private" {
                        option value="false" selected[cur.is_none_or(|t| !t.private)] { "Everyone with Transmittals access" }
                        option value="true" selected[cur.is_some_and(|t| t.private)] { "Only me, the recipients and admins" }
                    } }))
                    (field("Message", "message", false, true, html! { textarea id="message" name="message" { (cur.map(|t| t.message.as_str()).unwrap_or_default()) } }))
                }
                h2 { "Items sent" }
                table class="data" {
                    thead { tr { th { "Description" } th class="num" { "Copies" } } }
                    tbody { @for i in 0..rows { @let item = cur.and_then(|t| t.items.get(i)); tr {
                        td { input type="text" name=(format!("items.{i}.description")) value=(item.map(|x| x.description.as_str()).unwrap_or_default()) aria-label=(format!("Item {} description", i + 1)); }
                        td class="num" { input type="number" min="0" name=(format!("items.{i}.copies")) value=(item.map(|x| x.copies.to_string()).unwrap_or_default()) aria-label=(format!("Item {} copies", i + 1)); }
                    } } }
                }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|t| format!("{base}/{}", t.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save draft" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Transmittal",
        Nav::Transmittals(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Transmittals", Some(base)), ("Transmittal", None)],
        body,
    )
}
