//! Documents: folders and the files in them, each file with a version history.
//!
//! The server stores an uploaded file in the blob store first; the commands here record it by its
//! SHA-256, name, size and type. A private folder hides itself and everything under it.

use buildkiosk_core::{Id, text};
use buildkiosk_model::common::FileRef;
use buildkiosk_model::directory::Person;
use buildkiosk_model::document::{DocVersion, Document, Folder};
use buildkiosk_model::project::Project;
use buildkiosk_model::{Level, Tool};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::in_project;
use crate::view::{self, json};
use crate::{Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::mutate(
        "folder.create",
        "Create folder",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; name: text; parent?: folder id; private?: bool (admins only); access?: [person ids] who may open a private folder",
        folder_create,
    ),
    CommandSpec::mutate(
        "folder.update",
        "Edit folder",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; id: folder id; name?: text; parent?: folder id to move it under (empty moves it to the top level); private?: bool (admins only); access?: [person ids] (admins only)",
        folder_update,
    ),
    CommandSpec::mutate(
        "folder.delete",
        "Delete folder",
        Tool::Documents,
        Access::Project(Level::Admin),
        "project: project id; id: folder id (it must be empty)",
        folder_delete,
    ),
    CommandSpec::query(
        "folder.list",
        "List folders",
        Tool::Documents,
        Access::Project(Level::Read),
        "project: project id (the folders the caller may open)",
        folder_list,
    ),
    CommandSpec::mutate(
        "document.add",
        "Add document",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; folder?: folder id; title?: text (default the file name); description?: text; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        add,
    ),
    CommandSpec::mutate(
        "document.add_version",
        "Upload new version",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; id: document id; note?: text; sha256: 64 hex digits; name: file name; size: bytes; mime?: media type",
        add_version,
    ),
    CommandSpec::mutate(
        "document.update",
        "Edit document",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; id: document id; title?: text; description?: text; folder?: folder id (empty moves it to the top level)",
        update,
    ),
    CommandSpec::mutate(
        "document.delete",
        "Delete document",
        Tool::Documents,
        Access::Project(Level::Standard),
        "project: project id; id: document id (its uploader or an admin)",
        delete,
    ),
    CommandSpec::mutate(
        "document.restore",
        "Restore document",
        Tool::Documents,
        Access::Project(Level::Admin),
        "project: project id; id: document id",
        restore,
    ),
    CommandSpec::query(
        "document.list",
        "List documents",
        Tool::Documents,
        Access::Project(Level::Read),
        "project: project id; folder?: folder id (omit for every folder); q?: text; deleted?: bool (admins)",
        list,
    ),
    CommandSpec::query(
        "document.get",
        "Get document",
        Tool::Documents,
        Access::Project(Level::Read),
        "project: project id; id: document id (with every version)",
        get,
    ),
];

const MAX_FOLDERS: usize = 5_000;
const MAX_VERSIONS: usize = 200;
const MAX_ACCESS: usize = 200;

fn live_folder(ctx: &impl Read, project: Id<Project>, id: Id<Folder>) -> Result<&Folder> {
    let f = in_project(&ctx.state().folders, project, id, "folder")?;
    if f.deleted.is_some() || !view::can_see_folder(ctx.state(), ctx.actor(), f) {
        return Err(EngineError::not_found(format!("folder {id} not found in this project")));
    }
    Ok(f)
}

fn is_admin(ctx: &impl Read, project: Id<Project>) -> bool {
    ctx.level(project, Tool::Documents) == Level::Admin
}

fn depth(ctx: &Ctx<'_>, folder: &Folder) -> usize {
    let mut n = 1;
    let mut at = folder.parent;
    while let Some(id) = at {
        n += 1;
        if n > view::MAX_FOLDER_DEPTH {
            break;
        }
        at = ctx.state.folders.get(id).and_then(|f| f.parent);
    }
    n
}

/// How many levels of live folders sit below `id` (0 for none).
fn subtree_height(ctx: &Ctx<'_>, id: Id<Folder>, seen: usize) -> usize {
    if seen > view::MAX_FOLDER_DEPTH {
        return 0;
    }
    ctx.state
        .folders
        .iter()
        .filter(|c| c.deleted.is_none() && c.parent == Some(id))
        .map(|c| 1 + subtree_height(ctx, c.id, seen + 1))
        .max()
        .unwrap_or(0)
}

fn name_taken(ctx: &Ctx<'_>, project: Id<Project>, parent: Option<Id<Folder>>, name: &str, except: Option<Id<Folder>>) -> bool {
    ctx.state
        .folders
        .iter()
        .any(|f| f.project == project && f.deleted.is_none() && f.parent == parent && Some(f.id) != except && f.name.eq_ignore_ascii_case(name))
}

fn people(ctx: &Ctx<'_>, p: &Params<'_>, key: &str) -> Result<Vec<Id<Person>>> {
    let mut ids = p.ids::<Person>(key)?;
    ids.sort();
    ids.dedup();
    if ids.len() > MAX_ACCESS {
        return Err(p.bad(key, format!("lists more than {MAX_ACCESS} people")));
    }
    for id in &ids {
        ctx.check_person(*id)?;
    }
    Ok(ids)
}

fn folder_create(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let name = p.req_line("name", 100)?;
    let parent: Option<Id<Folder>> = p.opt_id("parent")?;
    if let Some(parent) = parent {
        let f = live_folder(ctx, project, parent)?;
        if depth(ctx, f) >= view::MAX_FOLDER_DEPTH {
            return Err(EngineError::conflict(format!("folders nest at most {} deep", view::MAX_FOLDER_DEPTH)));
        }
    }
    if name_taken(ctx, project, parent, &name, None) {
        return Err(EngineError::conflict(format!("a folder named {name} is already there")));
    }
    if ctx.state.folders.iter().filter(|f| f.project == project).count() >= MAX_FOLDERS {
        return Err(EngineError::conflict(format!("a project holds at most {MAX_FOLDERS} folders")));
    }
    let private = p.flag_or("private", false)?;
    let admin = is_admin(ctx, project);
    if (private || p.present("access")) && !admin {
        return Err(EngineError::forbidden("only Documents admins can make a folder private"));
    }
    let access = if p.present("access") { people(ctx, p, "access")? } else { Vec::new() };
    let id = ctx.state.folders.alloc();
    let created = ctx.stamp();
    ctx.state.folders.put(Folder { id, project, parent, name, private, access, created, deleted: None });
    Ok(json!({ "id": id }))
}

fn folder_update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Folder> = p.id("id")?;
    let mut f = live_folder(ctx, project, id)?.clone();
    if p.present("name") {
        let name = p.req_line("name", 100)?;
        if name_taken(ctx, project, f.parent, &name, Some(id)) {
            return Err(EngineError::conflict(format!("a folder named {name} is already there")));
        }
        f.name = name;
    }
    if p.present("parent") {
        let new_parent = p.opt_id::<Folder>("parent")?;
        if new_parent != f.parent {
            if let Some(np) = new_parent {
                live_folder(ctx, project, np)?;
                // Not into itself or anything below it.
                let mut at = Some(np);
                let mut hops = 0;
                while let Some(a) = at {
                    if a == id {
                        return Err(p.bad("parent", "a folder can't be moved into itself or into a folder inside it"));
                    }
                    hops += 1;
                    if hops > view::MAX_FOLDER_DEPTH + 1 {
                        break;
                    }
                    at = ctx.state.folders.get(a).and_then(|x| x.parent);
                }
            }
            let new_depth = new_parent.and_then(|np| ctx.state.folders.get(np)).map(|np| depth(ctx, np)).unwrap_or(0);
            if new_depth + 1 + subtree_height(ctx, id, 0) > view::MAX_FOLDER_DEPTH {
                return Err(p.bad("parent", format!("folders can be at most {} levels deep", view::MAX_FOLDER_DEPTH)));
            }
            if name_taken(ctx, project, new_parent, &f.name, Some(id)) {
                return Err(EngineError::conflict(format!("a folder named {} is already there", f.name)));
            }
            f.parent = new_parent;
        }
    }
    if p.present("private") || p.present("access") {
        if !is_admin(ctx, project) {
            return Err(EngineError::forbidden("only Documents admins can change who may open a folder"));
        }
        if p.present("private") {
            f.private = p.flag_or("private", false)?;
        }
        if p.present("access") {
            f.access = people(ctx, p, "access")?;
        }
    }
    ctx.state.folders.put(f);
    Ok(json!({ "id": id }))
}

fn folder_delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Folder> = p.id("id")?;
    live_folder(ctx, project, id)?;
    let busy = ctx.state.folders.iter().any(|f| f.deleted.is_none() && f.parent == Some(id))
        || ctx.state.documents.iter().any(|d| d.deleted.is_none() && d.folder == Some(id));
    if busy {
        return Err(EngineError::conflict("the folder is not empty; move or delete what is in it first"));
    }
    let stamp = ctx.stamp();
    ctx.state.folders.update(id, |f| f.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

fn folder_list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let mut list: Vec<&Folder> =
        v.state.folders.iter().filter(|f| f.project == project && f.deleted.is_none() && view::can_see_folder(v.state, v.actor, f)).collect();
    list.sort_by_key(|f| f.name.to_lowercase());
    json(&list)
}

fn read_file(p: &Params<'_>) -> Result<FileRef> {
    let sha256 = p.req_line("sha256", 64)?.to_ascii_lowercase();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(p.bad("sha256", "must be 64 hex digits"));
    }
    let name = p.req_line("name", 200)?;
    let size = p.int("size", 0..=2_000_000_000)?.ok_or_else(|| p.bad("size", "is required"))?;
    let mime = p.line("mime", 100)?.filter(|m| !m.is_empty()).unwrap_or_else(|| "application/octet-stream".into());
    Ok(FileRef { sha256, name, size: u64::try_from(size).unwrap_or(0), mime })
}

fn load(ctx: &Ctx<'_>, p: &Params<'_>) -> Result<Document> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Document> = p.id("id")?;
    let d = in_project(&ctx.state.documents, project, id, "document")?;
    if d.deleted.is_some() || !view::can_see_document(ctx.state, ctx.actor, d) {
        return Err(EngineError::not_found(format!("document {id} not found in this project")));
    }
    Ok(d.clone())
}

fn add(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let folder: Option<Id<Folder>> = p.opt_id("folder")?;
    if let Some(f) = folder {
        live_folder(ctx, project, f)?;
    }
    let file = read_file(p)?;
    let title = p.line("title", text::SHORT)?.filter(|t| !t.is_empty()).unwrap_or_else(|| file.name.clone());
    let description = p.text_or_empty("description", text::LONG)?;
    let id = ctx.state.documents.alloc();
    let stamp = ctx.stamp();
    let version = DocVersion { number: 1, file, by: ctx.person(), at: ctx.at, note: String::new() };
    ctx.state.documents.put(Document {
        id,
        project,
        folder,
        title,
        description,
        versions: vec![version],
        created: stamp,
        updated: stamp,
        deleted: None,
    });
    Ok(json!({ "id": id }))
}

fn add_version(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    let file = read_file(p)?;
    if d.current().is_some_and(|c| c.file.sha256 == file.sha256) {
        return Err(EngineError::conflict("that file is already the current version"));
    }
    if d.versions.len() >= MAX_VERSIONS {
        return Err(EngineError::conflict(format!("a document keeps at most {MAX_VERSIONS} versions")));
    }
    let number = d.current().map_or(1, |c| c.number.saturating_add(1));
    let note = p.text_or_empty("note", text::SHORT)?;
    d.versions.push(DocVersion { number, file, by: ctx.person(), at: ctx.at, note });
    d.updated = ctx.stamp();
    let id = d.id;
    ctx.state.documents.put(d);
    Ok(json!({ "id": id, "version": number }))
}

fn can_change(ctx: &Ctx<'_>, d: &Document) -> bool {
    is_admin(ctx, d.project) || (ctx.person().is_some() && d.created.by == ctx.person())
}

fn update(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let mut d = load(ctx, p)?;
    if !can_change(ctx, &d) {
        return Err(EngineError::forbidden("only the person who added a document or a Documents admin can edit it"));
    }
    if p.present("title") {
        d.title = p.req_line("title", text::SHORT)?;
    }
    if p.present("description") {
        d.description = p.text_or_empty("description", text::LONG)?;
    }
    if p.present("folder") {
        let folder: Option<Id<Folder>> = p.opt_id("folder")?;
        if let Some(f) = folder {
            live_folder(ctx, d.project, f)?;
        }
        d.folder = folder;
    }
    d.updated = ctx.stamp();
    let id = d.id;
    ctx.state.documents.put(d);
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let d = load(ctx, p)?;
    if !can_change(ctx, &d) {
        return Err(EngineError::forbidden("only the person who added a document or a Documents admin can delete it"));
    }
    let stamp = ctx.stamp();
    ctx.state.documents.update(d.id, |d| d.deleted = Some(stamp));
    Ok(json!({ "id": d.id }))
}

fn restore(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Document> = p.id("id")?;
    let d = in_project(&ctx.state.documents, project, id, "document")?;
    if d.deleted.is_none() {
        return Err(EngineError::conflict("the document is not in the recycle bin"));
    }
    ctx.state.documents.update(id, |d| d.deleted = None);
    Ok(json!({ "id": id }))
}

fn list(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let folder: Option<Id<Folder>> = p.opt_id("folder")?;
    let q = p.line("q", 200)?.map(|s| s.to_lowercase()).filter(|s| !s.is_empty());
    let bin = p.flag_or("deleted", false)?;
    if bin && v.level(project, Tool::Documents) < Level::Admin {
        return Err(EngineError::forbidden("only admins can see the recycle bin"));
    }
    let mut out: Vec<&Document> = v
        .state
        .documents
        .iter()
        .filter(|d| d.project == project && d.deleted.is_some() == bin)
        .filter(|d| folder.is_none() || d.folder == folder)
        .filter(|d| view::can_see_document(v.state, v.actor, d))
        .filter(|d| {
            q.as_ref().is_none_or(|q| {
                d.title.to_lowercase().contains(q.as_str())
                    || d.description.to_lowercase().contains(q.as_str())
                    || d.versions.iter().any(|x| x.file.name.to_lowercase().contains(q.as_str()))
            })
        })
        .collect();
    out.sort_by_key(|d| d.title.to_lowercase());
    json(&out)
}

fn get(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let id: Id<Document> = p.id("id")?;
    let d = in_project(&v.state.documents, project, id, "document")?;
    if d.deleted.is_some() || !view::can_see_document(v.state, v.actor, d) {
        return Err(EngineError::not_found(format!("document {id} not found in this project")));
    }
    json(d)
}
