//! Forms: a project defines its own templates (a list of questions), people fill in records from
//! them, and submit them. BuildKiosk ships no templates.
//!
//! A record keeps a copy of the template's questions. Its author edits it while it is a draft;
//! admins may edit and reopen a submitted one.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::form::{FieldKind, FormField, FormRecord, FormStatus, FormTemplate};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "form_template.create",
        "Create form template",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project: project id; name: text; description?: text; fields: [{label, kind?: text|long_text|number|date|yes_no|choice, required?: bool, options?: text such as `a, b, c` (for choice)}] (blank rows are skipped)",
        template_create,
    ),
    CommandSpec::mutate(
        "form_template.update",
        "Edit form template",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project, id: ids; then any form_template.create field (records already started keep the old questions)",
        template_update,
    ),
    CommandSpec::mutate(
        "form_template.delete",
        "Delete form template",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project, id: ids",
        template_delete,
    ),
    CommandSpec::query("form_template.list", "List form templates", Tool::Forms, Access::Project(Level::Read), "project: project id", template_list),
    CommandSpec::mutate(
        "form.create",
        "Start form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project: project id; template: template id; title?: text; answers?: {field key: answer}",
        create,
    ),
    CommandSpec::mutate(
        "form.update",
        "Edit form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids; title?: text; answers?: {field key: answer} (replaces the answers given; others stay)",
        update,
    ),
    CommandSpec::mutate(
        "form.submit",
        "Submit form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids (every required question needs an answer)",
        submit,
    ),
    CommandSpec::mutate("form.reopen", "Reopen form", Tool::Forms, Access::Project(Level::Admin), "project, id: ids", reopen),
    CommandSpec::mutate(
        "form.attach",
        "Attach file to form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        attach,
    ),
    CommandSpec::mutate(
        "form.detach",
        "Remove file from form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids; sha256: 64 hex digits",
        detach,
    ),
    CommandSpec::mutate(
        "form.delete",
        "Delete form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids (the author of a draft, or an admin)",
        delete,
    ),
    CommandSpec::mutate("form.restore", "Restore form", Tool::Forms, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "form.list",
        "List forms",
        Tool::Forms,
        Access::Project(Level::Read),
        "project: project id; template?: template id; status?: draft|submitted; q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("form.get", "Show form", Tool::Forms, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_FIELDS: usize = 100;
const MAX_OPTIONS: usize = 50;
const MAX_ATTACHMENTS: usize = 50;

// -- templates

fn template_fields(p: &Params<'_>) -> Result<Vec<FormField>> {
    let rows = p.items("fields")?;
    if rows.len() > MAX_FIELDS {
        return Err(p.bad("fields", format!("holds more than {MAX_FIELDS} questions")));
    }
    let mut out = Vec::new();
    for r in rows {
        let label = r.line_or_empty("label", text::SHORT)?;
        if label.is_empty() {
            continue;
        }
        let kind = r.choice::<FieldKind>("kind")?.unwrap_or_default();
        let mut options: Vec<String> = Vec::new();
        if kind == FieldKind::Choice {
            let raw: Vec<String> = r.text_or_empty("options", 5_000)?.split([',', '\n']).map(str::to_string).collect();
            for o in raw {
                let o = text::line(o.trim(), 100).map_err(|e| r.bad("options", e))?;
                if !o.is_empty() && !options.iter().any(|x: &String| x.eq_ignore_ascii_case(&o)) {
                    options.push(o);
                }
            }
            if options.len() < 2 {
                return Err(r.bad("options", format!("`{label}` needs at least two choices")));
            }
            if options.len() > MAX_OPTIONS {
                return Err(r.bad("options", format!("`{label}` has more than {MAX_OPTIONS} choices")));
            }
        }
        out.push(FormField { key: format!("f{}", out.len() + 1), label, kind, required: r.flag_or("required", false)?, options });
    }
    if out.is_empty() {
        return Err(p.bad("fields", "needs at least one question"));
    }
    Ok(out)
}

fn live_template(ctx: &impl Read, project: Id<Project>, id: Id<FormTemplate>) -> Result<FormTemplate> {
    let t = in_project(&ctx.state().form_templates, project, id, "form template")?;
    if t.deleted.is_some() {
        return Err(EngineError::not_found(format!("form template {id} not found in this project")));
    }
    Ok(t.clone())
}

fn template_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", text::SHORT)?;
    if ctx.state.form_templates.iter().any(|t| t.project == project && t.deleted.is_none() && t.name.eq_ignore_ascii_case(&name)) {
        return Err(EngineError::conflict(format!("a form template named {name} already exists")));
    }
    let fields = template_fields(p)?;
    let id = ctx.state.form_templates.alloc();
    let stamp = ctx.stamp();
    ctx.state.form_templates.put(FormTemplate {
        id,
        project,
        name,
        description: p.text_or_empty("description", text::LONG)?,
        fields,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn template_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<FormTemplate> = p.id("id")?;
    let mut t = live_template(ctx, project, id)?;
    if p.present("name") {
        let name = p.req_line("name", text::SHORT)?;
        if ctx.state.form_templates.iter().any(|x| x.project == project && x.deleted.is_none() && x.id != id && x.name.eq_ignore_ascii_case(&name)) {
            return Err(EngineError::conflict(format!("a form template named {name} already exists")));
        }
        t.name = name;
    }
    if p.present("description") {
        t.description = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("fields") {
        t.fields = template_fields(p)?;
    }
    t.updated = ctx.stamp();
    ctx.state.form_templates.put(t);
    Ok(json!({ "id": id }))
}

fn template_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<FormTemplate> = p.id("id")?;
    live_template(ctx, project, id)?;
    let stamp = ctx.stamp();
    ctx.state.form_templates.update(id, |t| t.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn template_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&FormTemplate> = v.state.form_templates.iter().filter(|t| t.project == project && t.deleted.is_none()).collect();
    list.sort_by_key(|t| t.name.to_lowercase());
    json(&list)
}

// -- records

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<FormRecord> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<FormRecord> = p.id("id")?;
    let r = in_project(&ctx.state.form_records, project, id, "form")?;
    if r.deleted.is_some() {
        return Err(EngineError::not_found(format!("form {id} not found in this project")));
    }
    Ok(r.clone())
}

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Forms) == Level::Admin
}

fn is_author(ctx: &Ctx<'_>, r: &FormRecord) -> bool {
    ctx.person().is_some() && r.created.by == ctx.person()
}

fn can_edit(ctx: &Ctx<'_>, r: &FormRecord) -> bool {
    is_admin(ctx, r.project) || (r.status == FormStatus::Draft && is_author(ctx, r))
}

/// Checks an answer against its question and returns it in a standard spelling.
fn clean_answer(f: &FormField, raw: &str, p: &Params<'_>) -> Result<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(String::new());
    }
    let bad = |msg: String| p.bad("answers", format!("{}: {msg}", f.label));
    Ok(match f.kind {
        FieldKind::Text => text::line(raw, text::SHORT).map_err(|e| bad(e.to_string()))?,
        FieldKind::LongText => text::multiline(raw, text::LONG).map_err(|e| bad(e.to_string()))?,
        FieldKind::Number => {
            let n: f64 = raw.replace(',', "").parse().map_err(|_| bad(format!("`{raw}` is not a number")))?;
            if !n.is_finite() || n.abs() > 1e15 {
                return Err(bad("the number is out of range".into()));
            }
            raw.replace(',', "")
        }
        FieldKind::Date => Date::parse(raw).ok_or_else(|| bad(format!("`{raw}` is not a date (YYYY-MM-DD)")))?.to_string(),
        FieldKind::YesNo => match raw.to_ascii_lowercase().as_str() {
            "yes" | "true" | "on" | "y" => "yes".into(),
            "no" | "false" | "off" | "n" => "no".into(),
            _ => return Err(bad("answer yes or no".into())),
        },
        FieldKind::Choice => {
            f.options.iter().find(|o| o.eq_ignore_ascii_case(raw)).cloned().ok_or_else(|| bad(format!("`{raw}` is not one of the choices")))?
        }
    })
}

fn apply_answers(p: &Params<'_>, r: &mut FormRecord) -> Result<()> {
    let Some(map) = p.object("answers") else { return Ok(()) };
    for (key, v) in map {
        let Some(field) = r.fields.iter().find(|f| f.key == *key) else { return Err(p.bad("answers", format!("there is no question `{key}`"))) };
        let raw = match v {
            Value::String(s) => s.clone(),
            Value::Bool(b) => {
                if *b {
                    "yes".into()
                } else {
                    "no".into()
                }
            }
            Value::Number(n) => n.to_string(),
            Value::Null => String::new(),
            _ => return Err(p.bad("answers", format!("{}: the answer must be text", field.label))),
        };
        let clean = clean_answer(field, &raw, p)?;
        if clean.is_empty() {
            r.answers.remove(key);
        } else {
            r.answers.insert(key.clone(), clean);
        }
    }
    Ok(())
}

fn create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let t = live_template(ctx, project, p.id("template")?)?;
    let number = ctx.state.form_records.iter().filter(|r| r.project == project).map(|r| r.number).max().unwrap_or(0).saturating_add(1);
    let stamp = ctx.stamp();
    let mut r = FormRecord {
        id: ctx.state.form_records.peek_next(),
        project,
        template: t.id,
        template_name: t.name.clone(),
        number,
        title: p.line_or_empty("title", text::SHORT)?,
        fields: t.fields.clone(),
        answers: BTreeMap::new(),
        status: FormStatus::Draft,
        attachments: Vec::new(),
        submitted: None,
        created: stamp,
        updated: stamp,
        deleted: None,
    };
    apply_answers(p, &mut r)?;
    let id = ctx.state.form_records.alloc();
    r.id = id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id, "number": number }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if !can_edit(ctx, &r) {
        return Err(EngineError::forbidden(format!("you can't edit {}", r.label())));
    }
    if p.present("title") {
        r.title = p.line_or_empty("title", text::SHORT)?;
    }
    apply_answers(p, &mut r)?;
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn submit(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if !can_edit(ctx, &r) {
        return Err(EngineError::forbidden(format!("you can't submit {}", r.label())));
    }
    if r.status != FormStatus::Draft {
        return Err(EngineError::conflict(format!("{} was already submitted", r.label())));
    }
    let missing: Vec<&str> = r.fields.iter().filter(|f| f.required && !r.answers.contains_key(&f.key)).map(|f| f.label.as_str()).collect();
    if !missing.is_empty() {
        return Err(EngineError::conflict(format!("answer these first: {}", missing.join(", "))));
    }
    r.status = FormStatus::Submitted;
    r.submitted = Some(ctx.stamp());
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn reopen(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if r.status != FormStatus::Submitted {
        return Err(EngineError::conflict("the form is not submitted"));
    }
    r.status = FormStatus::Draft;
    r.submitted = None;
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn attach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if !can_edit(ctx, &r) {
        return Err(EngineError::forbidden(format!("you can't add files to {}", r.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    if r.attachments.iter().any(|a| a.sha256 == sha256) {
        return Ok(json!({ "id": r.id }));
    }
    if r.attachments.len() >= MAX_ATTACHMENTS {
        return Err(EngineError::conflict(format!("{} already has {MAX_ATTACHMENTS} files", r.label())));
    }
    r.attachments.push(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime });
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn detach(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if !can_edit(ctx, &r) {
        return Err(EngineError::forbidden(format!("you can't remove files from {}", r.label())));
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    let before = r.attachments.len();
    r.attachments.retain(|a| a.sha256 != sha256);
    if r.attachments.len() == before {
        return Err(EngineError::not_found("that file is not attached"));
    }
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let r = load(ctx, p)?;
    if !can_edit(ctx, &r) {
        return Err(EngineError::forbidden("only the author of a draft or an admin can delete a form"));
    }
    let stamp = ctx.stamp();
    ctx.state.form_records.update(r.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": r.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<FormRecord> = p.id("id")?;
    let r = in_project(&ctx.state.form_records, project, id, "form")?;
    if r.deleted.is_none() {
        return Err(EngineError::conflict("the form is not in the recycle bin"));
    }
    ctx.state.form_records.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// The project's forms, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&FormRecord> {
    let mut v: Vec<&FormRecord> = state.form_records.iter().filter(|r| r.project == project && r.deleted.is_none()).collect();
    v.sort_by_key(|r| std::cmp::Reverse(r.number));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Forms) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&FormRecord> = v.state.form_records.iter().filter(|r| r.project == project && r.deleted.is_some()).collect();
        return json(&bin);
    }
    let template: Option<Id<FormTemplate>> = p.opt_id("template")?;
    let status = p.choice::<FormStatus>("status")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let list: Vec<&FormRecord> = visible(v.state, project)
        .into_iter()
        .filter(|r| template.is_none_or(|t| r.template == t))
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| {
            q.as_ref().is_none_or(|q| {
                r.title.to_lowercase().contains(q.as_str())
                    || r.template_name.to_lowercase().contains(q.as_str())
                    || r.answers.values().any(|a| a.to_lowercase().contains(q.as_str()))
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<FormRecord> = p.id("id")?;
    let r = in_project(&v.state.form_records, project, id, "form")?;
    if r.deleted.is_some() {
        return Err(EngineError::not_found(format!("form {id} not found in this project")));
    }
    json(r)
}
