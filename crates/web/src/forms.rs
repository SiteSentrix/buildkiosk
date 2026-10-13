//! Forms pages: the list of filled-in forms, the project's templates, and the fill-in page.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::form::{form_report, schedules, visible};
use buildkiosk_engine::model::form::{FieldKind, FormField, FormRecord, FormStatus, FormTemplate};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, csv_cell, empty, field};
use crate::pages::{human_size, not_found, project_for, user_options};

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
            div class="actions" { a class="btn" href=(format!("{base}/report")) { "Report" } a class="btn" href=(format!("{base}/templates")) { "Templates" } }
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
    let others: Vec<&Project> = view::projects_for(state, req.actor())
        .into_iter()
        .filter(|o| o.id != id && view::level(state, req.actor(), o.id, Tool::Forms) == Level::Admin)
        .collect();
    let sched = schedules(state, id);
    let people = if admin { user_options(req, id, true) } else { Vec::new() };
    let body = html! {
        div class="page-head" { div { h1 { "Form templates" } div class="sub" { "The questions each kind of form asks. Forms already started keep the questions they were started with." } } }
        @if list.is_empty() { div class="card" { (empty(tool_icon(Tool::Forms), "No templates", "Create the first template below.")) } } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Template" } th { "Questions" } th {} } }
                tbody { @for t in &list { tr {
                    td { strong { (t.name) } @if !t.description.is_empty() { br; span class="muted small" { (t.description) } } }
                    td { (t.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>().join(" · ")) }
                    td class="right" { @if admin { a class="btn small" href=(format!("{here}/{}", t.id)) { "Edit" } " "
                        @if !others.is_empty() { details class="disclosure" { summary { "Copy to another project" }
                            (command_form(req, "form_template.copy", &here, &here, html! {
                                input type="hidden" name="project" value=(id);
                                input type="hidden" name="id" value=(t.id);
                                select name="to_project" aria-label="Project to copy to" { @for o in &others { option value=(o.id) { (o.name) } } }
                                button type="submit" class="btn small" { "Copy" }
                            }))
                        } } " "
                        (button_form(req, "form_template.delete", &here, &[("project", id.to_string()), ("id", t.id.to_string())], "Delete", "small danger")) } }
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
                    div class="form-grid" {
                        (field("Signing roles (separated by commas)", "t_signers", false, true, html! { input type="text" id="t_signers" name="signers" value=(cur.map(|t| t.signers.join(", ")).unwrap_or_default()) placeholder="Inspector, Superintendent"; }))
                    }
                    table class="data" {
                        thead { tr { th { "Question" } th { "Answer type" } th { "Choices (for a list)" } th { "Required" } th { "Only if question no." } th { "has the answer" } } }
                        tbody { @for i in 0..rows { @let f = cur.and_then(|t| t.fields.get(i)); tr {
                            td { input type="text" name=(format!("fields.{i}.label")) value=(f.map(|x| x.label.as_str()).unwrap_or_default()) aria-label=(format!("Question {}", i + 1)); }
                            td { select name=(format!("fields.{i}.kind")) aria-label=(format!("Answer type of question {}", i + 1)) { @for k in FieldKind::ALL { option value=(k.key()) selected[f.is_some_and(|x| x.kind == *k)] { (k.label()) } } } }
                            td { input type="text" name=(format!("fields.{i}.options")) value=(f.map(|x| x.options.join(", ")).unwrap_or_default()) placeholder="a, b, c" aria-label=(format!("Choices for question {}", i + 1)); }
                            td { input type="checkbox" name=(format!("fields.{i}.required")) value="true" checked[f.is_some_and(|x| x.required)] aria-label=(format!("Question {} is required", i + 1)); }
                            @let cond = f.and_then(|x| x.show_if.as_ref());
                            @let cond_n = cond.and_then(|c| cur.and_then(|t| t.fields.iter().position(|y| y.key == c.field))).map(|n| (n + 1).to_string()).unwrap_or_default();
                            td { input type="text" inputmode="numeric" size="3" name=(format!("fields.{i}.show_if_question")) value=(cond_n) aria-label=(format!("Question {} is asked only if this earlier question number", i + 1)); }
                            td { input type="text" name=(format!("fields.{i}.show_if_answer")) value=(cond.map(|c| c.equals.as_str()).unwrap_or_default()) aria-label=(format!("Answer that shows question {}", i + 1)); }
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
    let body = html! {
        (body)
        div class="card" {
            h2 { "Recurring forms" }
            @if sched.is_empty() { p class="muted" { "No schedules. A schedule starts a draft of a template on a rhythm and tells the person who fills it in." } } @else {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "Template" } th { "Every" } th { "Next" } th { "Filled in by" } th {} } }
                    tbody { @for sc in &sched { tr {
                        td { (state.form_templates.get(sc.template).map(|t| t.name.as_str()).unwrap_or("(deleted)")) @if !sc.title.is_empty() { " · " (sc.title) } @if !sc.active { " " span class="pill" { "Stopped" } } }
                        td { (sc.every.label()) } td { (sc.next.to_string()) } td { (view::person_name(state, sc.assignee)) }
                        td class="right" { @if admin { (button_form(req, "form_schedule.delete", &here, &[("project", id.to_string()), ("id", sc.id.to_string())], "Delete", "small danger")) } }
                    } } }
                } }
            }
            @if admin && !list.is_empty() {
                (command_form(req, "form_schedule.save", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Template", "s_tpl", true, false, html! { select id="s_tpl" name="template" required { @for t in &list { option value=(t.id) { (t.name) } } } }))
                        (field("Every", "s_every", true, false, html! { select id="s_every" name="every" { option value="daily" { "Day" } option value="weekly" { "Week" } option value="biweekly" { "Two weeks" } option value="monthly" { "Month" } } }))
                        (field("Filled in by", "s_who", true, false, html! { select id="s_who" name="assignee" required { @for x in &people { option value=(x.id) { (x.name()) } } } }))
                        (field("First form on", "s_next", false, false, html! { input type="date" id="s_next" name="next" value=(req.today().to_string()); }))
                        (field("Title", "s_title", false, false, html! { input type="text" id="s_title" name="title" placeholder="Weekly walk"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Schedule" } }
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
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
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
                        @for f in &r.fields { (field(&match &f.show_if { Some(c) => format!("{} (only if {} is {})", f.label, r.fields.iter().find(|x| x.key == c.field).map(|x| x.label.as_str()).unwrap_or("an earlier question"), c.equals), None => f.label.clone() }, &format!("a_{}", f.key), f.required, f.kind == FieldKind::LongText, answer_input(f, &answer(&f.key)))) }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save answers" } }
                }))
            } @else {
                dl class="facts" { @for f in r.shown_fields() { dt { (f.label) } dd { (pretty(f, &answer(&f.key))) } } }
            }
        }
        @if !r.signers.is_empty() {
            div class="card" {
                h2 { "Signatures" }
                table class="data" {
                    thead { tr { th { "Role" } th { "Signed by" } th { "When" } } }
                    tbody { @for role in &r.signers { @let sg = r.signatures.iter().find(|x| x.role.eq_ignore_ascii_case(role)); tr {
                        td { (role) }
                        td { @if let Some(x) = sg { (x.name) } @else { span class="pill" { "Not signed" } } }
                        td { @if let Some(x) = sg { (req.when(x.stamp.at)) } }
                    } } }
                }
                @if r.status == FormStatus::Submitted && req.allowed("form.sign", Some(id)) && !r.unsigned().is_empty() {
                    (command_form(req, "form.sign", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(rid);
                        div class="form-grid" {
                            (field("Sign as", "sg_role", true, false, html! { select id="sg_role" name="role" { @for role in r.unsigned() { option value=(role) { (role) } } } }))
                            (field("Type your full name", "sg_name", true, false, html! { input type="text" id="sg_name" name="name" required autocomplete="name"; }))
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Sign" } }
                    }))
                } @else if r.status == FormStatus::Draft {
                    p class="muted small" { "Signing opens once the form is submitted." }
                }
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

/// Every form of one template as a table, one column per question.
pub fn report(
    req: &Req<'_>,
    id: Id<Project>,
    template: Option<Id<FormTemplate>>,
    status: Option<FormStatus>,
    from: Option<buildkiosk_core::Date>,
    to: Option<buildkiosk_core::Date>,
) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Forms) < Level::Read {
        return not_found(req);
    }
    let base = format!("/projects/{id}/forms");
    let templates = templates_of(req, id);
    let chosen = template.and_then(|t| templates.iter().copied().find(|x| x.id == t)).or_else(|| templates.first().copied());
    let rep = chosen.map(|t| form_report(state, id, t.id, status, from, to));
    let q = |extra: &str| {
        format!(
            "template={}&status={}&from={}&to={}{extra}",
            chosen.map(|t| t.id.to_string()).unwrap_or_default(),
            status.map(|s| s.key()).unwrap_or(""),
            from.map(|d| d.to_string()).unwrap_or_default(),
            to.map(|d| d.to_string()).unwrap_or_default()
        )
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "Forms report" } div class="sub" { "Every form from one template side by side" } }
            div class="actions" { @if chosen.is_some() { a class="btn" href=(format!("{base}/report.csv?{}", q(""))) { "Export CSV" } } a class="btn" href=(base.clone()) { "Back to forms" } }
        }
        form method="get" action=(format!("{base}/report")) class="filters" {
            select name="template" aria-label="Template" { @for t in &templates { option value=(t.id) selected[chosen.is_some_and(|c| c.id == t.id)] { (t.name) } } }
            select name="status" aria-label="Status" { option value="" { "Any status" } @for s in FormStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } } }
            input type="date" name="from" value=(from.map(|d| d.to_string()).unwrap_or_default()) aria-label="Started from";
            input type="date" name="to" value=(to.map(|d| d.to_string()).unwrap_or_default()) aria-label="Started to";
            button type="submit" class="btn" { "Show" }
        }
        @match &rep {
            None => { div class="card" { (empty(tool_icon(Tool::Forms), "No templates", "Create a template first.")) } }
            Some(r) if r.rows.is_empty() => { div class="card" { p class="muted" { "No forms match." } } }
            Some(r) => {
                div class="table-wrap" { table class="data" {
                    thead { tr { th { "#" } th { "Title" } th { "Status" } @for c in &r.columns { th { (c) } } } }
                    tbody { @for (rec, cells) in &r.rows { tr {
                        td { a href=(format!("{base}/{}", rec.id)) { (rec.number) } } td { (rec.title) } td { (status_pill(rec.status)) }
                        @for c in cells { td { (c) } }
                    } } }
                } }
            }
        }
    };
    layout::app(
        req,
        "Forms report",
        Nav::Forms(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Forms", Some(base)), ("Report", None)],
        body,
    )
}

pub fn report_csv(
    req: &Req<'_>,
    id: Id<Project>,
    template: Id<FormTemplate>,
    status: Option<FormStatus>,
    from: Option<buildkiosk_core::Date>,
    to: Option<buildkiosk_core::Date>,
) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Forms) < Level::Read || !state.form_templates.get(template).is_some_and(|t| t.project == id) {
        return None;
    }
    let rep = form_report(state, id, template, status, from, to);
    let mut head = vec!["Number".to_string(), "Title".to_string(), "Status".to_string(), "Started".to_string(), "Started by".to_string()];
    head.extend(rep.columns.iter().cloned());
    let mut out = head.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(",");
    out.push_str("\r\n");
    for (rec, cells) in &rep.rows {
        let mut row = vec![
            rec.number.to_string(),
            rec.title.clone(),
            rec.status.label().to_string(),
            rec.created.at.date_at(state.utc_offset()).to_string(),
            rec.created.by.map(|b| view::person_name(state, b)).unwrap_or_default(),
        ];
        row.extend(cells.iter().cloned());
        out.push_str(&row.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
