//! Action plans: a checklist of work in sections. The manager writes it, assigns the items and
//! activates it (which tells the people with items); owners tick items off; the manager completes
//! it once every item is done.
//!
//! The outline is plain text: a line starting with `#` begins a section, and each other non-blank
//! line (a leading `-` or `*` is dropped) is an item of the section above it.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::action_plan::{ActionPlan, PlanItem, PlanSection, PlanStatus};
use buildkiosk_model::directory::Person;
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "action_plan.create",
        "Create action plan",
        Tool::ActionPlans,
        Access::Project(Level::Standard),
        "project: project id; title: text; description?: text; manager?: person id (default you); outline?: text (`# Section` lines, then one item per line)",
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
                s.items.push(PlanItem { text: item, assignee: None, due: None, done: false, note: String::new() });
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
    let sections = if p.present("outline") { parse_outline(p, &p.text_or_empty("outline", text::LONG)?)? } else { Vec::new() };
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
        a.sections = merge(&a.sections, new);
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
