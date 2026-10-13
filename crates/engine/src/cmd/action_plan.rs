//! Action plans: a checklist of work in sections. The manager writes it, assigns the items and
//! activates it (which tells the people with items); owners tick items off; the manager completes
//! it once every item is done.
//!
//! The outline is plain text: a line starting with `#` begins a section, and each other non-blank
//! line (a leading `-` or `*` is dropped) is an item of the section above it.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::action_plan::{ActionPlan, PlanApproval, PlanComment, PlanItem, PlanSection, PlanStatus, PlanTemplate};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, Level, Tool, perm};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "action_plan.create",
        "Create action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project: project id; title: text; description?: text; manager?: person id (default you); outline?: text (`# Section` lines, then one item per line); template?: plan template id (used when no outline is given)",
        create,
    ),
    CommandSpec::mutate(
        "action_plan.update",
        "Edit action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids; title?, description?, manager?, outline? (a new outline keeps the assignee, date, note and done state of items whose text and section are unchanged)",
        update,
    ),
    CommandSpec::mutate(
        "action_plan.set_item",
        "Update plan item",
        Tool::ActionPlans,
        Access::Project(Level::Read),
        "project, id: ids; section: number from 0; item: number from 0; done?: bool (its assignee, the manager or an admin); assignee?: person id, due?: YYYY-MM-DD, note?: text (the manager or an admin)",
        set_item,
    ),
    CommandSpec::mutate(
        "action_plan.activate",
        "Start action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids (the manager or an admin)",
        activate,
    ),
    CommandSpec::mutate(
        "action_plan.complete",
        "Complete action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids (every item must be done)",
        complete,
    ),
    CommandSpec::mutate(
        "action_plan.reopen",
        "Reopen action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids (the manager or an admin)",
        reopen,
    ),
    CommandSpec::mutate(
        "action_plan.delete",
        "Delete action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids (the manager or an admin)",
        delete,
    ),
    CommandSpec::mutate("action_plan.restore", "Restore action plan", Tool::ActionPlans, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "action_plan.list",
        "List action plans",
        Tool::ActionPlans,
        Access::Project(Level::Read),
        "project: project id; status?: draft|active|complete; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::mutate(
        "action_plan.set_approvers",
        "Set plan approvers",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids; approvers: person ids (the manager or an admin; empty means no approval is needed; people taken off lose their answer)",
        set_approvers,
    ),
    CommandSpec::mutate(
        "action_plan.approve",
        "Approve or decline action plan",
        Tool::ActionPlans,
        Access::Project(Level::Read),
        "project, id: ids; approved?: bool (default true); note?: text (only a named approver; the plan cannot start until all approve)",
        approve,
    ),
    CommandSpec::mutate(
        "action_plan.comment",
        "Comment on plan item",
        Tool::ActionPlans,
        Access::Project(Level::Read),
        "project, id: ids; section, item: numbers from 0; text: remark (the item's owner, the manager, or anyone with Standard access)",
        comment,
    ),
    CommandSpec::mutate(
        "action_plan.template_save",
        "Save plan as template",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project, id: ids of the plan; name: text (a company-wide template of the sections and items, without owners, dates or progress; a template of the same name is replaced)",
        template_save,
    ),
    CommandSpec::mutate(
        "action_plan.template_delete",
        "Delete plan template",
        Tool::ActionPlans,
        Access::Member,
        "id: template id (its author or a company admin)",
        template_delete,
    ),
    CommandSpec::query("action_plan.template_list", "List plan templates", Tool::ActionPlans, Access::Member, "(none)", template_list),
    CommandSpec::query("action_plan.get", "Show action plan", Tool::ActionPlans, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_SECTIONS: usize = 50;
const MAX_ITEMS: usize = 500;

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<ActionPlan> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ActionPlan> = p.id("id")?;
    let a = in_project(&ctx.state.action_plans, project, id, "action plan")?;
    if a.deleted.is_some() {
        return Err(EngineError::not_found(format!("action plan {id} not found in this project")));
    }
    Ok(a.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::ActionPlans) == Level::Admin
}

fn is_manager(ctx: &Ctx<'_>, a: &ActionPlan) -> bool {
    is_admin(ctx, a.project) || (ctx.person().is_some() && (a.manager == ctx.person() || a.created.by == ctx.person()))
}

/// Reads an outline into sections; items before any `#` line go in a section called "Items".
fn parse_outline(p: &Params<'_>, raw: &str) -> Result<Vec<PlanSection>> {
    let mut out: Vec<PlanSection> = Vec::new();
    let mut items = 0usize;
    for (n, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(title) = line.strip_prefix('#') {
            let title =
                text::line(title.trim_start_matches('#').trim(), text::SHORT).map_err(|e| p.bad("outline", format!("line {}: {e}", n + 1)))?;
            if title.is_empty() {
                return Err(p.bad("outline", format!("line {}: a section needs a title", n + 1)));
            }
            out.push(PlanSection { title, items: Vec::new() });
        } else {
            let item =
                text::line(line.trim_start_matches(['-', '*']).trim(), text::SHORT).map_err(|e| p.bad("outline", format!("line {}: {e}", n + 1)))?;
            if item.is_empty() {
                continue;
            }
            if out.is_empty() {
                out.push(PlanSection { title: "Items".into(), items: Vec::new() });
            }
            items += 1;
            if items > MAX_ITEMS {
                return Err(p.bad("outline", format!("has more than {MAX_ITEMS} items")));
            }
            if let Some(s) = out.last_mut() {
                s.items.push(PlanItem { text: item, assignee: None, due: None, done: false, note: String::new(), comments: Vec::new() });
            }
        }
        if out.len() > MAX_SECTIONS {
            return Err(p.bad("outline", format!("has more than {MAX_SECTIONS} sections")));
        }
    }
    Ok(out)
}

/// A new outline keeps what was recorded on items that survive unchanged.
fn merge(old: &[PlanSection], mut new: Vec<PlanSection>) -> Vec<PlanSection> {
    for s in &mut new {
        if let Some(o) = old.iter().find(|o| o.title.eq_ignore_ascii_case(&s.title)) {
            for i in &mut s.items {
                if let Some(prev) = o.items.iter().find(|x| x.text.eq_ignore_ascii_case(&i.text)) {
                    *i = prev.clone();
                }
            }
        }
    }
    new
}

/// The wording of every section and item, which is what an approver agreed to.
fn shape(sections: &[PlanSection]) -> Vec<(&str, Vec<&str>)> {
    sections.iter().map(|s| (s.title.as_str(), s.items.iter().map(|i| i.text.as_str()).collect())).collect()
}

fn manager(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Option<Id<Person>>> {
    let m: Option<Id<Person>> = p.opt_id("manager")?;
    if let Some(m) = m {
        ctx.check_person(m)?;
    }
    Ok(m)
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = ctx.state.action_plans.iter().filter(|a| a.project == project).map(|a| a.number).max().unwrap_or(0).saturating_add(1);
    let sections = if p.present("outline") {
        parse_outline(p, &p.text_or_empty("outline", text::LONG)?)?
    } else if let Some(t) = p.opt_id::<PlanTemplate>("template")? {
        let t = ctx
            .state
            .plan_templates
            .get(t)
            .filter(|t| t.deleted.is_none())
            .ok_or_else(|| p.bad("template", format!("plan template {t} not found")))?;
        t.sections.clone()
    } else {
        Vec::new()
    };
    let id = ctx.state.action_plans.alloc();
    let stamp = ctx.stamp();
    ctx.state.action_plans.put(ActionPlan {
        id,
        project,
        number,
        title: p.req_line("title", text::SHORT)?,
        description: p.text_or_empty("description", text::LONG)?,
        status: PlanStatus::Draft,
        manager: manager(ctx, p)?.or(ctx.person()),
        sections,
        approvers: Vec::new(),
        approvals: Vec::new(),
        completed: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden(format!("only the manager or an admin can edit {}", a.label())));
    }
    if a.status == PlanStatus::Complete {
        return Err(EngineError::conflict(format!("{} is complete; reopen it to change it", a.label())));
    }
    if p.present("title") {
        a.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        a.description = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("manager") {
        a.manager = manager(ctx, p)?;
    }
    if p.present("outline") {
        let new = parse_outline(p, &p.text_or_empty("outline", text::LONG)?)?;
        let changed = shape(&a.sections) != shape(&new);
        a.sections = merge(&a.sections, new);
        if changed {
            a.approvals.clear();
        }
    }
    a.updated = ctx.stamp();
    let id = a.id;
    ctx.state.action_plans.put(a);
    Ok(json!({ "id": id }))
}

fn link(a: &ActionPlan) -> String {
    format!("/projects/{}/action-plans/{}", a.project, a.id)
}

fn set_item(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    let si = usize::try_from(p.int("section", 0..=1_000)?.ok_or_else(|| p.bad("section", "is required"))?).unwrap_or(usize::MAX);
    let ii = usize::try_from(p.int("item", 0..=10_000)?.ok_or_else(|| p.bad("item", "is required"))?).unwrap_or(usize::MAX);
    let manager = is_manager(ctx, &a);
    let me = ctx.person();
    let label = a.label();
    let Some(item) = a.sections.get_mut(si).and_then(|s| s.items.get_mut(ii)) else {
        return Err(EngineError::not_found(format!("{label} has no such item")));
    };
    if p.present("assignee") || p.present("due") || p.present("note") {
        if !manager {
            return Err(EngineError::forbidden("only the manager or an admin can assign items, set dates or add notes"));
        }
        if p.present("assignee") {
            let who: Option<Id<Person>> = p.opt_id("assignee")?;
            if let Some(w) = who {
                ctx.check_person(w)?;
            }
            item.assignee = who;
        }
        if p.present("due") {
            let d: Option<Date> = p.date("due")?;
            item.due = d;
        }
        if p.present("note") {
            item.note = p.line_or_empty("note", text::SHORT)?;
        }
    }
    if let Some(done) = p.flag("done")? {
        if !(manager || (item.assignee.is_some() && item.assignee == me)) {
            return Err(EngineError::forbidden("only the person it is assigned to, the manager or an admin can tick an item"));
        }
        item.done = done;
    }
    if a.status == PlanStatus::Complete {
        return Err(EngineError::conflict(format!("{label} is complete; reopen it to change it")));
    }
    a.updated = ctx.stamp();
    let id = a.id;
    ctx.state.action_plans.put(a);
    Ok(json!({ "id": id }))
}

fn activate(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden("only the manager or an admin can start the plan"));
    }
    if a.status != PlanStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already started", a.label())));
    }
    if a.progress().1 == 0 {
        return Err(EngineError::conflict("add at least one item first"));
    }
    if !a.approved() {
        let waiting = a.approvers.iter().filter(|x| !a.approvals.iter().any(|y| y.person == **x && y.approved)).count();
        return Err(EngineError::conflict(format!("{waiting} approver(s) have not approved {} yet", a.label())));
    }
    a.status = PlanStatus::Active;
    a.updated = ctx.stamp();
    let mut people: Vec<Id<Person>> = a.sections.iter().flat_map(|s| s.items.iter()).filter_map(|i| i.assignee).collect();
    people.sort();
    people.dedup();
    let (project, l, t) = (a.project, link(&a), format!("{} has started and you have items: {}", a.label(), a.title));
    let id = a.id;
    ctx.state.action_plans.put(a);
    notify(ctx, &people, project, &t, &l);
    Ok(json!({ "id": id }))
}

fn complete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden("only the manager or an admin can complete the plan"));
    }
    if a.status != PlanStatus::Active {
        return Err(EngineError::conflict(format!("{} is {}", a.label(), a.status)));
    }
    let (done, all) = a.progress();
    if done < all {
        return Err(EngineError::conflict(format!("{} of {all} items are still open", all - done)));
    }
    a.status = PlanStatus::Complete;
    a.completed = Some(ctx.stamp());
    a.updated = ctx.stamp();
    let id = a.id;
    ctx.state.action_plans.put(a);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden("only the manager or an admin can reopen the plan"));
    }
    if a.status != PlanStatus::Complete {
        return Err(EngineError::conflict(format!("{} is not complete", a.label())));
    }
    a.status = PlanStatus::Active;
    a.completed = None;
    a.updated = ctx.stamp();
    let id = a.id;
    ctx.state.action_plans.put(a);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden("only the manager or an admin can delete the plan"));
    }
    let stamp = ctx.stamp();
    ctx.state.action_plans.update(a.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": a.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ActionPlan> = p.id("id")?;
    let a = in_project(&ctx.state.action_plans, project, id, "action plan")?;
    if a.deleted.is_none() {
        return Err(EngineError::conflict("the plan is not in the recycle bin"));
    }
    ctx.state.action_plans.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn set_approvers(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    if !is_manager(ctx, &a) {
        return Err(EngineError::forbidden("only the manager or an admin can name approvers"));
    }
    if a.status == PlanStatus::Complete {
        return Err(EngineError::conflict(format!("{} is complete; reopen it to change it", a.label())));
    }
    let mut who: Vec<Id<Person>> = p.ids("approvers")?;
    who.sort();
    who.dedup();
    if who.len() > 20 {
        return Err(p.bad("approvers", "names more than 20 people"));
    }
    for w in &who {
        ctx.check_person(*w)?;
    }
    let added: Vec<Id<Person>> = who.iter().copied().filter(|w| !a.approvers.contains(w)).collect();
    a.approvals.retain(|x| who.contains(&x.person));
    a.approvers = who;
    a.updated = ctx.stamp();
    let (project, l, t, id) = (a.project, link(&a), format!("You are asked to approve {}: {}", a.label(), a.title), a.id);
    ctx.state.action_plans.put(a);
    notify(ctx, &added, project, &t, &l);
    Ok(json!({ "id": id }))
}

fn approve(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    let Some(me) = ctx.person().filter(|m| a.approvers.contains(m)) else {
        return Err(EngineError::forbidden("only a named approver can answer"));
    };
    if a.status == PlanStatus::Complete {
        return Err(EngineError::conflict(format!("{} is complete", a.label())));
    }
    let approved = p.flag("approved")?.unwrap_or(true);
    let note = p.line_or_empty("note", text::SHORT)?;
    let stamp = ctx.stamp();
    a.approvals.retain(|x| x.person != me);
    a.approvals.push(PlanApproval { person: me, approved, note, stamp });
    a.updated = stamp;
    let (project, l, id) = (a.project, link(&a), a.id);
    let t = format!("{} {} {}: {}", view::person_name(ctx.state, me), if approved { "approved" } else { "declined" }, a.label(), a.title);
    let to: Vec<Id<Person>> = a.manager.into_iter().collect();
    ctx.state.action_plans.put(a);
    notify(ctx, &to, project, &t, &l);
    Ok(json!({ "id": id, "approved": approved }))
}

fn comment(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut a = load(ctx, p)?;
    let si = usize::try_from(p.int("section", 0..=1_000)?.ok_or_else(|| p.bad("section", "is required"))?).unwrap_or(usize::MAX);
    let ii = usize::try_from(p.int("item", 0..=10_000)?.ok_or_else(|| p.bad("item", "is required"))?).unwrap_or(usize::MAX);
    let body = p.req_text("text", text::SHORT)?;
    let manager = is_manager(ctx, &a);
    let standard = ctx.level(a.project, Tool::ActionPlans) >= Level::Standard;
    let me = ctx.person();
    let stamp = ctx.stamp();
    let label = a.label();
    let Some(item) = a.sections.get_mut(si).and_then(|s| s.items.get_mut(ii)) else {
        return Err(EngineError::not_found(format!("{label} has no such item")));
    };
    if !(manager || standard || (item.assignee.is_some() && item.assignee == me)) {
        return Err(EngineError::forbidden("only the item's owner, the manager or people with Standard access can comment"));
    }
    if item.comments.len() >= 200 {
        return Err(EngineError::conflict("the item has 200 comments already"));
    }
    item.comments.push(PlanComment { text: body, stamp });
    let (owner, text_item) = (item.assignee, item.text.clone());
    a.updated = stamp;
    let (project, l, id) = (a.project, link(&a), a.id);
    let mut to: Vec<Id<Person>> = owner.into_iter().chain(a.manager).filter(|x| Some(*x) != me).collect();
    to.sort();
    to.dedup();
    let t = format!("New comment on \"{text_item}\" in {label}");
    ctx.state.action_plans.put(a);
    notify(ctx, &to, project, &t, &l);
    Ok(json!({ "id": id }))
}

/// Whether the person may look at plan templates: company admins and anyone who can read action plans on a project.
pub fn can_read_templates(state: &CompanyState, actor: Actor) -> bool {
    match actor {
        Actor::System => true,
        Actor::Person(p) => {
            perm::company_level(state, p, Tool::CompanyAdmin) == Level::Admin
                || perm::visible_projects(state, p).into_iter().any(|id| perm::project_level(state, p, id, Tool::ActionPlans) >= Level::Read)
        }
    }
}

fn template_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let a = load(ctx, p)?;
    let name = p.req_line("name", text::SHORT)?;
    let sections: Vec<PlanSection> = a
        .sections
        .iter()
        .map(|s| PlanSection {
            title: s.title.clone(),
            items: s
                .items
                .iter()
                .map(|i| PlanItem { text: i.text.clone(), assignee: None, due: None, done: false, note: String::new(), comments: Vec::new() })
                .collect(),
        })
        .collect();
    if sections.is_empty() {
        return Err(EngineError::conflict("the plan has no items to keep"));
    }
    let stamp = ctx.stamp();
    let existing = ctx.state.plan_templates.iter().find(|t| t.deleted.is_none() && t.name.eq_ignore_ascii_case(&name)).map(|t| t.id);
    let id = match existing {
        Some(id) => {
            ctx.state.plan_templates.update(id, |t| {
                t.description = a.description.clone();
                t.sections = sections;
            });
            id
        }
        None => {
            if ctx.state.plan_templates.iter().filter(|t| t.deleted.is_none()).count() >= 500 {
                return Err(EngineError::conflict("the company keeps at most 500 plan templates"));
            }
            let id = ctx.state.plan_templates.alloc();
            ctx.state.plan_templates.put(PlanTemplate { id, name, description: a.description.clone(), sections, created: stamp, deleted: None });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn template_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<PlanTemplate> = p.id("id")?;
    let Some(t) = ctx.state.plan_templates.get(id).filter(|t| t.deleted.is_none()) else {
        return Err(EngineError::not_found(format!("plan template {id} not found")));
    };
    let admin = match ctx.actor {
        Actor::System => true,
        Actor::Person(x) => perm::company_level(ctx.state, x, Tool::CompanyAdmin) == Level::Admin,
    };
    if !admin && !(ctx.person().is_some() && t.created.by == ctx.person()) {
        return Err(EngineError::forbidden("only the template's author or a company admin can delete it"));
    }
    let stamp = ctx.stamp();
    ctx.state.plan_templates.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// The company's plan templates, by name.
pub fn templates(state: &CompanyState) -> Vec<&PlanTemplate> {
    let mut v: Vec<&PlanTemplate> = state.plan_templates.iter().filter(|t| t.deleted.is_none()).collect();
    v.sort_by_key(|a| a.name.to_lowercase());
    v
}

fn template_list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    if !can_read_templates(v.state, v.actor) {
        return Err(EngineError::forbidden("plan templates need access to action plans on a project"));
    }
    json(&templates(v.state))
}

/// The project's plans, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&ActionPlan> {
    let mut v: Vec<&ActionPlan> = state.action_plans.iter().filter(|a| a.project == project && a.deleted.is_none()).collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::ActionPlans) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&ActionPlan> = v.state.action_plans.iter().filter(|a| a.project == project && a.deleted.is_some()).collect();
        return json(&bin);
    }
    let status = p.choice::<PlanStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&ActionPlan> = visible(v.state, project)
        .into_iter()
        .filter(|a| status.is_none_or(|s| a.status == s))
        .filter(|a| {
            q.as_ref().is_none_or(|q| {
                a.title.to_lowercase().contains(q.as_str())
                    || a.sections.iter().any(|s| s.items.iter().any(|i| i.text.to_lowercase().contains(q.as_str())))
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<ActionPlan> = p.id("id")?;
    let a = in_project(&v.state.action_plans, project, id, "action plan")?;
    if a.deleted.is_some() {
        return Err(EngineError::not_found(format!("action plan {id} not found in this project")));
    }
    json(a)
}
