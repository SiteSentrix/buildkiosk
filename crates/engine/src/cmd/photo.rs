//! Photos: pictures of the work with a caption, album, place and day taken.
//!
//! The server stores the picture and checks that its bytes really are a JPEG, PNG, GIF or WebP
//! before it passes the type here; the command refuses any other type, so only those four are ever
//! shown inline.

use buildkiosk_core::{Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::observation::Observation;
use buildkiosk_model::photo::{IMAGE_TYPES, Photo};
use buildkiosk_model::project::Project;
use buildkiosk_model::punch::PunchItem;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::workitem::WorkItem;
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "photo.add",
        "Add photo",
        Tool::Photos,
        Access::Project(Level::Standard),
        "project: project id; sha256: 64 hex digits; name: file name; size: bytes; mime: image/jpeg|image/png|image/gif|image/webp; caption?: text; album?: text; location?: text; taken?: YYYY-MM-DD (default today); width?, height?: pixels; camera?: text",
        add,
    ),
    CommandSpec::mutate(
        "photo.update",
        "Edit photo",
        Tool::Photos,
        Access::Project(Level::Standard),
        "project, id: ids; caption?: text; album?: text; location?: text; taken?: YYYY-MM-DD (the person who added it, or an admin)",
        update,
    ),
    CommandSpec::mutate(
        "photo.delete",
        "Delete photo",
        Tool::Photos,
        Access::Project(Level::Standard),
        "project, id: ids (the person who added it, or an admin)",
        delete,
    ),
    CommandSpec::mutate(
        "photo.link",
        "Tag photo to an item",
        Tool::Photos,
        Access::Project(Level::Standard),
        "project, id: ids of the photo; kind: rfi|observation|punch and item: the item's id (or one tag such as `rfi:12`); remove?: bool (the person who added the photo, or an admin; you must be able to see the item)",
        link,
    ),
    CommandSpec::mutate("photo.restore", "Restore photo", Tool::Photos, Access::Project(Level::Admin), "project, id: ids", restore),
    CommandSpec::query(
        "photo.list",
        "List photos",
        Tool::Photos,
        Access::Project(Level::Read),
        "project: project id; album?: text; q?: text (caption, album or place); rfi?, observation?, punch?: item id (photos tagged to it); deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query("photo.get", "Show photo", Tool::Photos, Access::Project(Level::Read), "project, id: ids", get),
];

const MAX_PHOTOS: usize = 50_000;

fn is_admin(ctx: &Ctx<'_>, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Photos) == Level::Admin
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Photo> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Photo> = p.id("id")?;
    let ph = in_project(&ctx.state.photos, project, id, "photo")?;
    if ph.deleted.is_some() {
        return Err(EngineError::not_found(format!("photo {id} not found in this project")));
    }
    Ok(ph.clone())
}

fn can_change(ctx: &Ctx<'_>, ph: &Photo) -> bool {
    is_admin(ctx, ph.project) || (ctx.person().is_some() && ph.created.by == ctx.person())
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.req_line("mime", 100)?.to_ascii_lowercase();
    if !IMAGE_TYPES.contains(&mime.as_str()) {
        return Err(p.bad("mime", "photos must be JPEG, PNG, GIF or WebP pictures"));
    }
    if ctx.state.photos.iter().filter(|x| x.project == project).count() >= MAX_PHOTOS {
        return Err(EngineError::conflict(format!("a project holds at most {MAX_PHOTOS} photos")));
    }
    let taken = if p.present("taken") { p.date("taken")? } else { Some(ctx.today()) };
    let id = ctx.state.photos.alloc();
    let stamp = ctx.stamp();
    ctx.state.photos.put(Photo {
        id,
        project,
        file: FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime },
        caption: p.line_or_empty("caption", text::SHORT)?,
        album: p.line_or_empty("album", 100)?,
        location: p.line_or_empty("location", text::SHORT)?,
        taken,
        width: u32::try_from(p.int("width", 0..=100_000)?.unwrap_or(0)).unwrap_or(0),
        height: u32::try_from(p.int("height", 0..=100_000)?.unwrap_or(0)).unwrap_or(0),
        camera: p.line_or_empty("camera", 100)?,
        rfis: Vec::new(),
        observations: Vec::new(),
        punch_items: Vec::new(),
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut ph = load(ctx, p)?;
    if !can_change(ctx, &ph) {
        return Err(EngineError::forbidden("only the person who added a photo or a Photos admin can edit it"));
    }
    if p.present("caption") {
        ph.caption = p.line_or_empty("caption", text::SHORT)?;
    }
    if p.present("album") {
        ph.album = p.line_or_empty("album", 100)?;
    }
    if p.present("location") {
        ph.location = p.line_or_empty("location", text::SHORT)?;
    }
    if p.present("taken") {
        ph.taken = p.date("taken")?;
    }
    ph.updated = ctx.stamp();
    let id = ph.id;
    ctx.state.photos.put(ph);
    Ok(json!({ "id": id }))
}

const MAX_TAGS: usize = 50;

fn link(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut ph = load(ctx, p)?;
    if !can_change(ctx, &ph) {
        return Err(EngineError::forbidden("only the person who added a photo or a Photos admin can tag it"));
    }
    // Either `kind` and `item`, or one `tag` such as `rfi:12` (what a page's drop-down sends).
    let (kind, item) = match p.line("tag", 40)?.filter(|t| !t.is_empty()) {
        Some(t) => {
            let (k, n) = t.split_once(':').ok_or_else(|| p.bad("tag", "must look like rfi:12"))?;
            (k.to_string(), n.trim().parse::<u64>().map_err(|_| p.bad("tag", "must look like rfi:12"))?)
        }
        None => (p.req_line("kind", 20)?, p.id::<Photo>("item")?.raw()),
    };
    let remove = p.flag("remove")?.unwrap_or(false);
    let project = ph.project;
    let missing = |what: &str| EngineError::not_found(format!("{what} not found in this project"));
    match kind.as_str() {
        "rfi" => {
            let id: Id<buildkiosk_model::rfi::Rfi> = Id::new(item);
            let ok = in_project(&ctx.state.rfis, project, id, "RFI")
                .ok()
                .is_some_and(|r| r.deleted.is_none() && view::can_see_rfi(ctx.state, ctx.actor, r));
            if !remove && !ok {
                return Err(missing("RFI"));
            }
            ph.rfis.retain(|x| *x != id);
            if !remove {
                ph.rfis.push(id);
            }
        }
        "observation" => {
            let id: Id<Observation> = Id::new(item);
            let ok = in_project(&ctx.state.observations, project, id, "observation")
                .ok()
                .is_some_and(|o| o.deleted.is_none() && o.visible_to(ctx.state, ctx.actor));
            if !remove && !ok {
                return Err(missing("observation"));
            }
            ph.observations.retain(|x| *x != id);
            if !remove {
                ph.observations.push(id);
            }
        }
        "punch" => {
            let id: Id<PunchItem> = Id::new(item);
            let ok = in_project(&ctx.state.punch_items, project, id, "punch item")
                .ok()
                .is_some_and(|o| o.deleted.is_none() && o.visible_to(ctx.state, ctx.actor));
            if !remove && !ok {
                return Err(missing("punch item"));
            }
            ph.punch_items.retain(|x| *x != id);
            if !remove {
                ph.punch_items.push(id);
            }
        }
        other => return Err(p.bad("kind", format!("`{other}` is not rfi, observation or punch"))),
    }
    if ph.rfis.len() + ph.observations.len() + ph.punch_items.len() > MAX_TAGS {
        return Err(EngineError::conflict(format!("a photo can be tagged to at most {MAX_TAGS} items")));
    }
    ph.updated = ctx.stamp();
    let id = ph.id;
    ctx.state.photos.put(ph);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let ph = load(ctx, p)?;
    if !can_change(ctx, &ph) {
        return Err(EngineError::forbidden("only the person who added a photo or a Photos admin can delete it"));
    }
    let stamp = ctx.stamp();
    ctx.state.photos.update(ph.id, |x| x.deleted = Some(stamp));
    Ok(json!({ "id": ph.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Photo> = p.id("id")?;
    let ph = in_project(&ctx.state.photos, project, id, "photo")?;
    if ph.deleted.is_none() {
        return Err(EngineError::conflict("the photo is not in the recycle bin"));
    }
    ctx.state.photos.update(id, |x| x.deleted = None);
    Ok(json!({ "id": id }))
}

/// The project's photos, newest first.
pub fn visible(state: &buildkiosk_model::CompanyState, project: Id<Project>) -> Vec<&Photo> {
    let mut v: Vec<&Photo> = state.photos.iter().filter(|x| x.project == project && x.deleted.is_none()).collect();
    v.sort_by_key(|x| std::cmp::Reverse((x.taken, x.id)));
    v
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    if p.flag_or("deleted", false)? {
        if v.level(project, Tool::Photos) < Level::Admin {
            return Err(EngineError::forbidden("only admins can see the recycle bin"));
        }
        let bin: Vec<&Photo> = v.state.photos.iter().filter(|x| x.project == project && x.deleted.is_some()).collect();
        return json(&bin);
    }
    let album = p.line("album", 100)?.filter(|a| !a.is_empty());
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let rfi: Option<Id<buildkiosk_model::rfi::Rfi>> = p.opt_id("rfi")?;
    let obs: Option<Id<Observation>> = p.opt_id("observation")?;
    let punch: Option<Id<PunchItem>> = p.opt_id("punch")?;
    let list: Vec<&Photo> = visible(v.state, project)
        .into_iter()
        .filter(|x| {
            rfi.is_none_or(|r| x.rfis.contains(&r))
                && obs.is_none_or(|o| x.observations.contains(&o))
                && punch.is_none_or(|u| x.punch_items.contains(&u))
        })
        .filter(|x| album.as_ref().is_none_or(|a| x.album.eq_ignore_ascii_case(a)))
        .filter(|x| {
            q.as_ref().is_none_or(|q| {
                x.caption.to_lowercase().contains(q.as_str())
                    || x.album.to_lowercase().contains(q.as_str())
                    || x.location.to_lowercase().contains(q.as_str())
            })
        })
        .collect();
    json(&list)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Photo> = p.id("id")?;
    let ph = in_project(&v.state.photos, project, id, "photo")?;
    if ph.deleted.is_some() {
        return Err(EngineError::not_found(format!("photo {id} not found in this project")));
    }
    json(ph)
}
