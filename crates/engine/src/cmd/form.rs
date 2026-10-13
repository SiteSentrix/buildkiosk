//! Forms: a project defines its own templates (a list of questions), people fill in records from
//! them, and submit them. BuildKiosk ships no templates.
//!
//! A record keeps a copy of the template's questions. Its author edits it while it is a draft;
//! admins may edit and reopen a submitted one.

use std::collections::BTreeMap;

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::form::{FieldKind, FormField, FormRecord, FormSchedule, FormSignature, FormStatus, FormTemplate, Repeat, ShowIf};
use buildkiosk_model::project::Project;
use buildkiosk_model::{CompanyState, Level, Tool};
use serde_json::{Value, json};

use super::notification::notify;
use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View, view};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "form_template.create",
        "Create form template",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project: project id; name: text; description?: text; fields: [{label, kind?: text|long_text|number|date|yes_no|choice, required?: bool, options?: text such as `a, b, c` (for choice), show_if_question?: number of an earlier question (1 is the first), show_if_answer?: the answer to it that shows this question}] (blank rows are skipped); signers?: roles that sign a submitted form, such as `Inspector, Superintendent`",
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
    CommandSpec::mutate(
        "form_template.copy",
        "Copy form template to another project",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project, id: ids of the template; to_project: project id (you must be a Forms admin there too; a template of the same name is refused)",
        template_copy,
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
    CommandSpec::mutate(
        "form.sign",
        "Sign form",
        Tool::Forms,
        Access::Project(Level::Standard),
        "project, id: ids; role: one of the template's signing roles; name: your own full name, typed (a submitted form; nobody signs two roles on one form)",
        sign,
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
    CommandSpec::mutate(
        "form_schedule.save",
        "Schedule a recurring form",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project: project id; id?: schedule id (changes it); template: template id; every: daily|weekly|biweekly|monthly; assignee: person id who fills it in; next?: YYYY-MM-DD of the first form (default today); title?: text; active?: bool",
        schedule_save,
    ),
    CommandSpec::mutate(
        "form_schedule.delete",
        "Delete form schedule",
        Tool::Forms,
        Access::Project(Level::Admin),
        "project, id: ids",
        schedule_delete,
    ),
    CommandSpec::query("form_schedule.list", "List form schedules", Tool::Forms, Access::Project(Level::Read), "project: project id", schedule_list),
    CommandSpec::mutate(
        "form.generate_due",
        "Start the forms that are due",
        Tool::Forms,
        Access::System,
        "(none) (makes one draft form for every active schedule whose day has come, tells the assignee, and sets the next day)",
        generate_due,
    ),
    CommandSpec::query(
        "form.report",
        "Report across forms",
        Tool::Forms,
        Access::Project(Level::Read),
        "project: project id; template: template id; status?: draft|submitted; from?, to?: dates the form was started (rows of every form from one template, one column per question)",
        report,
    ),
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
        let show_if = match r.int("show_if_question", 0..=100)? {
            None | Some(0) => None,
            Some(n) => {
                let idx = usize::try_from(n).unwrap_or(usize::MAX);
                let Some(before) = idx.checked_sub(1).and_then(|i| out.get(i)) else {
                    return Err(r.bad("show_if_question", format!("`{label}` can depend only on a question above it")));
                };
                let before: &FormField = before;
                let raw = r.line_or_empty("show_if_answer", 100)?;
                if raw.is_empty() {
                    return Err(r.bad("show_if_answer", format!("`{label}` needs the answer that shows it")));
                }
                let equals = match before.kind {
                    FieldKind::YesNo => match raw.to_ascii_lowercase().as_str() {
                        "yes" | "true" | "y" => "yes".to_string(),
                        "no" | "false" | "n" => "no".to_string(),
                        _ => return Err(r.bad("show_if_answer", "answer yes or no")),
                    },
                    FieldKind::Choice => before
                        .options
                        .iter()
                        .find(|o| o.eq_ignore_ascii_case(&raw))
                        .cloned()
                        .ok_or_else(|| r.bad("show_if_answer", format!("`{raw}` is not one of the choices of `{}`", before.label)))?,
                    _ => raw,
                };
                Some(ShowIf { field: before.key.clone(), equals })
            }
        };
        out.push(FormField { key: format!("f{}", out.len() + 1), label, kind, required: r.flag_or("required", false)?, options, show_if });
    }
    if out.is_empty() {
        return Err(p.bad("fields", "needs at least one question"));
    }
    Ok(out)
}

fn template_signers(p: &Params<'_>) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for part in p.text_or_empty("signers", 1_000)?.split([',', '\n']) {
        let role = text::line(part.trim(), 60).map_err(|e| p.bad("signers", e))?;
        if !role.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&role)) {
            out.push(role);
        }
    }
    if out.len() > 5 {
        return Err(p.bad("signers", "names more than five roles"));
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
        signers: template_signers(p)?,
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
    if p.present("signers") {
        t.signers = template_signers(p)?;
    }
    t.updated = ctx.stamp();
    ctx.state.form_templates.put(t);
    Ok(json!({ "id": id }))
}

fn template_copy(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let to: Id<Project> = p.id("to_project")?;
    let t = live_template(ctx, project, p.id("id")?)?;
    if !ctx.state.projects.get(to).is_some_and(|x| x.deleted.is_none()) || ctx.level(to, Tool::Forms) != Level::Admin {
        return Err(EngineError::not_found(format!("project {to} not found")));
    }
    if ctx.state.form_templates.iter().any(|x| x.project == to && x.deleted.is_none() && x.name.eq_ignore_ascii_case(&t.name)) {
        return Err(EngineError::conflict(format!("the other project already has a form template named {}", t.name)));
    }
    let id = ctx.state.form_templates.alloc();
    let stamp = ctx.stamp();
    ctx.state.form_templates.put(FormTemplate { id, project: to, created: stamp, updated: stamp, deleted: None, ..t });
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
    is_admin(ctx, r.project) || (r.status == FormStatus::Draft && (is_author(ctx, r) || (r.assignee.is_some() && r.assignee == ctx.person())))
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
        signers: t.signers.clone(),
        signatures: Vec::new(),
        assignee: None,
        schedule: None,
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
    if r.status == FormStatus::Submitted {
        // An admin's change to a submitted form voids the signatures on what was signed.
        r.signatures.clear();
    }
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
    let shown: Vec<String> = r.shown_fields().iter().map(|f| f.key.clone()).collect();
    let missing: Vec<&str> =
        r.fields.iter().filter(|f| f.required && shown.contains(&f.key) && !r.answers.contains_key(&f.key)).map(|f| f.label.as_str()).collect();
    if !missing.is_empty() {
        return Err(EngineError::conflict(format!("answer these first: {}", missing.join(", "))));
    }
    // Answers to questions the conditions left out are not part of the form.
    r.answers.retain(|k, _| shown.contains(k));
    r.status = FormStatus::Submitted;
    r.submitted = Some(ctx.stamp());
    r.updated = ctx.stamp();
    let id = r.id;
    ctx.state.form_records.put(r);
    Ok(json!({ "id": id }))
}

fn sign(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut r = load(ctx, p)?;
    if r.status != FormStatus::Submitted {
        return Err(EngineError::conflict("a form is signed once it is submitted"));
    }
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("a signature belongs to a person"))?;
    let role = p.req_line("role", 60)?;
    let Some(role) = r.signers.iter().find(|x| x.eq_ignore_ascii_case(&role)).cloned() else {
        return Err(p.bad("role", format!("{} has no signing role `{role}`", r.label())));
    };
    if r.signatures.iter().any(|s| s.role == role) {
        return Err(EngineError::conflict(format!("{role} has signed already")));
    }
    if r.signatures.iter().any(|s| s.person == me) {
        return Err(EngineError::conflict("you have signed this form in another role already"));
    }
    let name = p.req_line("name", 200)?;
    if !name.eq_ignore_ascii_case(&view::person_name(ctx.state, me)) {
        return Err(p.bad("name", "type your own full name to sign"));
    }
    r.signatures.push(FormSignature { role, person: me, name, stamp: ctx.stamp() });
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
    r.signatures.clear();
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

// -- recurring forms

fn live_schedule(ctx: &impl Read, project: Id<Project>, id: Id<FormSchedule>) -> Result<FormSchedule> {
    let s = in_project(&ctx.state().form_schedules, project, id, "form schedule")?;
    if s.deleted.is_some() {
        return Err(EngineError::not_found(format!("form schedule {id} not found in this project")));
    }
    Ok(s.clone())
}

fn schedule_save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let existing = match p.opt_id::<FormSchedule>("id")? {
        Some(id) => Some(live_schedule(ctx, project, id)?),
        None => None,
    };
    let template = live_template(ctx, project, p.id("template")?)?.id;
    let every = p
        .choice::<Repeat>("every")?
        .filter(|e| *e != Repeat::Never)
        .ok_or_else(|| p.bad("every", "must be daily, weekly, biweekly, monthly, month_end, quarterly or yearly"))?;
    let assignee: Id<Person> = p.id("assignee")?;
    ctx.check_person(assignee)?;
    let next = p.date("next")?.unwrap_or_else(|| ctx.today());
    let title = p.line_or_empty("title", text::SHORT)?;
    let active = p.flag("active")?.unwrap_or(true);
    let stamp = ctx.stamp();
    let id = match existing {
        Some(s) => {
            ctx.state
                .form_schedules
                .update(s.id, |x| (x.template, x.every, x.assignee, x.next, x.title, x.active) = (template, every, assignee, next, title, active));
            s.id
        }
        None => {
            if ctx.state.form_schedules.iter().filter(|s| s.project == project && s.deleted.is_none()).count() >= 100 {
                return Err(EngineError::conflict("a project keeps at most 100 form schedules"));
            }
            let id = ctx.state.form_schedules.alloc();
            ctx.state.form_schedules.put(FormSchedule { id, project, template, title, every, next, assignee, active, created: stamp, deleted: None });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn schedule_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let s = live_schedule(ctx, project, p.id("id")?)?;
    let stamp = ctx.stamp();
    ctx.state.form_schedules.update(s.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": s.id }))
}

/// The project's form schedules, by next day.
pub fn schedules(state: &CompanyState, project: Id<Project>) -> Vec<&FormSchedule> {
    let mut v: Vec<&FormSchedule> = state.form_schedules.iter().filter(|s| s.project == project && s.deleted.is_none()).collect();
    v.sort_by_key(|s| (s.next, s.id));
    v
}

fn schedule_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    json(&schedules(v.state, p.id("project")?))
}

fn generate_due(ctx: &mut Ctx<'_>, _p: &Params<'_>) -> Result<Value> {
    let today = ctx.today();
    let due: Vec<FormSchedule> = ctx.state.form_schedules.iter().filter(|s| s.deleted.is_none() && s.active && s.next <= today).cloned().collect();
    let mut made = 0usize;
    for s in due {
        // One form per pass however many days were missed; the next day moves past today.
        let mut next = s.next;
        for _ in 0..400 {
            if next > today {
                break;
            }
            match s.every.step(next) {
                Some(n) => next = n,
                None => break,
            }
        }
        let live = ctx.state.form_templates.get(s.template).filter(|t| t.deleted.is_none() && t.project == s.project).cloned();
        let Some(t) = live else {
            // The template is gone: stop the schedule rather than fail every pass.
            ctx.state.form_schedules.update(s.id, |x| x.active = false);
            continue;
        };
        let number = ctx.state.form_records.iter().filter(|r| r.project == s.project).map(|r| r.number).max().unwrap_or(0).saturating_add(1);
        let stamp = ctx.stamp();
        let id = ctx.state.form_records.alloc();
        let title = if s.title.is_empty() { today.to_string() } else { format!("{} {today}", s.title) };
        ctx.state.form_records.put(FormRecord {
            id,
            project: s.project,
            template: t.id,
            template_name: t.name.clone(),
            number,
            title: title.chars().take(text::SHORT).collect(),
            fields: t.fields.clone(),
            answers: BTreeMap::new(),
            status: FormStatus::Draft,
            attachments: Vec::new(),
            signers: t.signers.clone(),
            signatures: Vec::new(),
            assignee: Some(s.assignee),
            schedule: Some(s.id),
            submitted: None,
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        ctx.state.form_schedules.update(s.id, |x| x.next = next);
        let link = format!("/projects/{}/forms/{id}", s.project);
        notify(ctx, &[s.assignee], s.project, &format!("{} #{number} is ready to fill in", t.name), &link);
        made += 1;
    }
    Ok(json!({ "made": made }))
}

// -- report

/// One template's forms as rows: the question labels (the template's, then any a form had that it no longer has)
/// and, for each form, its answers under them.
pub struct FormReport<'a> {
    pub columns: Vec<String>,
    pub rows: Vec<(&'a FormRecord, Vec<String>)>,
}

pub fn form_report<'a>(
    state: &'a CompanyState,
    project: Id<Project>,
    template: Id<FormTemplate>,
    status: Option<FormStatus>,
    from: Option<Date>,
    to: Option<Date>,
) -> FormReport<'a> {
    let mut columns: Vec<String> = state.form_templates.get(template).map(|t| t.fields.iter().map(|f| f.label.clone()).collect()).unwrap_or_default();
    let mut recs: Vec<&FormRecord> = visible(state, project)
        .into_iter()
        .filter(|r| r.template == template && status.is_none_or(|s| r.status == s))
        .filter(|r| {
            let day = r.created.at.date_at(state.utc_offset());
            from.is_none_or(|f| day >= f) && to.is_none_or(|t| day <= t)
        })
        .collect();
    recs.sort_by_key(|r| r.number);
    for r in &recs {
        for f in &r.fields {
            if !columns.contains(&f.label) {
                columns.push(f.label.clone());
            }
        }
    }
    let rows = recs
        .into_iter()
        .map(|r| {
            let cells = columns
                .iter()
                .map(|c| r.fields.iter().find(|f| f.label == *c).and_then(|f| r.answers.get(&f.key)).cloned().unwrap_or_default())
                .collect();
            (r, cells)
        })
        .collect();
    FormReport { columns, rows }
}

fn report(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let template: Id<FormTemplate> = p.id("template")?;
    in_project(&v.state.form_templates, project, template, "form template")?;
    let r = form_report(v.state, project, template, p.choice::<FormStatus>("status")?, p.date("from")?, p.date("to")?);
    let rows: Vec<Value> = r
        .rows
        .iter()
        .map(|(rec, cells)| json!({ "id": rec.id, "number": rec.number, "title": rec.title, "status": rec.status, "started": rec.created.at, "answers": cells }))
        .collect();
    Ok(json!({ "columns": r.columns, "rows": rows }))
}
