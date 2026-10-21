//! Projects, their team (members) and cost codes.

use std::collections::BTreeMap;

use buildkiosk_core::{Id, text};
use buildkiosk_model::directory::{Person, Vendor};
use buildkiosk_model::perm::{PermissionTemplate, ProjectMember, membership};
use buildkiosk_model::project::{CostCode, CostType, Project, ProjectStage};
use buildkiosk_model::{Choice, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::Read;
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Result, View};

const PROJECT_FIELDS: &str = "name: text; number?: text (job number); stage?: bidding|preconstruction|construction|closeout|warranty|complete|on_hold|cancelled; \
     kind?: text; street?, city?, region?, postal_code?, country?: text; start?, completion?: YYYY-MM-DD; value?: amount; description?: text; \
     owner?: company id; rfi_due_days?: integer (1..90); observation_types?: text, one category per line (up to 50)";

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate("project.create", "Create project", Tool::Portfolio, Access::Company(Level::Standard), PROJECT_FIELDS, create),
    CommandSpec::mutate(
        "project.update",
        "Edit project",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; then any of the project.create fields, plus active?: bool",
        update,
    ),
    CommandSpec::mutate("project.delete", "Delete project", Tool::Portfolio, Access::Company(Level::Admin), "id: project id", delete),
    CommandSpec::mutate("project.restore", "Restore project", Tool::Portfolio, Access::Company(Level::Admin), "id: project id", restore),
    CommandSpec::query("project.list", "List my projects", Tool::Portfolio, Access::Member, "include_inactive?: bool", list),
    CommandSpec::query("project.get", "Show project", Tool::ProjectHome, Access::Project(Level::Read), "project: project id", get),
    CommandSpec::mutate(
        "member.add",
        "Add person to project",
        Tool::ProjectDirectory,
        Access::Project(Level::Admin),
        "project: project id; person: person id; template?: permission template id; role?: text; project_admin?: bool",
        member_add,
    ),
    CommandSpec::mutate(
        "member.update",
        "Change project permissions",
        Tool::ProjectDirectory,
        Access::Project(Level::Admin),
        "project: project id; person: person id; template?: template id; role?: text; project_admin?: bool; \
         overrides?: {tool key: none|read|standard|admin|inherit}",
        member_update,
    ),
    CommandSpec::mutate(
        "member.remove",
        "Remove person from project",
        Tool::ProjectDirectory,
        Access::Project(Level::Admin),
        "project: project id; person: person id",
        member_remove,
    ),
    CommandSpec::query("member.list", "List project team", Tool::ProjectDirectory, Access::Project(Level::Read), "project: project id", member_list),
    CommandSpec::mutate(
        "cost_code.create",
        "Add cost code",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; code: text; description: text; cost_type?: labor|material|equipment|subcontract|other (the kind of cost, used to group the budget)",
        cost_code_create,
    ),
    CommandSpec::mutate(
        "cost_code.import",
        "Import cost codes",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; text: one code per line, `code<TAB or comma>description`; with tabs a third cell may give the cost type (labor, material, equipment, subcontract or other)",
        cost_code_import,
    ),
    CommandSpec::mutate(
        "cost_code.set_type",
        "Set a cost code's cost type",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; id: cost code id; cost_type: labor|material|equipment|subcontract|other, or empty to clear it",
        cost_code_set_type,
    ),
    CommandSpec::mutate(
        "cost_code.set_types",
        "Set the cost type of many cost codes",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; text: one per line, `cost code<space, comma or tab>type` where the type is labor, material, equipment, subcontract, other, or none to clear it; all lines apply or none",
        cost_code_set_types,
    ),
    CommandSpec::mutate(
        "cost_code.delete",
        "Delete cost code",
        Tool::ProjectAdmin,
        Access::Project(Level::Admin),
        "project: project id; id: cost code id",
        cost_code_delete,
    ),
    CommandSpec::query("cost_code.list", "List cost codes", Tool::ProjectHome, Access::Project(Level::Read), "project: project id", cost_code_list),
    CommandSpec::mutate(
        "company.cost_code.import",
        "Import company cost codes",
        Tool::CompanyAdmin,
        Access::Company(Level::Admin),
        "text: one code per line, `code<TAB or comma>description`; replace?: bool (delete the current company list first)",
        company_cost_code_import,
    ),
];

fn apply_fields(ctx: &Ctx<'_>, p: &Params<'_>, x: &mut Project, creating: bool) -> Result<()> {
    if creating || p.present("name") {
        x.name = p.req_line("name", text::SHORT)?;
    }
    if p.present("number") {
        x.number = p.line_or_empty("number", 50)?;
    }
    if let Some(s) = p.choice::<ProjectStage>("stage")? {
        x.stage = s;
    }
    for (key, field, max) in [
        ("kind", &mut x.kind, 100),
        ("street", &mut x.address.street, text::SHORT),
        ("city", &mut x.address.city, 100),
        ("region", &mut x.address.region, 100),
        ("postal_code", &mut x.address.postal_code, 20),
        ("country", &mut x.address.country, 100),
    ] {
        if p.present(key) {
            *field = p.line_or_empty(key, max)?;
        }
    }
    if p.present("start") {
        x.start = p.date("start")?;
    }
    if p.present("completion") {
        x.completion = p.date("completion")?;
    }
    if let (Some(s), Some(c)) = (x.start, x.completion)
        && c < s
    {
        return Err(p.bad("completion", "is before the start date"));
    }
    if p.present("value") {
        x.value = p.money("value")?;
    }
    if p.present("description") {
        x.description = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("owner") {
        let o = p.opt_id::<Vendor>("owner")?;
        if let Some(o) = o {
            ctx.check_vendor(o)?;
        }
        x.owner = o;
    }
    if p.present("rfi_due_days") {
        x.rfi_due_days = p.int("rfi_due_days", 1..=90)?.and_then(|d| u32::try_from(d).ok());
    }
    if p.present("observation_types") {
        let body = p.text_or_empty("observation_types", 10_000)?;
        let mut types: Vec<String> = Vec::new();
        for line in body.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let t = text::required_line(line, 100).map_err(|e| p.bad("observation_types", e.to_string()))?;
            if !types.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                types.push(t);
            }
        }
        if types.len() > 50 {
            return Err(p.bad("observation_types", "can hold at most 50 categories"));
        }
        x.observation_types = types;
    }
    if let Some(a) = p.flag("active")? {
        x.active = a;
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let stamp = ctx.stamp();
    let mut x = Project {
        id: ctx.state.projects.peek_next(),
        number: String::new(),
        name: String::new(),
        stage: ProjectStage::default(),
        kind: String::new(),
        address: Default::default(),
        start: None,
        completion: None,
        value: None,
        description: String::new(),
        owner: None,
        rfi_due_days: None,
        observation_types: Vec::new(),
        active: true,
        budget_locked: false,
        forecast_method: Default::default(),
        require_signed_contracts: false,
        incident_hours: Vec::new(),
        overtime: Default::default(),
        calendar: Default::default(),
        holiday_lists: Vec::new(),
        resource_limits: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_fields(ctx, p, &mut x, true)?;
    if !x.number.is_empty() && ctx.state.projects.iter().any(|o| o.deleted.is_none() && o.number == x.number) {
        return Err(p.bad("number", format!("project number {} is in use", x.number)));
    }
    let id = ctx.state.projects.alloc();
    x.id = id;
    ctx.state.projects.put(x);
    // The creator runs the project.
    if let Some(person) = ctx.person() {
        let mid = ctx.state.members.alloc();
        ctx.state.members.put(ProjectMember {
            id: mid,
            project: id,
            person,
            template: None,
            overrides: BTreeMap::new(),
            project_admin: true,
            role: "Project creator".into(),
            added: stamp,
            removed: None,
        });
    }
    // Start from the company's standard cost codes.
    let standard: Vec<(String, String, Option<CostType>)> = ctx
        .state
        .cost_codes
        .iter()
        .filter(|c| c.project.is_none() && c.deleted.is_none())
        .map(|c| (c.code.clone(), c.description.clone(), c.cost_type))
        .collect();
    for (code, description, cost_type) in standard {
        let cid = ctx.state.cost_codes.alloc();
        ctx.state.cost_codes.put(CostCode { id: cid, project: Some(id), code, description, cost_type, created: stamp, deleted: None });
    }
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Project> = p.id("project")?;
    let before = ctx.state.projects.get(id).cloned().ok_or_else(|| EngineError::not_found(format!("project {id} not found")))?;
    let mut x = before.clone();
    apply_fields(ctx, p, &mut x, false)?;
    if !x.number.is_empty() && ctx.state.projects.iter().any(|o| o.id != id && o.deleted.is_none() && o.number == x.number) {
        return Err(p.bad("number", format!("project number {} is in use", x.number)));
    }
    if x != before {
        x.updated = ctx.stamp();
        ctx.state.projects.put(x);
    }
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Project> = p.id("id")?;
    let stamp = ctx.stamp();
    ctx.state
        .projects
        .update(id, |x| {
            if x.deleted.is_none() {
                x.deleted = Some(stamp)
            }
        })
        .ok_or_else(|| EngineError::not_found(format!("project {id} not found")))?;
    Ok(json!({ "id": id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Project> = p.id("id")?;
    ctx.state.projects.update(id, |x| x.deleted = None).ok_or_else(|| EngineError::not_found(format!("project {id} not found")))?;
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let inactive = p.flag_or("include_inactive", false)?;
    let list: Vec<&Project> = view::projects_for(v.state, v.actor).into_iter().filter(|x| inactive || x.active).collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<Project> = p.id("project")?;
    json(v.state.projects.get(id).ok_or_else(|| EngineError::not_found(format!("project {id} not found")))?)
}

fn check_template(ctx: &Ctx<'_>, t: Option<Id<PermissionTemplate>>) -> Result<()> {
    match t {
        Some(t) if ctx.state.templates.get(t).is_none_or(|x| x.deleted.is_some()) => Err(ctx.bad(format!("permission template {t} not found"))),
        _ => Ok(()),
    }
}

fn member_add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = p.id("person")?;
    ctx.check_person(person)?;
    if membership(ctx.state, project, person).is_some() {
        return Err(EngineError::conflict(format!("{} is already on the project", view::person_name(ctx.state, person))));
    }
    let template = p.opt_id("template")?;
    check_template(ctx, template)?;
    let project_admin = p.flag_or("project_admin", false)?;
    if project_admin && ctx.level(project, Tool::ProjectAdmin) < Level::Admin {
        return Err(EngineError::forbidden("only project admins can add project admins"));
    }
    let id = ctx.state.members.alloc();
    let stamp = ctx.stamp();
    ctx.state.members.put(ProjectMember {
        id,
        project,
        person,
        template,
        overrides: BTreeMap::new(),
        project_admin,
        role: p.line_or_empty("role", 100)?,
        added: stamp,
        removed: None,
    });
    Ok(json!({ "id": id }))
}

fn member_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = p.id("person")?;
    let mut m = membership(ctx.state, project, person).cloned().ok_or_else(|| EngineError::not_found("that person isn't on the project"))?;
    if p.present("template") {
        let t = p.opt_id("template")?;
        check_template(ctx, t)?;
        m.template = t;
    }
    if p.present("role") {
        m.role = p.line_or_empty("role", 100)?;
    }
    if let Some(a) = p.flag("project_admin")?
        && a != m.project_admin
    {
        if ctx.level(project, Tool::ProjectAdmin) < Level::Admin {
            return Err(EngineError::forbidden("only project admins can change who is a project admin"));
        }
        m.project_admin = a;
    }
    if let Some(map) = p.object("overrides") {
        if map.len() > Tool::ALL.len() * 2 {
            return Err(p.bad("overrides", "has too many entries"));
        }
        for (k, v) in map {
            let tool = Tool::parse(k).ok_or_else(|| p.bad("overrides", format!("`{k}` is not a tool")))?;
            match v.as_str().map(str::trim) {
                Some("inherit") | Some("") | None => {
                    m.overrides.remove(&tool);
                }
                Some(s) => {
                    let level = Level::parse(s).ok_or_else(|| p.bad("overrides", format!("`{s}` is not a level for {k}")))?;
                    m.overrides.insert(tool, level);
                }
            }
        }
    }
    let id = m.id;
    ctx.state.members.put(m);
    Ok(json!({ "id": id }))
}

fn member_remove(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let person: Id<Person> = p.id("person")?;
    let id = membership(ctx.state, project, person).map(|m| m.id).ok_or_else(|| EngineError::not_found("that person isn't on the project"))?;
    let stamp = ctx.stamp();
    ctx.state.members.update(id, |m| m.removed = Some(stamp));
    Ok(json!({ "id": id }))
}

fn member_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let list: Vec<&ProjectMember> = v.state.members.iter().filter(|m| m.project == project && m.removed.is_none()).collect();
    json(&list)
}

/// Parses `code<TAB or comma>description` lines; blank lines and lines starting with `#` are skipped.
fn parse_code_lines(p: &Params<'_>) -> Result<Vec<(String, String, Option<CostType>)>> {
    let body = p.req_text("text", 500_000)?;
    let mut out = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // With tabs the cells are code, description and an optional cost type; with commas only code and description
        // (a description may contain commas).
        let (code, desc, kind) = if line.contains('\t') {
            let mut cells = line.splitn(3, '\t');
            (cells.next().unwrap_or(""), cells.next().unwrap_or(""), cells.next().unwrap_or("").trim())
        } else {
            let (c, d) = line.split_once(',').unwrap_or((line, ""));
            (c, d, "")
        };
        let code = text::required_line(code, 50).map_err(|e| p.bad("text", format!("line {}: code {e}", n + 1)))?;
        let desc = text::line(desc.trim().trim_matches('"'), text::SHORT).map_err(|e| p.bad("text", format!("line {}: description {e}", n + 1)))?;
        let kind = if kind.is_empty() {
            None
        } else {
            Some(CostType::parse(kind).ok_or_else(|| {
                p.bad("text", format!("line {}: `{kind}` is not a cost type (labor, material, equipment, subcontract or other)", n + 1))
            })?)
        };
        out.push((code, desc, kind));
        if out.len() > 20_000 {
            return Err(p.bad("text", "has more than 20,000 codes"));
        }
    }
    Ok(out)
}

fn cost_code_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let code = p.req_line("code", 50)?;
    let description = p.req_line("description", text::SHORT)?;
    if ctx.state.cost_codes.iter().any(|c| c.project == Some(project) && c.deleted.is_none() && c.code == code) {
        return Err(p.bad("code", format!("cost code {code} exists on this project")));
    }
    let id = ctx.state.cost_codes.alloc();
    let stamp = ctx.stamp();
    let cost_type = p.choice::<CostType>("cost_type")?;
    ctx.state.cost_codes.put(CostCode { id, project: Some(project), code, description, cost_type, created: stamp, deleted: None });
    Ok(json!({ "id": id }))
}

fn cost_code_import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let lines = parse_code_lines(p)?;
    let stamp = ctx.stamp();
    let mut added = 0usize;
    for (code, description, cost_type) in lines {
        if ctx.state.cost_codes.iter().any(|c| c.project == Some(project) && c.deleted.is_none() && c.code == code) {
            continue;
        }
        let id = ctx.state.cost_codes.alloc();
        ctx.state.cost_codes.put(CostCode { id, project: Some(project), code, description, cost_type, created: stamp, deleted: None });
        added += 1;
    }
    Ok(json!({ "added": added }))
}

/// A cost type from text: empty, `none` or `clear` clear it.
fn cost_type_of(raw: &str) -> Option<Option<CostType>> {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") || raw.eq_ignore_ascii_case("clear") {
        return Some(None);
    }
    CostType::parse(raw).map(Some)
}

fn cost_code_set_type(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<CostCode> = p.id("id")?;
    if ctx.state.cost_codes.get(id).is_none_or(|c| c.project != Some(project) || c.deleted.is_some()) {
        return Err(EngineError::not_found(format!("cost code {id} not found in this project")));
    }
    let raw = p.line_or_empty("cost_type", 40)?;
    let cost_type = cost_type_of(&raw).ok_or_else(|| p.bad("cost_type", "must be labor, material, equipment, subcontract or other"))?;
    ctx.state.cost_codes.update(id, |c| c.cost_type = cost_type);
    Ok(json!({ "id": id }))
}

fn cost_code_set_types(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let body = p.req_text("text", 200_000)?;
    // Read every line before changing anything.
    let mut changes: Vec<(Id<CostCode>, Option<CostType>)> = Vec::new();
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (code, kind) = line
            .rsplit_once(['\t', ',', ' '])
            .map(|(c, k)| (c.trim().trim_end_matches(',').trim(), k.trim()))
            .ok_or_else(|| p.bad("text", format!("line {}: give a cost code and a cost type", n + 1)))?;
        let cost_type = cost_type_of(kind).ok_or_else(|| {
            p.bad("text", format!("line {}: `{kind}` is not a cost type (labor, material, equipment, subcontract, other or none)", n + 1))
        })?;
        let found = ctx
            .state
            .cost_codes
            .iter()
            .find(|c| c.project == Some(project) && c.deleted.is_none() && c.code.eq_ignore_ascii_case(code))
            .map(|c| c.id)
            .ok_or_else(|| p.bad("text", format!("line {}: the project has no cost code `{code}`", n + 1)))?;
        changes.push((found, cost_type));
        if changes.len() > 20_000 {
            return Err(p.bad("text", "has more than 20,000 lines"));
        }
    }
    for (id, cost_type) in &changes {
        ctx.state.cost_codes.update(*id, |c| c.cost_type = *cost_type);
    }
    Ok(json!({ "changed": changes.len() }))
}

fn cost_code_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<CostCode> = p.id("id")?;
    if ctx.state.cost_codes.get(id).is_none_or(|c| c.project != Some(project)) {
        return Err(EngineError::not_found(format!("cost code {id} not found in this project")));
    }
    let stamp = ctx.stamp();
    ctx.state.cost_codes.update(id, |c| c.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn cost_code_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&CostCode> = v.state.cost_codes.iter().filter(|c| c.project == Some(project) && c.deleted.is_none()).collect();
    list.sort_by(|a, b| a.code.cmp(&b.code));
    json(&list)
}

fn company_cost_code_import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let lines = parse_code_lines(p)?;
    let stamp = ctx.stamp();
    if p.flag_or("replace", false)? {
        let current: Vec<Id<CostCode>> = ctx.state.cost_codes.iter().filter(|c| c.project.is_none() && c.deleted.is_none()).map(|c| c.id).collect();
        for id in current {
            ctx.state.cost_codes.update(id, |c| c.deleted = Some(stamp));
        }
    }
    let mut added = 0usize;
    for (code, description, cost_type) in lines {
        if ctx.state.cost_codes.iter().any(|c| c.project.is_none() && c.deleted.is_none() && c.code == code) {
            continue;
        }
        let id = ctx.state.cost_codes.alloc();
        ctx.state.cost_codes.put(CostCode { id, project: None, code, description, cost_type, created: stamp, deleted: None });
        added += 1;
    }
    Ok(json!({ "added": added }))
}

/// The level names a member's template gives, for the project directory page.
pub fn template_name(state: &buildkiosk_model::CompanyState, m: &ProjectMember) -> String {
    if m.project_admin {
        return "Project admin".into();
    }
    m.template.and_then(|t| state.templates.get(t)).map(|t| t.name.clone()).unwrap_or_else(|| "No template".into())
}

/// For UIs: the members' levels in one tool.
pub fn tool_level_label(level: Level) -> &'static str {
    level.label()
}
