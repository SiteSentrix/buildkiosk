//! Inspection pages: list, detail with the checklist, new form and the template page.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::inspection::{can_answer, can_edit};
use buildkiosk_engine::model::inspection::{Answer, Inspection, InspectionStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

/// Whether line `n` opens a new, named section.
/// A template's checklist as the text the form takes: `# Section` lines and one line per item.
fn items_text(items: &[buildkiosk_engine::model::inspection::TemplateItem]) -> String {
    let mut out = String::new();
    for (n, item) in items.iter().enumerate() {
        if starts_section(items, n) {
            out.push_str(&format!("# {}\n", item.section));
        }
        out.push_str(&item.text);
        out.push('\n');
    }
    out
}

fn starts_section(items: &[impl HasSection], n: usize) -> bool {
    let Some(item) = items.get(n) else { return false };
    !item.section().is_empty() && n.checked_sub(1).and_then(|p| items.get(p)).is_none_or(|prev| prev.section() != item.section())
}

trait HasSection {
    fn section(&self) -> &str;
}

impl HasSection for buildkiosk_engine::model::inspection::ChecklistItem {
    fn section(&self) -> &str {
        &self.section
    }
}

impl HasSection for buildkiosk_engine::model::inspection::TemplateItem {
    fn section(&self) -> &str {
        &self.section
    }
}

fn answer_pill(a: Answer) -> Markup {
    let class = match a {
        Answer::Pass => "pill green",
        Answer::Fail => "pill red",
        Answer::NotApplicable => "tag",
    };
    html! { span class=(class) { (a.label()) } }
}

fn outcome_pill(i: &Inspection) -> Markup {
    match (i.status, i.failed()) {
        (InspectionStatus::Open, _) => html! { span class="pill blue" { "In progress" } },
        (InspectionStatus::Completed, 0) => html! { span class="pill green" { "Passed" } },
        (InspectionStatus::Completed, n) => html! { span class="pill red" { (n) " failed" } },
    }
}

/// Filters of the list, from the query string.
#[derive(Default)]
pub struct Filter {
    pub status: Option<InspectionStatus>,
    pub q: String,
    pub mine: bool,
}

fn crumbs<'a>(p: &'a Project, last: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some((l, list)) => {
            c.push(("Inspections", Some(list)));
            c.push((l, None));
        }
        None => c.push(("Inspections", None)),
    }
    c
}

pub fn list(req: &Req<'_>, id: Id<Project>, filter: &Filter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Inspections) < Level::Read {
        return not_found(req);
    }
    let all = view::inspections_for(state, req.actor(), id);
    let count = |s: InspectionStatus| all.iter().filter(|x| x.status == s).count();
    let q = filter.q.to_lowercase();
    let today = req.today();
    let shown: Vec<&Inspection> = all
        .iter()
        .copied()
        .filter(|i| filter.status.is_none_or(|s| i.status == s))
        .filter(|i| !filter.mine || i.inspector == Some(req.person))
        .filter(|i| q.is_empty() || i.title.to_lowercase().contains(&q) || i.location.to_lowercase().contains(&q) || i.number.to_string() == q)
        .collect();
    let base = format!("/projects/{id}/inspections");
    let tab = |label: &str, status: Option<InspectionStatus>, n: usize| {
        let href = match status {
            Some(s) => format!("{base}?status={}", s.key()),
            None => base.clone(),
        };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "Inspections" } div class="sub" { "Checklists walked on site" } }
            div class="actions" {
                a class="btn" href=(format!("{base}/templates")) { "Templates" }
                a class="btn" href=(format!("{base}.csv")) { (Icon::Download.svg()) "Export CSV" }
                @if req.allowed("inspection.create", Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) "Start inspection" } }
            }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("In progress", Some(InspectionStatus::Open), count(InspectionStatus::Open)))
            (tab("Completed", Some(InspectionStatus::Completed), count(InspectionStatus::Completed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title, location or number" aria-label="Search inspections";
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Assigned to me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Inspections), "No inspections here", if all.is_empty() { "Start an inspection from a template to get going." } else { "No inspection matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Inspection" } th { "Location" } th { "Inspector" } th { "Scheduled" } th { "Progress" } th { "Result" } } }
                    tbody {
                        @for i in &shown {
                            tr {
                                td class="nowrap" { (i.number) }
                                td { a class="row-link" href=(format!("{base}/{}", i.id)) { (i.title) } }
                                td { (i.location) }
                                td { @if let Some(x) = i.inspector { (view::person_name(state, x)) } }
                                td class="nowrap" { @if let Some(d) = i.scheduled { span class=(if i.is_overdue(today) { "overdue" } else { "" }) { (d.pretty()) } } }
                                td class="nowrap" { (i.answered()) " of " (i.items.len()) }
                                td { (outcome_pill(i)) }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, "Inspections", Nav::Inspections(id), &crumbs(p, None), body)
}

pub fn detail(req: &Req<'_>, id: Id<Project>, iid: Id<Inspection>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = state.inspections.get(iid).filter(|i| i.project == id && view::can_see_inspection(state, req.actor(), i)) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/inspections");
    let here = format!("{base}/{}", i.id);
    let ids = [("project", id.to_string()), ("id", i.id.to_string())];
    let level = view::level(state, req.actor(), id, Tool::Inspections);
    let manager_like = level == Level::Admin || i.manager == Some(req.person);
    let editing = can_answer(state, req.actor(), i);
    let standard = level >= Level::Standard;
    let people = user_options(req, id, true);
    let body = html! {
        div class="page-head" {
            div {
                h1 { (i.label()) ": " (i.title) " " (outcome_pill(i)) }
                div class="sub" { @if !i.template_name.is_empty() { "From “" (i.template_name) "” · " } "Started " (req.when(i.created.at)) @if let Some(b) = i.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "Printable checklist" }
                @if can_edit(state, req.actor(), i) { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if standard && manager_like && i.status == InspectionStatus::Completed { (button_form(req, "inspection.reopen", &here, &ids, "Reopen", "")) }
                @if standard && (level == Level::Admin || (i.answered() == 0 && i.created.by == Some(req.person))) { (button_form(req, "inspection.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="detail" {
            div {
                div class="card" {
                    h2 { "Checklist" }
                    @if editing {
                        form method="post" action=(format!("{here}/answers")) {
                            input type="hidden" name="_csrf" value=(req.csrf);
                            @for (n, item) in i.items.iter().enumerate() {
                                @if starts_section(&i.items, n) { h3 { (item.section) } }
                                div class="response" {
                                    p { strong { (n + 1) ". " } (item.text) }
                                    div class="row" {
                                        @for (value, label) in [("", "Not answered"), ("pass", "Pass"), ("fail", "Fail"), ("na", "N/A")] {
                                            label class="check" { input type="radio" name=(format!("line.{n}.answer")) value=(value) checked[item.answer.map(|a| a.key()).unwrap_or("") == value]; (label) }
                                        }
                                    }
                                    input type="text" name=(format!("line.{n}.note")) value=(item.note) placeholder="Note (optional)" aria-label=(format!("Note for line {}", n + 1));
                                }
                            }
                            div class="form-actions" {
                                button type="submit" class="btn" { "Save answers" }
                                button type="submit" name="_complete" value="1" class="btn primary" { "Save and complete" }
                            }
                        }
                    } @else {
                        @for (n, item) in i.items.iter().enumerate() {
                            @if starts_section(&i.items, n) { h3 { (item.section) } }
                            div class="response" {
                                div class="meta" {
                                    span { strong { (n + 1) ". " } (item.text) }
                                    span { @if let Some(a) = item.answer { (answer_pill(a)) } @else { span class="muted" { "Not answered" } } }
                                }
                                @if !item.note.is_empty() { p class="prose" { (item.note) } }
                                @if let Some(pid) = item.punch { p class="small" { a href=(format!("/projects/{id}/punch-list/{pid}")) { "Punch item raised" } } }
                            }
                        }
                    }
                }
                @if i.status == InspectionStatus::Completed && i.unraised() > 0 && standard && manager_like {
                    div class="card" {
                        h2 { (i.unraised()) " failed line" @if i.unraised() != 1 { "s" } " without a punch item" }
                        (command_form(req, "inspection.raise_punch", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id);
                            input type="hidden" name="assignees[]" value="";
                            details class="multi" {
                                summary { "Assign the punch items to…" }
                                div class="options" { @for x in &people { label { input type="checkbox" name="assignees[]" value=(x.id); (x.name()) } } }
                            }
                            div class="form-actions" { button type="submit" class="btn primary" { "Raise punch items" } }
                        }))
                    }
                }
            }
            div class="stack" {
                div class="card" {
                    h2 { "Details" }
                    dl class="facts" {
                        dt { "Progress" } dd { (i.answered()) " of " (i.items.len()) " answered" @if i.failed() > 0 { ", " (i.failed()) " failed" } }
                        dt { "Location" } dd { (dash(&i.location)) }
                        dt { "Kind" } dd { (dash(&i.kind)) }
                        dt { "Inspector" } dd { @if let Some(x) = i.inspector { (view::person_name(state, x)) } @else { "—" } }
                        dt { "Manager" } dd { @if let Some(x) = i.manager { (view::person_name(state, x)) } @else { "—" } }
                        dt { "Scheduled" } dd { @if let Some(d) = i.scheduled { span class=(if i.is_overdue(req.today()) { "overdue" } else { "" }) { (d.pretty()) } } @else { "—" } }
                        @if let Some(c) = i.completed { dt { "Completed" } dd { (req.when(c.at)) } }
                    }
                }
                div class="card" {
                    h2 { "Files" }
                    @if i.attachments.is_empty() { p class="muted" { "No files attached." } }
                    @for a in &i.attachments {
                        div class="row" {
                            a href=(format!("{here}/files/{}", a.sha256)) { (a.name) }
                            span class="muted small" { (human_size(a.size)) }
                            @if editing || (standard && manager_like) { (button_form(req, "inspection.detach", &here, &[("project", id.to_string()), ("id", i.id.to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
                        }
                    }
                    @if editing || (standard && manager_like) {
                        form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                            input type="hidden" name="_csrf" value=(req.csrf);
                            input type="file" name="file" required aria-label="File to attach";
                            button type="submit" class="btn small" { "Attach file" }
                        }
                    }
                }
                div class="card" {
                    h2 { "History" }
                    ul class="timeline" {
                        @for e in history.iter().rev() {
                            li {
                                div { (find_command(&e.cmd).map(|c| c.label).unwrap_or(e.cmd.as_str())) " by " @if let Some(a) = e.actor { (view::person_name(state, a)) } @else { "the system" } }
                                div class="when" { (req.when(e.at)) }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, &i.label(), Nav::Inspections(id), &crumbs(p, Some((&i.label(), base.clone()))), body)
}

pub fn form(req: &Req<'_>, id: Id<Project>, existing: Option<Id<Inspection>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(iid) => match state
            .inspections
            .get(iid)
            .filter(|i| i.project == id && view::can_see_inspection(state, req.actor(), i) && can_edit(state, req.actor(), i))
        {
            Some(i) => Some(i),
            None => return not_found(req),
        },
        None => {
            if !req.allowed("inspection.create", Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let people = user_options(req, id, true);
    let templates: Vec<_> = state.inspection_templates.iter().filter(|t| t.project == id && t.deleted.is_none()).collect();
    let base = format!("/projects/{id}/inspections");
    let (command, next, back) = match current {
        Some(i) => ("inspection.update", format!("{base}/{}", i.id), format!("{base}/{}/edit", i.id)),
        None => ("inspection.create", format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&Inspection) -> String| current.map(f).unwrap_or_default();
    let manager = current.map(|i| i.manager).unwrap_or(Some(req.person));
    let inspector = current.and_then(|i| i.inspector);
    let title = if current.is_some() { "Edit inspection" } else { "Start inspection" };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(i) = current { input type="hidden" name="id" value=(i.id); }
                div class="form-grid" {
                    @if current.is_none() {
                        div class="field full" {
                            label for="template" { "Checklist template" }
                            select id="template" name="template" {
                                option value="" { "No template (type the checklist below)" }
                                @for t in &templates { option value=(t.id) { (t.name) " (" (t.items.len()) " lines)" } }
                            }
                            @if templates.is_empty() { div class="hint" { "No templates yet. Add your own or the starter set on the " a href=(format!("{base}/templates")) { "templates page" } "." } }
                        }
                    }
                    (field("Title", "title", false, true, html! { input type="text" id="title" name="title" maxlength="200" value=(text(|i| i.title.clone())); div class="hint" { "Leave blank to use the template's name." } }))
                    (field("Location", "location", false, false, html! { input type="text" id="location" name="location" value=(text(|i| i.location.clone())); }))
                    (field("Kind", "kind", false, false, html! { input type="text" id="kind" name="kind" value=(text(|i| i.kind.clone())); }))
                    (field("Inspector", "inspector", false, false, html! { select id="inspector" name="inspector" { option value="" { "—" } @for x in &people { option value=(x.id) selected[Some(x.id) == inspector] { (x.name()) } } } }))
                    (field("Manager", "manager", false, false, html! { select id="manager" name="manager" { @for x in &people { option value=(x.id) selected[Some(x.id) == manager] { (x.name()) } } } }))
                    (field("Scheduled for", "scheduled", false, false, html! { input type="date" id="scheduled" name="scheduled" value=(current.and_then(|i| i.scheduled).map(|d| d.to_string()).unwrap_or_default()); }))
                    @if current.is_none() {
                        (field("Checklist lines (when no template)", "items", false, true, html! { textarea id="items" name="items" class="short" placeholder="# Section heading\nOne checklist line per line" {} }))
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(current.map(|i| format!("{base}/{}", i.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { "Start inspection" }) }
                }
            }))
        }
    };
    layout::app(req, title, Nav::Inspections(id), &crumbs(p, Some((title, base.clone()))), body)
}

pub fn templates(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Inspections) < Level::Read {
        return not_found(req);
    }
    let mut list: Vec<_> = state.inspection_templates.iter().filter(|t| t.project == id && t.deleted.is_none()).collect();
    list.sort_by_key(|t| t.name.to_lowercase());
    let here = format!("/projects/{id}/inspections/templates");
    let can_edit = req.allowed("inspection_template.create", Some(id));
    let body = html! {
        div class="page-head" {
            div { h1 { "Inspection templates" } div class="sub" { "Reusable checklists; starting an inspection copies one" } }
            div class="actions" { @if can_edit { (button_form(req, "inspection_template.install_starters", &here, &[("project", id.to_string())], "Add the starter templates", "")) } }
        }
        @if list.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Inspections), "No templates yet", "Write your own below, or add the small starter set and change it to suit.")) }
        } @else {
            @for t in &list {
                div class="card" {
                    div class="card-head" {
                        h2 { (t.name) @if !t.kind.is_empty() { " " span class="tag" { (t.kind) } } }
                        @if can_edit { (button_form(req, "inspection_template.delete", &here, &[("project", id.to_string()), ("id", t.id.to_string())], "Delete", "small danger")) }
                    }
                    @if can_edit {
                        details class="disclosure" {
                            summary { "Edit this template" }
                            p class="muted small" { "Inspections already started keep the checklist they were made with." }
                            (command_form(req, "inspection_template.update", &here, &here, html! {
                                input type="hidden" name="project" value=(id);
                                input type="hidden" name="id" value=(t.id);
                                div class="form-grid" {
                                    (field("Name", &format!("n{}", t.id), true, false, html! { input type="text" id=(format!("n{}", t.id)) name="name" value=(t.name) required; }))
                                    (field("Kind", &format!("k{}", t.id), false, false, html! { input type="text" id=(format!("k{}", t.id)) name="kind" value=(t.kind); }))
                                    (field("Checklist", &format!("i{}", t.id), true, true, html! { textarea id=(format!("i{}", t.id)) name="items" required { (items_text(&t.items)) } }))
                                }
                                div class="form-actions" { button type="submit" class="btn primary" { "Save template" } }
                            }))
                        }
                    }
                    ol class="stack" {
                        @for (n, item) in t.items.iter().enumerate() {
                            @if starts_section(&t.items, n) { p class="muted small" { strong { (item.section) } } }
                            li { (item.text) }
                        }
                    }
                }
            }
        }
        @if can_edit {
            @let others: Vec<(String, &buildkiosk_engine::model::inspection::InspectionTemplate)> = state
                .inspection_templates
                .iter()
                .filter(|t| t.project != id && t.deleted.is_none() && view::level(state, req.actor(), t.project, Tool::Inspections) >= Level::Read)
                .filter_map(|t| state.projects.get(t.project).filter(|p| p.deleted.is_none()).map(|p| (p.name.clone(), t)))
                .collect();
            @if !others.is_empty() {
                div class="card" {
                    h2 { "Copy a template from another project" }
                    table class="data" {
                        tbody { @for (pname, t) in &others { tr {
                            td { (t.name) } td class="muted" { (pname) } td class="muted" { (t.items.len()) " lines" }
                            td class="right" { (button_form(req, "inspection_template.copy", &here, &[("project", id.to_string()), ("from_project", t.project.to_string()), ("id", t.id.to_string())], "Copy here", "small")) }
                        } } }
                    }
                }
            }
            div class="card" {
                h2 { "Add a template" }
                (command_form(req, "inspection_template.create", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Name", "tname", true, false, html! { input type="text" id="tname" name="name" required; }))
                        (field("Kind", "tkind", false, false, html! { input type="text" id="tkind" name="kind"; }))
                        (field("Checklist", "titems", true, true, html! { textarea id="titems" name="items" required placeholder="# Section heading\nOne checklist line per line" {} }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add template" } }
                }))
            }
        }
    };
    layout::app(req, "Inspection templates", Nav::Inspections(id), &crumbs(p, Some(("Templates", format!("/projects/{id}/inspections")))), body)
}

/// The inspections as CSV.
pub fn csv(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let day = |t: buildkiosk_core::Timestamp| t.date_at(state.utc_offset()).to_string();
    let mut out = String::from("Number,Title,Kind,Location,Status,Lines,Answered,Failed,Inspector,Manager,Scheduled,Created,Completed\r\n");
    for i in view::inspections_for(state, req.actor(), id) {
        let row = [
            i.number.to_string(),
            i.title.clone(),
            i.kind.clone(),
            i.location.clone(),
            i.status.label().to_string(),
            i.items.len().to_string(),
            i.answered().to_string(),
            i.failed().to_string(),
            i.inspector.map(|m| view::person_name(state, m)).unwrap_or_default(),
            i.manager.map(|m| view::person_name(state, m)).unwrap_or_default(),
            i.scheduled.map(|d| d.to_string()).unwrap_or_default(),
            day(i.created.at),
            i.completed.map(|c| day(c.at)).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}
