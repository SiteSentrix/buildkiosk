//! Drawings: a project's sheet log with revisions.
//!
//! A sheet is identified by its number. Each upload of a newer issue is a revision; the newest is
//! current and older ones stay as history. A sheet can be logged before its file arrives.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::{DistributionGroup, Person};
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
        "drawing.publish",
        "Tell people a sheet revision was issued",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; id: drawing id; people?: [person ids]; groups?: [distribution group ids] (their members are told too); message?: text (the newest revision is announced)",
        publish,
    ),
    CommandSpec::mutate(
        "drawing.markup_add",
        "Mark up a sheet",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project: project id; id: drawing id; revision?: label (default the current one); page?: page number from 1 (default 1); kind: pin|rect|arrow|text|dim|area|polyline|polygon|count (one counted item; text names it); \
         x, y: 0..10000 across and down the page (not needed for polyline and polygon); points?: for polyline (a length along a path, 2 to 200 points) and polygon (the area inside an outline, 3 to 200), `x,y x,y x,y` in order, sides that meet only at corners; x2?, y2?: the far corner of a box or area, the head of an arrow, or the far end of a length (required for those); \
         text?: the note (required for text); cost_code?: the cost code a measurement or count is for. A length (dim, polyline) or area (area, polygon) needs the page's scale (drawing.scale_set) and is worked out from it.",
        markup_add,
    ),
    CommandSpec::mutate(
        "drawing.markup_code",
        "Set the cost code of a measurement or count",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project, id: ids; revision?: label (default the current one); markup: markup number; cost_code?: cost code id (empty clears it)",
        markup_code,
    ),
    CommandSpec::query(
        "drawing.quantities",
        "Quantities measured and counted on sheets",
        Tool::Drawings,
        Access::Project(Level::Read),
        "project: project id; drawing?: one sheet (default every sheet); the current revision of each, added up by cost code, kind and label",
        quantities,
    ),
    CommandSpec::mutate(
        "drawing.scale_set",
        "Set the scale of a sheet page from a stretch of known length",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project, id: ids; revision?: label (default the current one); page?: page number from 1 (default 1); \
         x, y, x2, y2: 0..10000 the two ends of a stretch drawn on the page (at least 1% of the page's height long); \
         aspect: the page's width over its height in thousandths (100..100000; 1294 for a letter page on its side); \
         length: the stretch's real length (a number, up to three decimals); unit: ft|in|m|mm. \
         Lengths and areas already measured on the page are worked out again.",
        scale_set,
    ),
    CommandSpec::mutate(
        "drawing.scale_clear",
        "Remove the scale of a sheet page",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project, id: ids; revision?: label (default the current one); page?: page number from 1 (default 1). Measurements on the page keep their lines but lose their values.",
        scale_clear,
    ),
    CommandSpec::mutate(
        "drawing.markup_resolve",
        "Resolve or reopen a markup",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project, id: ids; revision?: label (default the current one); markup: markup number; resolved?: bool (default true)",
        markup_resolve,
    ),
    CommandSpec::mutate(
        "drawing.markup_delete",
        "Remove a markup",
        Tool::Drawings,
        Access::Project(Level::Standard),
        "project, id: ids; revision?: label (default the current one); markup: markup number (its author or an admin)",
        markup_delete,
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
        markups: Vec::new(),
        scales: Vec::new(),
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

const MAX_MARKUPS: usize = 500;

/// The index of the revision named by `revision` (default: the newest) in the sheet.
fn revision_index(d: &Drawing, p: &Params<'_>) -> Result<usize> {
    match p.line("revision", 50)?.filter(|r| !r.is_empty()) {
        Some(label) => d
            .revisions
            .iter()
            .position(|r| r.label.eq_ignore_ascii_case(&label))
            .ok_or_else(|| p.bad("revision", format!("{} has no revision {label}", d.number))),
        None => d.revisions.len().checked_sub(1).ok_or_else(|| EngineError::conflict(format!("{} has no revision to mark up yet", d.number))),
    }
}

/// The corners of a path or outline, written `x,y x,y x,y` (spaces or semicolons between them).
fn parse_points(p: &Params<'_>, kind: buildkiosk_model::drawing::MarkupKind) -> Result<Vec<(u16, u16)>> {
    use buildkiosk_model::drawing::{MARKUP_SCALE, MAX_POINTS, MarkupKind, outline_is_simple};
    let raw = p.text_or_empty("points", 10_000)?;
    let mut out: Vec<(u16, u16)> = Vec::new();
    for part in raw.split(|c: char| c.is_whitespace() || c == ';').filter(|s| !s.is_empty()) {
        let bad = || p.bad("points", format!("`{part}` is not a point written x,y with each from 0 to {MARKUP_SCALE}"));
        let (a, b) = part.split_once(',').ok_or_else(bad)?;
        let num = |s: &str| s.trim().parse::<u16>().ok().filter(|v| *v <= MARKUP_SCALE);
        out.push((num(a).ok_or_else(bad)?, num(b).ok_or_else(bad)?));
        if out.len() > MAX_POINTS {
            return Err(p.bad("points", format!("has more than {MAX_POINTS} points")));
        }
    }
    let least = if kind == MarkupKind::Polygon { 3 } else { 2 };
    if out.len() < least {
        return Err(p.bad("points", format!("needs at least {least} points")));
    }
    if !outline_is_simple(&out, kind == MarkupKind::Polygon) {
        return Err(EngineError::conflict("the path crosses itself, repeats a point or doubles back; draw it so its sides only meet at corners"));
    }
    Ok(out)
}

fn markup_add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    use buildkiosk_model::drawing::{MARKUP_SCALE, Markup, MarkupKind};
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let kind = p.choice::<MarkupKind>("kind")?.ok_or_else(|| p.bad("kind", "is required"))?;
    let coord = |key: &str, required: bool| -> Result<u16> {
        match p.int(key, 0..=i64::from(MARKUP_SCALE))? {
            Some(v) => Ok(u16::try_from(v).unwrap_or(0)),
            None if required => Err(p.bad(key, "is required")),
            None => Ok(0),
        }
    };
    let points = if kind.is_path() { parse_points(p, kind)? } else { Vec::new() };
    let (x, y, x2, y2) = if let (Some(a), Some(b)) = (points.first(), points.get(1)) {
        (a.0, a.1, b.0, b.1)
    } else {
        let (x, y) = (coord("x", true)?, coord("y", true)?);
        let two_points = matches!(kind, MarkupKind::Rectangle | MarkupKind::Arrow | MarkupKind::Dimension | MarkupKind::Area);
        (x, y, coord("x2", two_points)?, coord("y2", two_points)?)
    };
    let text = p.text_or_empty("text", 1_000)?;
    if kind == MarkupKind::Text && text.trim().is_empty() {
        return Err(p.bad("text", "is required for a text markup"));
    }
    let page = p.int("page", 1..=2_000)?.and_then(|v| u32::try_from(v).ok()).unwrap_or(1);
    let (person, now) = (ctx.person(), ctx.at);
    let Some(rev) = d.revisions.get_mut(at) else { return Err(EngineError::not_found("revision not found")) };
    if rev.markups.len() >= MAX_MARKUPS {
        return Err(EngineError::conflict(format!("a revision holds at most {MAX_MARKUPS} markups")));
    }
    let id = rev.markups.iter().map(|m| m.id).max().unwrap_or(0).saturating_add(1);
    let (mut measure, mut unit) = (0, None);
    if kind.measures_length() || kind.measures_area() {
        let Some(scale) = rev.scales.iter().find(|s| s.page == page) else {
            return Err(EngineError::conflict(format!("set the scale of page {page} first (drawing.scale_set)")));
        };
        let worked = scale.measure_markup(kind, x, y, x2, y2, &points);
        measure = worked.ok_or_else(|| EngineError::conflict("that is too large to measure"))?;
        unit = Some(scale.unit);
    }
    let cost_code = markup_code_param(ctx, d.project, p)?;
    let Some(rev) = d.revisions.get_mut(at) else { return Err(EngineError::not_found("revision not found")) };
    rev.markups.push(Markup { id, page, kind, x, y, x2, y2, points, text, resolved: false, measure, unit, cost_code, by: person, at: now });
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did, "markup": id }))
}

/// The cost code in `cost_code`, which must be usable on the project.
fn markup_code_param(ctx: &Ctx<'_>, project: Id<Project>, p: &Params<'_>) -> Result<Option<Id<buildkiosk_model::project::CostCode>>> {
    if !p.present("cost_code") || p.text_or_empty("cost_code", 40)?.trim().is_empty() {
        return Ok(None);
    }
    let code: Id<buildkiosk_model::project::CostCode> = p.id("cost_code")?;
    super::budget::usable_code(ctx.state, project, code)?;
    Ok(Some(code))
}

fn markup_code(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let number = p.int("markup", 1..=i64::from(u32::MAX))?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("markup", "is required"))?;
    let code = markup_code_param(ctx, d.project, p)?;
    let Some(m) = d.revisions.get_mut(at).and_then(|r| r.markups.iter_mut().find(|m| m.id == number)) else {
        return Err(EngineError::not_found(format!("markup {number} not found")));
    };
    if !(m.kind.measures_length()
        || m.kind.measures_area()
        || matches!(m.kind, buildkiosk_model::drawing::MarkupKind::Count | buildkiosk_model::drawing::MarkupKind::Pin))
    {
        return Err(EngineError::conflict("only measurements, counts and pins have a cost code"));
    }
    m.cost_code = code;
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did }))
}

/// What was measured and counted, added up by cost code, kind and label, on the current revision of each sheet.
fn quantities(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let one: Option<Id<Drawing>> = if p.present("drawing") { Some(p.id("drawing")?) } else { None };
    let rows = super::estimate::quantity_rows(v.state, project, one)?;
    Ok(Value::Array(
        rows.iter()
            .map(|r| {
                let code = r.cost_code.and_then(|c| v.state.cost_codes.get(c)).map(|c| c.display());
                json!({
                    "drawing": r.drawing, "sheet": r.sheet, "kind": r.kind, "label": r.label, "cost_code": r.cost_code, "code": code,
                    "count": r.count, "quantity": r.quantity, "unit": r.unit,
                })
            })
            .collect(),
    ))
}

/// The page number from 1 in `page` (default 1).
fn page_number(p: &Params<'_>) -> Result<u32> {
    Ok(p.int("page", 1..=2_000)?.and_then(|v| u32::try_from(v).ok()).unwrap_or(1))
}

/// Works the lengths and areas on a page out again from its scale (or clears them when it has none).
fn remeasure(rev: &mut buildkiosk_model::drawing::DrawingRevision, page: u32) {
    let scale = rev.scales.iter().find(|s| s.page == page).cloned();
    for m in rev.markups.iter_mut().filter(|m| m.page == page && (m.kind.measures_length() || m.kind.measures_area())) {
        let worked = scale.as_ref().and_then(|s| s.measure_markup(m.kind, m.x, m.y, m.x2, m.y2, &m.points));
        m.measure = worked.unwrap_or(0);
        m.unit = worked.and(scale.as_ref().map(|s| s.unit));
    }
}

fn scale_set(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    use buildkiosk_model::drawing::{LengthUnit, MARKUP_SCALE, MAX_ASPECT, MIN_ASPECT, PageScale};
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let page = page_number(p)?;
    let coord = |key: &str| -> Result<u16> {
        let v = p.int(key, 0..=i64::from(MARKUP_SCALE))?.ok_or_else(|| p.bad(key, "is required"))?;
        Ok(u16::try_from(v).unwrap_or(0))
    };
    let (x, y, x2, y2) = (coord("x")?, coord("y")?, coord("x2")?, coord("y2")?);
    let aspect = p
        .int("aspect", i64::from(MIN_ASPECT)..=i64::from(MAX_ASPECT))?
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| p.bad("aspect", "is required"))?;
    let unit = p.choice::<LengthUnit>("unit")?.ok_or_else(|| p.bad("unit", "is required (ft, in, m or mm)"))?;
    let text = p.line("length", 30)?.ok_or_else(|| p.bad("length", "is required"))?;
    let length_um = unit.parse_length(&text).ok_or_else(|| p.bad("length", "must be a positive number with at most three decimals, up to 10 km"))?;
    let scale = PageScale { page, x, y, x2, y2, aspect, length_um, unit, by: ctx.person(), at: ctx.at };
    if !scale.usable() {
        return Err(p.bad("x2", "the stretch must be at least 1% of the page's height long"));
    }
    let Some(rev) = d.revisions.get_mut(at) else { return Err(EngineError::not_found("revision not found")) };
    rev.scales.retain(|s| s.page != page);
    rev.scales.push(scale);
    remeasure(rev, page);
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did }))
}

fn scale_clear(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let page = page_number(p)?;
    let Some(rev) = d.revisions.get_mut(at) else { return Err(EngineError::not_found("revision not found")) };
    if !rev.scales.iter().any(|s| s.page == page) {
        return Err(EngineError::not_found(format!("page {page} has no scale")));
    }
    rev.scales.retain(|s| s.page != page);
    remeasure(rev, page);
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did }))
}

fn markup_resolve(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let number = p.int("markup", 1..=i64::from(u32::MAX))?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("markup", "is required"))?;
    let resolved = p.flag_or("resolved", true)?;
    let Some(m) = d.revisions.get_mut(at).and_then(|r| r.markups.iter_mut().find(|m| m.id == number)) else {
        return Err(EngineError::not_found(format!("markup {number} not found")));
    };
    m.resolved = resolved;
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did }))
}

fn markup_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let at = revision_index(&d, p)?;
    let number = p.int("markup", 1..=i64::from(u32::MAX))?.and_then(|v| u32::try_from(v).ok()).ok_or_else(|| p.bad("markup", "is required"))?;
    let admin = ctx.level(d.project, Tool::Drawings) == Level::Admin;
    let me = ctx.person();
    let Some(rev) = d.revisions.get_mut(at) else { return Err(EngineError::not_found("revision not found")) };
    let Some(pos) = rev.markups.iter().position(|m| m.id == number) else { return Err(EngineError::not_found(format!("markup {number} not found"))) };
    if !admin && rev.markups.get(pos).and_then(|m| m.by) != me {
        return Err(EngineError::forbidden("only the person who drew a markup or a Drawings admin can remove it"));
    }
    rev.markups.remove(pos);
    d.updated = ctx.stamp();
    let did = d.id;
    ctx.state.drawings.put(d);
    Ok(json!({ "id": did }))
}

fn publish(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let d = load(ctx, p)?;
    let Some(rev) = d.current() else { return Err(EngineError::conflict(format!("{} has no revision to issue yet", d.number))) };
    let mut people: Vec<Id<Person>> = p.ids("people")?;
    ctx.check_people(&people)?;
    let groups: Vec<Id<DistributionGroup>> = p.ids("groups")?;
    for g in groups {
        let group = ctx
            .state
            .groups
            .get(g)
            .filter(|x| x.deleted.is_none() && x.project.is_none_or(|gp| gp == d.project))
            .ok_or_else(|| p.bad("groups", format!("group {g} not found")))?;
        for m in &group.members {
            if !people.contains(m) && people.len() < 200 {
                people.push(*m);
            }
        }
    }
    if people.is_empty() {
        return Err(p.bad("people", "choose at least one person or group to tell"));
    }
    let message = p.line_or_empty("message", 300)?;
    let text = format!(
        "Sheet {}{} revision {} was issued{}{}",
        d.number,
        if d.title.is_empty() { String::new() } else { format!(" ({})", d.title) },
        rev.label,
        if rev.set.is_empty() { String::new() } else { format!(" in {}", rev.set) },
        if message.is_empty() { String::new() } else { format!(": {message}") }
    );
    let link = format!("/projects/{}/drawings/{}", d.project, d.id);
    super::notification::notify(ctx, &people, d.project, &text, &link);
    Ok(json!({ "id": d.id, "told": people.len() }))
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
