//! Action plan pages: list, the plan with its items, and the form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::action_plan::{can_read_templates, templates, visible};
use buildkiosk_engine::model::action_plan::{ActionPlan, PlanStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{human_size, not_found, project_for, user_options};

fn status_pill(s: PlanStatus) -> Markup {
    let class = match s {
        PlanStatus::Draft => "violet",
        PlanStatus::Active => "blue",
        PlanStatus::Complete => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn outline_of(a: &ActionPlan) -> String {
    a.sections
        .iter()
        .map(|s| format!("# {}\n{}", s.title, s.items.iter().map(|i| format!("- {}", i.text)).collect::<Vec<_>>().join("\n")))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn list(req: &Req<'_>, id: Id<Project>, status: Option<PlanStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::ActionPlans);
    if lvl < Level::Read {
        return not_found(req);
    }
    let needle = q.trim().to_lowercase();
    let shown: Vec<&ActionPlan> = visible(state, id)
        .into_iter()
        .filter(|a| status.is_none_or(|s| a.status == s))
        .filter(|a| {
            needle.is_empty()
                || a.title.to_lowercase().contains(&needle)
                || a.sections.iter().any(|s| s.items.iter().any(|i| i.text.to_lowercase().contains(&needle)))
        })
        .collect();
    let base = format!("/projects/{id}/action-plans");
    let body = html! {
        div class="page-head" {
            div { h1 { "Action plans" } div class="sub" { "Checklists of work in sections, with an owner and a date for each item" } }
            div class="actions" { @if lvl >= Level::Standard { a class="btn primary" href=(format!("{base}/new")) { "New action plan" } } }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search title or items" aria-label="Search action plans";
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in PlanStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::ActionPlans), "No action plans", "Write a plan to track a body of work item by item.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Plan" } th { "Manager" } th { "Progress" } th { "Status" } } }
                tbody { @for a in &shown { @let (done, all) = a.progress(); tr {
                    td { (a.number) }
                    td { a href=(format!("{base}/{}", a.id)) { (a.title) } }
                    td { (a.manager.map(|m| view::person_name(state, m)).unwrap_or_default()) }
                    td { (done) " of " (all) " items" }
                    td { (status_pill(a.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Action plans",
        Nav::ActionPlans(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Action plans", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, aid: Id<ActionPlan>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::ActionPlans);
    let Some(a) = state.action_plans.get(aid).filter(|a| a.project == id && a.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/action-plans");
    let here = format!("{base}/{aid}");
    let ids = [("project", id.to_string()), ("id", aid.to_string())];
    let me = req.actor().person();
    let manager = lvl == Level::Admin || (me.is_some() && (a.manager == me || a.created.by == me));
    let (done, all) = a.progress();
    let can_files = lvl >= Level::Standard && a.status != PlanStatus::Complete && req.allowed("action_plan.attach", Some(id));
    let today = req.today();
    let people = user_options(req, id, true);
    let body = html! {
        div class="page-head" {
            div { h1 { (a.label()) ": " (a.title) " " (status_pill(a.status)) } div class="sub" { (done) " of " (all) " items done · managed by " (a.manager.map(|m| view::person_name(state, m)).unwrap_or_else(|| "nobody".into())) } }
            div class="actions" {
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                @if manager && a.status != PlanStatus::Complete { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if manager && a.status == PlanStatus::Draft { (button_form(req, "action_plan.activate", &here, &ids, "Start the plan", "primary")) }
                @if manager && a.status == PlanStatus::Active { (button_form(req, "action_plan.complete", &here, &ids, "Mark complete", "primary")) }
                @if manager && a.status == PlanStatus::Complete { (button_form(req, "action_plan.reopen", &here, &ids, "Reopen", "")) }
                @if manager { (button_form(req, "action_plan.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        @if !a.description.is_empty() { div class="card" { p class="prose" { (a.description) } } }
        @if let Some(c) = a.completed { div class="card" { p { span class="pill green" { "Signed off" } " by " (c.by.map(|x| view::person_name(state, x)).unwrap_or_else(|| "the system".into())) " on " (c.at.date_at(state.utc_offset()).to_string()) } } }
        @if !a.approvers.is_empty() || (manager && a.status != PlanStatus::Complete) {
            div class="card" {
                h2 { "Approval" }
                @if a.approvers.is_empty() { p class="muted" { "No approvers named; the plan can start without approval." } } @else {
                    table class="data" {
                        thead { tr { th { "Approver" } th { "Answer" } th { "Note" } } }
                        tbody { @for who in &a.approvers { @let ans = a.approvals.iter().find(|x| x.person == *who); tr {
                            td { (view::person_name(state, *who)) }
                            td { @match ans { Some(x) if x.approved => { span class="pill green" { "Approved" } " " span class="muted small" { (x.stamp.at.date_at(state.utc_offset()).to_string()) } } Some(x) => { span class="pill red" { "Declined" } " " span class="muted small" { (x.stamp.at.date_at(state.utc_offset()).to_string()) } } None => { span class="pill" { "Waiting" } } } }
                            td { (ans.map(|x| x.note.as_str()).unwrap_or_default()) }
                        } } }
                    }
                }
                @if me.is_some_and(|m| a.approvers.contains(&m)) && a.status != PlanStatus::Complete {
                    (command_form(req, "action_plan.approve", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(aid);
                        div class="form-grid" { (field("Note", "ap_note", false, true, html! { input type="text" id="ap_note" name="note"; })) }
                        div class="form-actions" {
                            button type="submit" name="approved" value="false" class="btn" { "Decline" }
                            button type="submit" name="approved" value="true" class="btn primary" { "Approve" }
                        }
                    }))
                }
                @if manager && a.status != PlanStatus::Complete {
                    details class="disclosure" {
                        summary { "Choose approvers" }
                        (command_form(req, "action_plan.set_approvers", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(aid);
                            input type="hidden" name="approvers[]" value="";
                            div class="options" { @for x in &people { label { input type="checkbox" name="approvers[]" value=(x.id) checked[a.approvers.contains(&x.id)]; (x.name()) } } }
                            p class="muted small" { "The plan cannot start until every approver has approved. Changing the wording of items clears the answers." }
                            div class="form-actions" { button type="submit" class="btn primary small" { "Save approvers" } }
                        }))
                    }
                }
            }
        }
        @if lvl >= Level::Standard && !a.sections.is_empty() && req.allowed("action_plan.template_save", Some(id)) {
            div class="card" {
                details class="disclosure" {
                    summary { "Save as a template" }
                    (command_form(req, "action_plan.template_save", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(aid);
                        div class="form-grid" { (field("Template name", "tpl_name", true, false, html! { input type="text" id="tpl_name" name="name" required value=(a.title); })) }
                        p class="muted small" { "Keeps the sections and items for any project to start from, without owners, dates or progress." }
                        div class="form-actions" { button type="submit" class="btn primary small" { "Save template" } }
                    }))
                }
            }
        }
        div class="card" {
            h2 { "Files" }
            @if a.attachments.is_empty() { p class="muted" { "No files attached." } } @else {
                ul class="plain" { @for f in &a.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if can_files { " " (button_form(req, "action_plan.detach", &here, &[("project", id.to_string()), ("id", aid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if can_files {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
        @if a.sections.is_empty() { div class="card" { (empty(tool_icon(Tool::ActionPlans), "No items", "Edit the plan to add sections and items.")) } }
        @for (si, s) in a.sections.iter().enumerate() {
            div class="card" {
                h2 { (s.title) }
                table class="data" {
                    thead { tr { th { "Item" } th { "Owner" } th { "Due" } th { "Done" } @if manager { th {} } } }
                    tbody { @for (ii, i) in s.items.iter().enumerate() { tr {
                        td { span class=(if i.done { "muted" } else { "" }) { (i.text) } @if !i.note.is_empty() { br; span class="muted small" { (i.note) } }
                            @if !i.comments.is_empty() || (a.status != PlanStatus::Complete && (manager || lvl >= Level::Standard || (i.assignee.is_some() && i.assignee == me))) {
                                details class="disclosure" {
                                    summary { "Comments (" (i.comments.len()) ")" }
                                    @for c in &i.comments { p class="small" { strong { (c.stamp.by.map(|x| view::person_name(state, x)).unwrap_or_default()) } " " span class="muted" { (c.stamp.at.date_at(state.utc_offset()).to_string()) } br; (c.text) } }
                                    @if a.status != PlanStatus::Complete && (manager || lvl >= Level::Standard || (i.assignee.is_some() && i.assignee == me)) {
                                        (command_form(req, "action_plan.comment", &here, &here, html! {
                                            input type="hidden" name="project" value=(id);
                                            input type="hidden" name="id" value=(aid);
                                            input type="hidden" name="section" value=(si);
                                            input type="hidden" name="item" value=(ii);
                                            input type="text" name="text" required aria-label="Comment" placeholder="Add a comment";
                                            button type="submit" class="btn small" { "Post" }
                                        }))
                                    }
                                }
                            }
                        }
                        td { (i.assignee.map(|x| view::person_name(state, x)).unwrap_or_default()) }
                        td class="nowrap" { @if let Some(d) = i.due { @if d < today && !i.done { span class="overdue" { (d.to_string()) } } @else { (d.to_string()) } } }
                        td {
                            @if req.allowed("action_plan.set_item", Some(id)) && a.status != PlanStatus::Complete && (manager || (i.assignee.is_some() && i.assignee == me)) {
                                (button_form(req, "action_plan.set_item", &here, &[("project", id.to_string()), ("id", aid.to_string()), ("section", si.to_string()), ("item", ii.to_string()), ("done", (!i.done).to_string())], if i.done { "Done · undo" } else { "Mark done" }, "small"))
                            } @else if i.done { span class="pill green" { "Done" } } @else { span class="pill" { "Open" } }
                        }
                        @if manager && a.status != PlanStatus::Complete { td {
                            details class="disclosure" {
                                summary { "Assign" }
                                (command_form(req, "action_plan.set_item", &here, &here, html! {
                                    input type="hidden" name="project" value=(id);
                                    input type="hidden" name="id" value=(aid);
                                    input type="hidden" name="section" value=(si);
                                    input type="hidden" name="item" value=(ii);
                                    div class="form-grid" {
                                        (field("Owner", &format!("as_{si}_{ii}"), false, false, html! { select id=(format!("as_{si}_{ii}")) name="assignee" {
                                            option value="" { "—" }
                                            @for x in &people { option value=(x.id) selected[i.assignee == Some(x.id)] { (x.name()) } }
                                        } }))
                                        (field("Due", &format!("du_{si}_{ii}"), false, false, html! { input type="date" id=(format!("du_{si}_{ii}")) name="due" value=(i.due.map(|d| d.to_string()).unwrap_or_default()); }))
                                        (field("Note", &format!("no_{si}_{ii}"), false, true, html! { input type="text" id=(format!("no_{si}_{ii}")) name="note" value=(i.note); }))
                                    }
                                    div class="form-actions" { button type="submit" class="btn primary small" { "Save" } }
                                }))
                            }
                        } }
                    } } }
                }
            }
        }
    };
    layout::app(
        req,
        &a.label(),
        Nav::ActionPlans(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Action plans", Some(base)), (&a.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<ActionPlan>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.action_plans.get(c).filter(|a| a.project == id && a.deleted.is_none()) {
            Some(a) => Some(a),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "action_plan.update" } else { "action_plan.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/action-plans");
    let back = cur.map(|a| format!("{base}/{}/edit", a.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|a| format!("{base}/{}", a.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let people = user_options(req, id, true);
    let manager = cur.and_then(|a| a.manager).or(Some(req.person));
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit action plan" } else { "New action plan" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(a) = cur { input type="hidden" name="id" value=(a.id); }
                div class="form-grid" {
                    (field("Title", "title", true, false, html! { input type="text" id="title" name="title" required value=(cur.map(|a| a.title.as_str()).unwrap_or_default()); }))
                    (field("Manager", "manager", false, false, html! { select id="manager" name="manager" { @for x in &people { option value=(x.id) selected[manager == Some(x.id)] { (x.name()) } } } }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (cur.map(|a| a.description.as_str()).unwrap_or_default()) } }))
                    @if cur.is_none() && can_read_templates(state, req.actor()) && !templates(state).is_empty() {
                        (field("Start from a template", "template", false, false, html! { select id="template" name="template" {
                            option value="" { "Blank (use the outline below)" }
                            @for t in templates(state) { option value=(t.id) { (t.name) } }
                        } }))
                    }
                    (field("Sections and items", "outline", false, true, html! { textarea id="outline" name="outline" rows="12" placeholder="# Documents\n- Collect warranties\n- Record drawings\n# Site\n- Final clean" { (cur.map(outline_of).unwrap_or_default()) } }))
                }
                p class="muted small" { "A line starting with # begins a section; every other line is an item. Assign owners and dates on the plan page. Editing the outline keeps the owner, date, note and done state of items whose wording and section are unchanged." }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|a| format!("{base}/{}", a.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Action plan",
        Nav::ActionPlans(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Action plans", Some(base)), ("Action plan", None)],
        body,
    )
}
