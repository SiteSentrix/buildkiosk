//! Forms pages: the list of filled-in forms, the project's templates, and the fill-in page.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::form::visible;
use buildkiosk_engine::model::form::{FieldKind, FormField, FormRecord, FormStatus, FormTemplate};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{human_size, not_found, project_for};

fn status_pill(s: FormStatus) -> Markup {
    html! { span class=(format!("pill {}", if s == FormStatus::Submitted { "green" } else { "violet" })) { (s.label()) } }
}

fn templates_of<'a>(req: &'a Req<'_>, id: Id<Project>) -> Vec<&'a FormTemplate> {
    let mut v: Vec<&FormTemplate> = req.engine.state().form_templates.iter().filter(|t| t.project == id && t.deleted.is_none()).collect();
    v.sort_by_key(|t| t.name.to_lowercase());
    v
}

pub fn list(req: &Req<'_>, id: Id<Project>, template: Option<Id<FormTemplate>>, status: Option<FormStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Forms);
    if lvl < Level::Read {
        return not_found(req);
    }
    let templates = templates_of(req, id);
    let needle = q.trim().to_lowercase();
    let shown: Vec<&FormRecord> = visible(state, id)
        .into_iter()
        .filter(|r| template.is_none_or(|t| r.template == t))
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| {
            needle.is_empty()
                || r.title.to_lowercase().contains(&needle)
                || r.template_name.to_lowercase().contains(&needle)
                || r.answers.values().any(|a| a.to_lowercase().contains(&needle))
        })
        .collect();
    let base = format!("/projects/{id}/forms");
    let body = html! {
        div class="page-head" {
            div { h1 { "Forms" } div class="sub" { "Checklists and reports filled in from the project's own templates" } }
            div class="actions" { a class="btn" href=(format!("{base}/templates")) { "Templates" } }
        }
        @if lvl >= Level::Standard && !templates.is_empty() {
            div class="card" {
                h2 { "Start a form" }
                (command_form(req, "form.create", &format!("{base}/{{id}}"), &base, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Template", "template", true, false, html! { select id="template" name="template" required { @for t in &templates { option value=(t.id) { (t.name) } } } }))
                        (field("Title (optional)", "title", false, false, html! { input type="text" id="title" name="title" placeholder="Slab A"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Start" } }
                }))
            }
        } @else if templates.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Forms), "No templates yet", if lvl == Level::Admin { "Create a template on the Templates page, then start forms from it." } else { "A Forms admin needs to create a template first." })) }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search title or answers" aria-label="Search forms";
            select name="template" aria-label="Template" {
                option value="" { "Any template" }
                @for t in &templates { option value=(t.id) selected[template == Some(t.id)] { (t.name) } }
            }
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in FormStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if !shown.is_empty() {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Form" } th { "Title" } th { "Started" } th { "Status" } } }
                tbody { @for r in &shown { tr {
                    td { (r.number) }
                    td { a href=(format!("{base}/{}", r.id)) { (r.template_name) } }
                    td { (r.title) }
                    td class="nowrap" { (req.when(r.created.at)) " " (r.created.by.map(|b| view::person_name(state, b)).unwrap_or_default()) }
                    td { (status_pill(r.status)) }
                } } }
            } }
        }
    };
    layout::app(req, "Forms", Nav::Forms(id), &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Forms", None)], body)
}

pub fn templates(req: &Req<'_>, id: Id<Project>, editing: Option<Id<FormTemplate>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Forms);
    if lvl < Level::Read {
        return not_found(req);
    }
    let base = format!("/projects/{id}/forms");
    let here = format!("{base}/templates");
    let list = templates_of(req, id);
    let cur = editing.and_then(|e| list.iter().copied().find(|t| t.id == e));
    let admin = lvl == Level::Admin;
    let rows = cur.map(|t| t.fields.len()).unwrap_or(0) + 4;
    let body = html! {
        div class="page-head" { div { h1 { "Form templates" } div class="sub" { "The questions each kind of form asks. Forms already started keep the questions they were started with." } } }
        @if list.is_empty() { div class="card" { (empty(tool_icon(Tool::Forms), "No templates", "Create the first template below.")) } } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Template" } th { "Questions" } th {} } }
                tbody { @for t in &list { tr {
                    td { strong { (t.name) } @if !t.description.is_empty() { br; span class="muted small" { (t.description) } } }
                    td { (t.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>().join(" · ")) }
                    td class="right" { @if admin { a class="btn small" href=(format!("{here}/{}", t.id)) { "Edit" } " " (button_form(req, "form_template.delete", &here, &[("project", id.to_string()), ("id", t.id.to_string())], "Delete", "small danger")) } }
                } } }
            } }
        }
        @if admin {
            div class="card" {
                h2 { (if cur.is_some() { "Edit template" } else { "New template" }) }
                (command_form(req, if cur.is_some() { "form_template.update" } else { "form_template.create" }, &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    @if let Some(t) = cur { input type="hidden" name="id" value=(t.id); }
                    div class="form-grid" {
                        (field("Name", "t_name", true, false, html! { input type="text" id="t_name" name="name" required value=(cur.map(|t| t.name.as_str()).unwrap_or_default()); }))
                        (field("Description", "t_desc", false, false, html! { input type="text" id="t_desc" name="description" value=(cur.map(|t| t.description.as_str()).unwrap_or_default()); }))
                    }
                    table class="data" {
                        thead { tr { th { "Question" } th { "Answer type" } th { "Choices (for a list)" } th { "Required" } } }
                        tbody { @for i in 0..rows { @let f = cur.and_then(|t| t.fields.get(i)); tr {
                            td { input type="text" name=(format!("fields.{i}.label")) value=(f.map(|x| x.label.as_str()).unwrap_or_default()) aria-label=(format!("Question {}", i + 1)); }
                            td { select name=(format!("fields.{i}.kind")) aria-label=(format!("Answer type of question {}", i + 1)) { @for k in FieldKind::ALL { option value=(k.key()) selected[f.is_some_and(|x| x.kind == *k)] { (k.label()) } } } }
                            td { input type="text" name=(format!("fields.{i}.options")) value=(f.map(|x| x.options.join(", ")).unwrap_or_default()) placeholder="a, b, c" aria-label=(format!("Choices for question {}", i + 1)); }
                            td { input type="checkbox" name=(format!("fields.{i}.required")) value="true" checked[f.is_some_and(|x| x.required)] aria-label=(format!("Question {} is required", i + 1)); }
                        } } }
                    }
                    div class="form-actions" {
                        @if cur.is_some() { a class="btn" href=(here.clone()) { "Cancel" } }
                        button type="submit" class="btn primary" { "Save template" }
                    }
                }))
            }
        }
    };
    layout::app(
        req,
        "Form templates",
        Nav::Forms(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Forms", Some(base)), ("Templates", None)],
        body,
    )
}

fn answer_input(f: &FormField, value: &str) -> Markup {
    let name = format!("answers.{}", f.key);
    let id = format!("a_{}", f.key);
    html! {
        @match f.kind {
            FieldKind::Text => input type="text" id=(id) name=(name) value=(value);
            FieldKind::LongText => textarea id=(id) name=(name) { (value) }
            FieldKind::Number => input type="text" inputmode="decimal" id=(id) name=(name) value=(value);
            FieldKind::Date => input type="date" id=(id) name=(name) value=(value);
            FieldKind::YesNo => select id=(id) name=(name) { option value="" selected[value.is_empty()] { "—" } option value="yes" selected[value == "yes"] { "Yes" } option value="no" selected[value == "no"] { "No" } }
            FieldKind::Choice => select id=(id) name=(name) { option value="" selected[value.is_empty()] { "—" } @for o in &f.options { option value=(o) selected[o == value] { (o) } } }
        }
    }
}

pub fn detail(req: &Req<'_>, id: Id<Project>, rid: Id<FormRecord>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Forms);
    let Some(r) = state.form_records.get(rid).filter(|r| r.project == id && r.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/forms");
    let here = format!("{base}/{rid}");
    let ids = [("project", id.to_string()), ("id", rid.to_string())];
    let admin = lvl == Level::Admin;
    let author = r.created.by.is_some() && r.created.by == req.actor().person();
    let editing = req.allowed("form.update", Some(id)) && (admin || (r.status == FormStatus::Draft && author));
    let answer = |key: &str| r.answers.get(key).cloned().unwrap_or_default();
    let pretty = |f: &FormField, v: &str| -> String {
        match (f.kind, v) {
            (_, "") => "—".into(),
            (FieldKind::YesNo, "yes") => "Yes".into(),
            (FieldKind::YesNo, "no") => "No".into(),
            _ => v.to_string(),
        }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { (r.label()) @if !r.title.is_empty() { ": " (r.title) } " " (status_pill(r.status)) } div class="sub" { "Started " (req.when(r.created.at)) " by " (r.created.by.map(|b| view::person_name(state, b)).unwrap_or_default()) } }
            div class="actions" {
                @if editing && r.status == FormStatus::Draft { (button_form(req, "form.submit", &here, &ids, "Submit", "primary")) }
                @if admin && r.status == FormStatus::Submitted { (button_form(req, "form.reopen", &here, &ids, "Reopen", "")) }
                @if editing { (button_form(req, "form.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="card" {
            @if editing {
                (command_form(req, "form.update", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(rid);
                    div class="form-grid" {
                        (field("Title", "title", false, true, html! { input type="text" id="title" name="title" value=(r.title); }))
                        @for f in &r.fields { (field(&f.label, &format!("a_{}", f.key), f.required, f.kind == FieldKind::LongText, answer_input(f, &answer(&f.key)))) }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save answers" } }
                }))
            } @else {
                dl class="facts" { @for f in &r.fields { dt { (f.label) } dd { (pretty(f, &answer(&f.key))) } } }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if r.attachments.is_empty() { p class="muted" { "No files attached." } } @else {
                ul class="plain" { @for f in &r.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if editing { " " (button_form(req, "form.detach", &here, &[("project", id.to_string()), ("id", rid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if editing {
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
        &r.label(),
        Nav::Forms(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Forms", Some(base)), (&r.label(), None)],
        body,
    )
}
