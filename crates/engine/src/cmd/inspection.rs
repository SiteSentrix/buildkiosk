//! Inspections: checklists walked on site and answered item by item.
//!
//! Templates are a project's reusable checklists (written by its people, or the starter set from
//! `inspection_template.install_starters`). Starting an inspection copies a template's lines into
//! it, so changing a template later never rewrites an inspection. The inspector answers each line
//! (pass, fail or not applicable) and completes the inspection once every line is answered; failed
//! lines can be turned into punch items.
//!
//! Who may do what (beyond the tool level checked before the command runs):
//! - answer, complete, attach files: the inspector, the manager, admins (while it is in progress);
//! - edit, reopen, raise punch items: the manager or an inspections admin;
//! - delete: admins, or the author of an inspection with no answers yet; restore: admins.

use buildkiosk_core::{Id, text};
use buildkiosk_model::Choice;
use buildkiosk_model::directory::Person;
use buildkiosk_model::inspection::{Answer, ChecklistItem, Inspection, InspectionStatus, InspectionTemplate, TemplateItem};
use buildkiosk_model::project::Project;
use buildkiosk_model::punch::PunchItem;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::submittal::file_ref;
use super::{Access, CommandSpec};
use crate::ctx::{Read, in_project};
use crate::view::{self, json};
use crate::workitem::{self, WorkItem};
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "inspection_template.create",
        "Add inspection template",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project: project id; name: text; kind?: text; items: text, one checklist line per line; a line starting with `# ` begins a new section",
        template_create,
    ),
    CommandSpec::mutate(
        "inspection_template.delete",
        "Delete inspection template",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project: project id; id: template id",
        template_delete,
    ),
    CommandSpec::mutate(
        "inspection_template.install_starters",
        "Add starter inspection templates",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project: project id (adds a small set of general checklists; ones already there are skipped)",
        install_starters,
    ),
    CommandSpec::query(
        "inspection_template.list",
        "List inspection templates",
        Tool::Inspections,
        Access::Project(Level::Read),
        "project: project id",
        template_list,
    ),
    CommandSpec::mutate(
        "inspection.create",
        "Start inspection",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project: project id; template?: template id (copies its checklist); items?: text, checklist lines when no template is used; title?: text (default: the template's name); \
         kind?, location?: text; inspector?: person id; manager?: person id (default: you); scheduled?: YYYY-MM-DD",
        create,
    ),
    CommandSpec::mutate(
        "inspection.update",
        "Edit inspection",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids; title?, kind?, location?: text; inspector?, manager?: person id; scheduled?: YYYY-MM-DD",
        update,
    ),
    CommandSpec::mutate(
        "inspection.answer",
        "Answer inspection checklist",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids; answers: [{line: line number from 1, answer: pass|fail|na|(blank to clear), note?: text}]",
        answer,
    ),
    CommandSpec::mutate(
        "inspection.complete",
        "Complete inspection",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids (every line must be answered)",
        complete,
    ),
    CommandSpec::mutate("inspection.reopen", "Reopen inspection", Tool::Inspections, Access::Project(Level::Standard), "project, id: ids", reopen),
    CommandSpec::mutate(
        "inspection.raise_punch",
        "Raise punch items for failed lines",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids; assignees?: [person ids] (a punch item is made for each failed line that has none yet)",
        raise_punch,
    ),
    CommandSpec::mutate(
        "inspection.attach",
        "Attach file to inspection",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text (of an uploaded file); name: text; size: integer (bytes); mime?: text",
        attach,
    ),
    CommandSpec::mutate(
        "inspection.detach",
        "Remove attachment from inspection",
        Tool::Inspections,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: text",
        detach,
    ),
    CommandSpec::mutate("inspection.delete", "Delete inspection", Tool::Inspections, Access::Project(Level::Standard), "project, id: ids", delete),
    CommandSpec::mutate("inspection.restore", "Restore inspection", Tool::Inspections, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "inspection.list",
        "List inspections",
        Tool::Inspections,
        Access::Project(Level::Read),
        "project: project id; status?: open|completed; q?: text; mine?: bool (I am the inspector); overdue?: bool; deleted?: bool (recycle bin, admins)",
        list,
    ),
    CommandSpec::query("inspection.get", "Show inspection", Tool::Inspections, Access::Project(Level::Read), "project, id: ids", get),
];

/// The general checklists `install_starters` adds. Written for BuildKiosk; edit or replace freely.
/// (name, kind, [(section, line)]).
type Starter = (&'static str, &'static str, &'static [(&'static str, &'static str)]);

const STARTERS: &[Starter] = &[
    (
        "Pre-pour concrete check",
        "Concrete",
        &[
            ("Formwork and reinforcement", "Forms are clean, braced and match the drawings"),
            ("Formwork and reinforcement", "Reinforcement size, spacing and cover match the drawings"),
            ("Formwork and reinforcement", "Embeds, sleeves and penetrations are in place and secured"),
            ("Site and conditions", "Subgrade is firm and free of standing water and debris"),
            ("Site and conditions", "Weather and curing plan are confirmed"),
            ("Site and conditions", "Mix, delivery schedule and pump are confirmed with the supplier"),
            ("Site and conditions", "Required testing and special inspection are arranged"),
        ],
    ),
    (
        "Framing walk-through",
        "Framing",
        &[
            ("Structure", "Members are the specified size and spacing"),
            ("Structure", "Connections and hangers use the right fasteners"),
            ("Structure", "Blocking and backing are in place for fixtures and casework"),
            ("Structure", "Openings are framed to the drawn sizes"),
            ("Before closing in", "Mechanical, electrical and plumbing rough-in is complete and visible"),
            ("Before closing in", "Fire blocking and draft stopping are installed"),
            ("Before closing in", "Framing is plumb, level and dry"),
        ],
    ),
    (
        "Site safety walk-through",
        "Safety",
        &[
            ("Site", "Walkways and access routes are clear"),
            ("Site", "Floor openings are guarded or covered, and open edges protected"),
            ("Equipment and materials", "Ladders and scaffolds are in good condition and used correctly"),
            ("Equipment and materials", "Materials are stacked stably and out of the way"),
            ("Equipment and materials", "Fire extinguishers are accessible and first aid kits stocked"),
            ("People", "Workers wear the protective gear the task needs"),
            ("People", "Hazardous materials are labeled and stored properly"),
        ],
    ),
    (
        "Final walk-through",
        "Closeout",
        &[
            ("Finishes", "Paint, flooring and trim are complete and undamaged"),
            ("Finishes", "Doors and hardware operate and latch"),
            ("Systems", "Lights, outlets and switches work"),
            ("Systems", "Plumbing fixtures run without leaks"),
            ("Systems", "Heating and cooling operate"),
            ("Cleanup", "Debris is removed and surfaces are cleaned"),
            ("Cleanup", "Keys, manuals and warranty papers are ready for handover"),
        ],
    ),
];

/// Most lines one checklist can have.
const MAX_LINES: usize = 300;
const MAX_ATTACHMENTS: usize = 50;

/// Reads checklist text: one line per item, `# ` starts a section.
fn parse_items(p: &Params<'_>, key: &str, body: &str) -> Result<Vec<TemplateItem>> {
    let mut section = String::new();
    let mut out = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix("# ").or_else(|| line.strip_prefix('#')) {
            section = text::line(h.trim(), 120).map_err(|e| p.bad(key, format!("line {}: {e}", n + 1)))?;
            continue;
        }
        if out.len() >= MAX_LINES {
            return Err(p.bad(key, format!("has more than {MAX_LINES} lines")));
        }
        out.push(TemplateItem {
            section: section.clone(),
            text: text::required_line(line, text::SHORT).map_err(|e| p.bad(key, format!("line {}: {e}", n + 1)))?,
        });
    }
    Ok(out)
}

fn template_exists(ctx: &Ctx<'_>, project: Id<Project>, name: &str) -> bool {
    ctx.state.inspection_templates.iter().any(|t| t.project == project && t.deleted.is_none() && t.name.eq_ignore_ascii_case(name))
}

fn add_template(ctx: &mut Ctx<'_>, project: Id<Project>, name: String, kind: String, items: Vec<TemplateItem>) -> Id<InspectionTemplate> {
    let id = ctx.state.inspection_templates.alloc();
    let created = ctx.stamp();
    ctx.state.inspection_templates.put(InspectionTemplate { id, project, name, kind, items, created, deleted: None });
    id
}

fn template_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", text::SHORT)?;
    if template_exists(ctx, project, &name) {
        return Err(EngineError::conflict(format!("there is already a template called {name}")));
    }
    let items = parse_items(p, "items", &p.req_text("items", 100_000)?)?;
    if items.is_empty() {
        return Err(p.bad("items", "needs at least one checklist line"));
    }
    let id = add_template(ctx, project, name, p.line_or_empty("kind", 100)?, items);
    Ok(json!({ "id": id }))
}

fn template_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<InspectionTemplate> = p.id("id")?;
    in_project(&ctx.state.inspection_templates, project, id, "inspection template")?;
    let stamp = ctx.stamp();
    ctx.state.inspection_templates.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn install_starters(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut added = 0usize;
    for (name, kind, lines) in STARTERS {
        if template_exists(ctx, project, name) {
            continue;
        }
        let items = lines.iter().map(|(section, text)| TemplateItem { section: (*section).into(), text: (*text).into() }).collect();
        add_template(ctx, project, (*name).into(), (*kind).into(), items);
        added += 1;
    }
    Ok(json!({ "added": added }))
}

fn template_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&InspectionTemplate> = v.state.inspection_templates.iter().filter(|t| t.project == project && t.deleted.is_none()).collect();
    list.sort_by_key(|t| t.name.to_lowercase());
    json(&list)
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Inspection> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Inspection> = p.id("id")?;
    let i = in_project(&ctx.state.inspections, project, id, "inspection")?;
    if !view::can_see_inspection(ctx.state, ctx.actor, i) {
        return Err(EngineError::not_found(format!("inspection {id} not found in this project")));
    }
    Ok(i.clone())
}

fn is_admin(ctx: &Ctx<'_>, i: &Inspection) -> bool {
    ctx.level(i.project, Tool::Inspections) == Level::Admin
}

fn is_manager(ctx: &Ctx<'_>, i: &Inspection) -> bool {
    is_admin(ctx, i) || (ctx.person().is_some() && i.manager == ctx.person())
}

/// Whether the actor may edit the inspection's details.
pub fn can_edit(state: &buildkiosk_model::CompanyState, actor: crate::Actor, i: &Inspection) -> bool {
    let lvl = view::level(state, actor, i.project, Tool::Inspections);
    lvl == Level::Admin || (lvl >= Level::Standard && actor.person().is_some() && i.manager == actor.person())
}

/// Whether the actor may answer, complete or attach files now.
pub fn can_answer(state: &buildkiosk_model::CompanyState, actor: crate::Actor, i: &Inspection) -> bool {
    let lvl = view::level(state, actor, i.project, Tool::Inspections);
    i.status == InspectionStatus::Open
        && (lvl == Level::Admin
            || (lvl >= Level::Standard && actor.person().is_some() && (i.manager == actor.person() || i.inspector == actor.person())))
}

fn link(i: &Inspection) -> String {
    format!("/projects/{}/inspections/{}", i.project, i.id)
}

fn person_field(ctx: &Ctx<'_>, p: &Params<'_>, key: &str) -> Result<Option<Id<Person>>> {
    let v = p.opt_id::<Person>(key)?;
    if let Some(v) = v {
        ctx.check_person(v)?;
    }
    Ok(v)
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let template = match p.opt_id::<InspectionTemplate>("template")? {
        Some(t) => Some(in_project(&ctx.state.inspection_templates, project, t, "inspection template")?.clone()).filter(|t| t.deleted.is_none()),
        None => None,
    };
    let (mut items, mut name, mut kind) = match &template {
        Some(t) => (t.items.clone(), t.name.clone(), t.kind.clone()),
        None => (Vec::new(), String::new(), String::new()),
    };
    if template.is_none() {
        if p.opt_id::<InspectionTemplate>("template")?.is_some() {
            return Err(EngineError::not_found("inspection template not found in this project"));
        }
        if p.present("items") {
            items = parse_items(p, "items", &p.req_text("items", 100_000)?)?;
        }
    }
    if items.is_empty() {
        return Err(p.bad("template", "choose a template, or give checklist lines in `items`"));
    }
    if p.present("kind") {
        kind = p.line_or_empty("kind", 100)?;
    }
    let title = match p.line("title", text::SHORT)? {
        Some(t) if !t.is_empty() => t,
        _ if !name.is_empty() => std::mem::take(&mut name),
        _ => return Err(p.bad("title", "is required when no template is used")),
    };
    let number = ctx.state.inspections.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let i = Inspection {
        id: ctx.state.inspections.peek_next(),
        project,
        number,
        title,
        template_name: template.map(|t| t.name).unwrap_or_default(),
        kind,
        location: p.line_or_empty("location", text::SHORT)?,
        status: InspectionStatus::Open,
        inspector: person_field(ctx, p, "inspector")?,
        manager: if p.present("manager") { person_field(ctx, p, "manager")? } else { ctx.person() },
        scheduled: p.date("scheduled")?,
        items: items
            .into_iter()
            .map(|t| ChecklistItem { section: t.section, text: t.text, answer: None, note: String::new(), punch: None })
            .collect(),
        completed: None,
        attachments: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    let id = ctx.state.inspections.alloc();
    let mut i = i;
    i.id = id;
    notify(ctx, &i.inspector.into_iter().collect::<Vec<_>>(), project, &format!("{} was assigned to you: {}", i.label(), i.title), &link(&i));
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_edit(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("only the manager or an inspections admin can edit {}", i.label())));
    }
    let before = i.clone();
    if p.present("title") {
        i.title = p.req_line("title", text::SHORT)?;
    }
    for (key, field) in [("kind", &mut i.kind), ("location", &mut i.location)] {
        if p.present(key) {
            *field = p.line_or_empty(key, text::SHORT)?;
        }
    }
    if p.present("inspector") {
        i.inspector = person_field(ctx, p, "inspector")?;
    }
    if p.present("manager") {
        i.manager = person_field(ctx, p, "manager")?;
    }
    if p.present("scheduled") {
        i.scheduled = p.date("scheduled")?;
    }
    let id = i.id;
    if i != before {
        if i.inspector != before.inspector {
            notify(
                ctx,
                &i.inspector.into_iter().collect::<Vec<_>>(),
                i.project,
                &format!("{} was assigned to you: {}", i.label(), i.title),
                &link(&i),
            );
        }
        i.updated = ctx.stamp();
        ctx.state.inspections.put(i);
    }
    Ok(json!({ "id": id }))
}

fn answer(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_answer(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(if i.status == InspectionStatus::Completed {
            format!("{} is completed; reopen it to change answers", i.label())
        } else {
            format!("only the inspector or manager of {} can answer it", i.label())
        }));
    }
    let rows = p.items("answers")?;
    if rows.is_empty() {
        return Err(p.bad("answers", "is required"));
    }
    for row in rows {
        let line = row.int("line", 1..=300)?.ok_or_else(|| row.bad("line", "is required"))?;
        let at = usize::try_from(line).unwrap_or(0).saturating_sub(1);
        let note = row.line_or_empty("note", text::SHORT)?;
        let choice = row.line_or_empty("answer", 10)?;
        let new = match choice.as_str() {
            "" => None,
            c => Some(Answer::parse(c).ok_or_else(|| row.bad("answer", "must be pass, fail or na"))?),
        };
        let Some(item) = i.items.get_mut(at) else { return Err(row.bad("line", format!("there is no line {line}"))) };
        if item.answer != new {
            // A changed answer no longer matches a punch item raised for the old one.
            item.punch = None;
        }
        item.answer = new;
        item.note = note;
    }
    i.updated = ctx.stamp();
    let (id, answered, total) = (i.id, i.answered(), i.items.len());
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id, "answered": answered, "lines": total }))
}

fn complete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_answer(ctx.state, ctx.actor, &i) {
        return Err(EngineError::forbidden(format!("only the inspector or manager of {} can complete it", i.label())));
    }
    let left = i.items.len().saturating_sub(i.answered());
    if left > 0 {
        return Err(EngineError::conflict(format!("{left} checklist line{} still need an answer", if left == 1 { "" } else { "s" })));
    }
    i.status = InspectionStatus::Completed;
    i.completed = Some(ctx.stamp());
    i.updated = ctx.stamp();
    let id = i.id;
    let outcome = if i.failed() == 0 { "no failed lines".to_string() } else { format!("{} failed", i.failed()) };
    notify(ctx, &i.manager.into_iter().collect::<Vec<_>>(), i.project, &format!("{} was completed ({outcome}): {}", i.label(), i.title), &link(&i));
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !is_manager(ctx, &i) {
        return Err(EngineError::forbidden("only the manager or an inspections admin can reopen an inspection"));
    }
    if i.status != InspectionStatus::Completed {
        return Err(EngineError::conflict(format!("{} is still in progress", i.label())));
    }
    i.status = InspectionStatus::Open;
    i.completed = None;
    i.updated = ctx.stamp();
    let id = i.id;
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id }))
}

fn raise_punch(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !is_manager(ctx, &i) {
        return Err(EngineError::forbidden("only the manager or an inspections admin can raise punch items"));
    }
    let assignees = p.ids("assignees")?;
    ctx.check_people(&assignees)?;
    let mut made = Vec::new();
    for at in 0..i.items.len() {
        let Some(item) = i.items.get(at) else { continue };
        if item.answer != Some(Answer::Fail) || item.punch.is_some() {
            continue;
        }
        let mut punch_item = PunchItem::blank(ctx, i.project);
        punch_item.title = format!("{}: {}", i.label(), item.text);
        punch_item.description = if item.note.is_empty() {
            format!("Failed on {} ({}).", i.label(), i.title)
        } else {
            format!("{}\n\nFailed on {} ({}).", item.note, i.label(), i.title)
        };
        punch_item.location = i.location.clone();
        punch_item.kind = i.kind.clone();
        punch_item.assignees = assignees.clone();
        punch_item.recompute_ball_in_court();
        let id = workitem::insert(ctx, punch_item);
        if let Some(item) = i.items.get_mut(at) {
            item.punch = Some(id);
        }
        made.push(id);
    }
    if made.is_empty() {
        return Err(EngineError::conflict("there are no failed lines without a punch item"));
    }
    i.updated = ctx.stamp();
    let id = i.id;
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id, "punch_items": made }))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_answer(ctx.state, ctx.actor, &i) && !is_manager(ctx, &i) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", i.label())));
    }
    let file = file_ref(p)?;
    if i.attachments.iter().any(|a| a.sha256 == file.sha256) {
        return Ok(json!({ "id": i.id }));
    }
    if i.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", i.label())));
    }
    i.attachments.push(file);
    i.updated = ctx.stamp();
    let id = i.id;
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut i = load(ctx, p)?;
    if !can_answer(ctx.state, ctx.actor, &i) && !is_manager(ctx, &i) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", i.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = i.attachments.len();
    i.attachments.retain(|a| a.sha256 != sha256);
    if i.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached to this inspection"));
    }
    i.updated = ctx.stamp();
    let id = i.id;
    ctx.state.inspections.put(i);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let i = load(ctx, p)?;
    let untouched = i.answered() == 0 && i.created.by.is_some() && i.created.by == ctx.person();
    if !is_admin(ctx, &i) && !untouched {
        return Err(EngineError::forbidden("only inspections admins can delete inspections (authors can delete their own before any answers)"));
    }
    let stamp = ctx.stamp();
    ctx.state.inspections.update(i.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": i.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Inspection> = p.id("id")?;
    let i = in_project(&ctx.state.inspections, project, id, "inspection")?;
    if i.deleted.is_none() {
        return Err(EngineError::conflict(format!("{} is not in the recycle bin", i.label())));
    }
    ctx.state.inspections.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let status = p.choice::<InspectionStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase());
    let mine = p.flag_or("mine", false)?;
    let overdue = p.flag_or("overdue", false)?;
    let today = v.today();
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Inspections) < Level::Admin {
            return Err(EngineError::forbidden("only inspections admins can see the recycle bin"));
        }
        let bin: Vec<&Inspection> = v.state.inspections.iter().filter(|i| i.project == project && i.deleted.is_some()).collect();
        return json(&bin);
    }
    let me = v.person();
    let list: Vec<&Inspection> = view::inspections_for(v.state, v.actor, project)
        .into_iter()
        .filter(|i| status.is_none_or(|s| i.status == s))
        .filter(|i| !mine || (me.is_some() && i.inspector == me))
        .filter(|i| !overdue || i.is_overdue(today))
        .filter(|i| {
            q.as_ref().is_none_or(|q| i.title.to_lowercase().contains(q) || i.location.to_lowercase().contains(q) || i.number.to_string() == *q)
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Inspection> = p.id("id")?;
    let i = in_project(&v.state.inspections, project, id, "inspection")?;
    if !view::can_see_inspection(v.state, v.actor, i) {
        return Err(EngineError::not_found(format!("inspection {id} not found in this project")));
    }
    json(i)
}
