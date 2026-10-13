//! Drawings: a project's sheet log with revisions.
//!
//! A sheet is identified by its number. Each upload of a newer issue is a revision; the newest is
//! current and older ones stay as history. A sheet can be logged before its file arrives.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::drawing::{Discipline, Drawing, DrawingRevision};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Choice, Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::json;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "drawing.add",
        "Add drawing",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; number: sheet number; title?: text; discipline?: general|civil|landscape|architectural|interior|structural|mechanical|plumbing|fire_protection|electrical|other; area?: text; revision?: label (with sha256, name, size, mime? to add its file); date?: YYYY-MM-DD; set?: text; note?: text",
        add,
    ),
    CommandSpec::mutate(
        "drawing.add_revision",
        "Add revision",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; id: drawing id; revision: label; date?: YYYY-MM-DD; set?: text; note?: text; sha256?: 64 hex digits; name?: file name; size?: bytes; mime?: media type (the file may come later: it can't be added afterwards)",
        add_revision,
    ),
    CommandSpec::mutate(
        "drawing.update",
        "Edit drawing",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; id: drawing id; number?: text; title?: text; discipline?: as in drawing.add; area?: text",
        update,
    ),
    CommandSpec::mutate(
        "drawing.import",
        "Import drawing log",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; text: lines of `number, title, discipline, area` (comma or tab separated; # lines and blank lines are skipped; sheets already logged are skipped)",
        import,
    ),
    CommandSpec::mutate(
        "drawing.delete",
        "Delete drawing",
        Tool::Drawings,
        Access::Project(Level::Admin),
        "project: project id; id: drawing id",
        delete,
    ),
    CommandSpec::mutate(
        "drawing.restore",
        "Restore drawing",
        Tool::Drawings,
        Access::Project(Level::Admin),
        "project: project id; id: drawing id",
        restore,
    ),
    CommandSpec::query(
        "drawing.list",
        "List drawings",
        Tool::Drawings,
        Access::Project(Level::Read),
        "project: project id; q?: text; discipline?: as in drawing.add; area?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query(
        "drawing.get",
        "Get drawing",
        Tool::Drawings,
        Access::Project(Level::Read),
        "project: project id; id: drawing id (with every revision)",
        get,
    ),
];

const MAX_REVISIONS: usize = 100;
const MAX_IMPORT: usize = 5_000;
const MAX_DRAWINGS: usize = 20_000;

fn number_taken(ctx: &Ctx<'_>, project: Id<Project>, number: &str, except: Option<Id<Drawing>>) -> bool {
    ctx.state.drawings.iter().any(|d| d.project == project && d.deleted.is_none() && Some(d.id) != except && d.number.eq_ignore_ascii_case(number))
}

fn file(p: &Params<'_>) -> Result<Option<FileRef>> {
    if p.line("sha256", 64)?.is_none_or(|s| s.is_empty()) {
        return Ok(None);
    }
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    Ok(Some(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime }))
}

fn revision(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<DrawingRevision> {
    let label = p.req_line("revision", 50)?;
    let date: Option<Date> = p.date("date")?;
    Ok(DrawingRevision {
        label,
        date,
        set: p.line_or_empty("set", 100)?,
        note: p.text_or_empty("note", text::SHORT)?,
        file: file(p)?,
        by: ctx.person(),
        at: ctx.at,
    })
}

fn discipline(p: &Params<'_>) -> Result<Option<Discipline>> {
    p.choice::<Discipline>("discipline")
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let number = p.req_line("number", 50)?;
    if number_taken(ctx, project, &number, None) {
        return Err(EngineError::conflict(format!("sheet {number} is already in the log; add a revision to it instead")));
    }
    if ctx.state.drawings.iter().filter(|d| d.project == project).count() >= MAX_DRAWINGS {
        return Err(EngineError::conflict(format!("a project's log holds at most {MAX_DRAWINGS} sheets")));
    }
    let revisions = if p.line("revision", 50)?.is_some_and(|r| !r.is_empty()) { vec![revision(ctx, p)?] } else { Vec::new() };
    let id = ctx.state.drawings.alloc();
    let stamp = ctx.stamp();
    ctx.state.drawings.put(Drawing {
        id,
        project,
        number,
        title: p.line_or_empty("title", text::SHORT)?,
        discipline: discipline(p)?.unwrap_or_default(),
        area: p.line_or_empty("area", 100)?,
        revisions,
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Drawing> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Drawing> = p.id("id")?;
    let d = in_project(&ctx.state.drawings, project, id, "drawing")?;
    if d.deleted.is_some() {
        return Err(EngineError::not_found(format!("drawing {id} not found in this project")));
    }
    Ok(d.clone())
}

fn add_revision(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let rev = revision(ctx, p)?;
    if d.revisions.iter().any(|r| r.label.eq_ignore_ascii_case(&rev.label)) {
        return Err(EngineError::conflict(format!("{} already has a revision {}", d.number, rev.label)));
    }
    if d.revisions.len() >= MAX_REVISIONS {
        return Err(EngineError::conflict(format!("a sheet keeps at most {MAX_REVISIONS} revisions")));
    }
    d.revisions.push(rev);
    d.updated = ctx.stamp();
    let id = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    if p.present("number") {
        let number = p.req_line("number", 50)?;
        if number_taken(ctx, d.project, &number, Some(d.id)) {
            return Err(EngineError::conflict(format!("sheet {number} is already in the log")));
        }
        d.number = number;
    }
    if p.present("title") {
        d.title = p.line_or_empty("title", text::SHORT)?;
    }
    if let Some(x) = discipline(p)? {
        d.discipline = x;
    }
    if p.present("area") {
        d.area = p.line_or_empty("area", 100)?;
    }
    d.updated = ctx.stamp();
    let id = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": id }))
}

fn import(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let body = p.req_text("text", 1_000_000)?;
    let (mut added, mut skipped, mut seen) = (0usize, 0usize, 0usize);
    for (n, raw) in body.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        seen += 1;
        if seen > MAX_IMPORT {
            return Err(p.bad("text", format!("has more than {MAX_IMPORT} sheets")));
        }
        let sep = if line.contains('\t') { '\t' } else { ',' };
        let cells: Vec<&str> = line.split(sep).map(|c| c.trim().trim_matches('"')).collect();
        let cell = |i: usize| cells.get(i).copied().unwrap_or("");
        let at = |what: &str, e: String| p.bad("text", format!("line {}: {what} {e}", n + 1));
        let number = text::required_line(cell(0), 50).map_err(|e| at("number", e.to_string()))?;
        if number.eq_ignore_ascii_case("number") || number.eq_ignore_ascii_case("sheet") {
            seen -= 1;
            continue; // a header line
        }
        let title = text::line(cell(1), text::SHORT).map_err(|e| at("title", e.to_string()))?;
        let disc = match cell(2) {
            "" => Discipline::default(),
            s => Discipline::parse(s).ok_or_else(|| {
                at("discipline", format!("`{s}` is not one of {}", Discipline::ALL.iter().map(|d| d.key()).collect::<Vec<_>>().join(", ")))
            })?,
        };
        let area = text::line(cell(3), 100).map_err(|e| at("area", e.to_string()))?;
        if number_taken(ctx, project, &number, None) {
            skipped += 1;
            continue;
        }
        let id = ctx.state.drawings.alloc();
        let stamp = ctx.stamp();
        ctx.state.drawings.put(Drawing {
            id,
            project,
            number,
            title,
            discipline: disc,
            area,
            revisions: Vec::new(),
            created: stamp,
            updated: stamp,
            deleted: None,
        });
        added += 1;
    }
    Ok(json!({ "added": added, "skipped": skipped }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let d = load(ctx, p)?;
    let stamp = ctx.stamp();
    ctx.state.drawings.update(d.id, |d| d.deleted = Some(stamp));
    Ok(json!({ "id": d.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Drawing> = p.id("id")?;
    let d = in_project(&ctx.state.drawings, project, id, "drawing")?;
    if d.deleted.is_none() {
        return Err(EngineError::conflict("the drawing is not in the recycle bin"));
    }
    if number_taken(ctx, project, &d.number.clone(), Some(id)) {
        return Err(EngineError::conflict("another sheet now uses that number"));
    }
    ctx.state.drawings.update(id, |d| d.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let disc = discipline(p)?;
    let area = p.line("area", 100)?.filter(|a| !a.is_empty());
    let bin = p.flag_or("deleted", false)?;
    if bin && v.level(project, Tool::Drawings) < Level::Admin {
        return Err(EngineError::forbidden("only admins can see the recycle bin"));
    }
    let mut out: Vec<&Drawing> = v
        .state
        .drawings
        .iter()
        .filter(|d| d.project == project && d.deleted.is_some() == bin)
        .filter(|d| disc.is_none_or(|x| d.discipline == x))
        .filter(|d| area.as_ref().is_none_or(|a| d.area.eq_ignore_ascii_case(a)))
        .filter(|d| q.as_ref().is_none_or(|q| d.number.to_lowercase().contains(q.as_str()) || d.title.to_lowercase().contains(q.as_str())))
        .collect();
    out.sort_by_key(|d| d.number.to_lowercase());
    json(&out)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Drawing> = p.id("id")?;
    let d = in_project(&v.state.drawings, project, id, "drawing")?;
    if d.deleted.is_some() {
        return Err(EngineError::not_found(format!("drawing {id} not found in this project")));
    }
    json(d)
}
