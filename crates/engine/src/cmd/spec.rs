//! Specification sections: a project's list of section numbers and titles.
//!
//! The list is the project's own (typed in or imported); other tools, such as submittals, link to
//! a section by its number.

use buildkiosk_core::{Id, text};
use buildkiosk_model::project::Project;
use buildkiosk_model::spec::SpecSection;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "spec.add",
        "Add specification section",
        Tool::Specifications,
        Access::Project(Level::Standard),
        "project: project id; number: text; title?: text",
        add,
    ),
    CommandSpec::mutate(
        "spec.import",
        "Import specification sections",
        Tool::Specifications,
        Access::Project(Level::Standard),
        "project: project id; text: lines of `number, title` (comma or tab separated; blank lines and lines starting with # are skipped)",
        import,
    ),
    CommandSpec::mutate(
        "spec.delete",
        "Delete specification section",
        Tool::Specifications,
        Access::Project(Level::Standard),
        "project: project id; id: section id",
        delete,
    ),
    CommandSpec::query("spec.list", "List specification sections", Tool::Specifications, Access::Project(Level::Read), "project: project id", list),
];

/// Most sections one import reads.
const MAX_IMPORT: usize = 5_000;

fn exists(ctx: &Ctx<'_>, project: Id<Project>, number: &str) -> bool {
    ctx.state.spec_sections.iter().any(|s| s.project == project && s.deleted.is_none() && s.number.eq_ignore_ascii_case(number))
}

fn insert(ctx: &mut Ctx<'_>, project: Id<Project>, number: String, title: String) -> Id<SpecSection> {
    let id = ctx.state.spec_sections.alloc();
    let created = ctx.stamp();
    ctx.state.spec_sections.put(SpecSection { id, project, number, title, created, deleted: None });
    id
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = p.req_line("number", 50)?;
    if exists(ctx, project, &number) {
        return Err(EngineError::conflict(format!("section {number} is already in this project's list")));
    }
    let title = p.line_or_empty("title", text::SHORT)?;
    let id = insert(ctx, project, number, title);
    Ok(json!({ "id": id }))
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let body = p.req_text("text", 500_000)?;
    let (mut added, mut skipped) = (0usize, 0usize);
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if added + skipped >= MAX_IMPORT {
            return Err(p.bad("text", format!("has more than {MAX_IMPORT} sections")));
        }
        let (number, title) = line.split_once('\t').or_else(|| line.split_once(',')).unwrap_or((line, ""));
        let number = text::required_line(number.trim_matches('"'), 50).map_err(|e| p.bad("text", format!("line {}: number {e}", n + 1)))?;
        let title = text::line(title.trim().trim_matches('"'), text::SHORT).map_err(|e| p.bad("text", format!("line {}: title {e}", n + 1)))?;
        if exists(ctx, project, &number) {
            skipped += 1;
        } else {
            insert(ctx, project, number, title);
            added += 1;
        }
    }
    Ok(json!({ "added": added, "skipped": skipped }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<SpecSection> = p.id("id")?;
    in_project(&ctx.state.spec_sections, project, id, "specification section")?;
    let stamp = ctx.stamp();
    ctx.state.spec_sections.update(id, |s| s.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&SpecSection> = v.state.spec_sections.iter().filter(|s| s.project == project && s.deleted.is_none()).collect();
    list.sort_by(|a, b| a.number.cmp(&b.number));
    json(&list)
}
