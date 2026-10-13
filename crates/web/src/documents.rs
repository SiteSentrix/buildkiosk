//! The documents pages: folders, the files in them, and a document's versions.

use buildkiosk_core::Id;
use buildkiosk_engine::model::document::{Document, Folder};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

/// The folders from the top down to `folder`.
fn trail<'a>(state: &'a buildkiosk_engine::model::CompanyState, folder: &'a Folder) -> Vec<&'a Folder> {
    let mut out = vec![folder];
    let mut at = folder.parent;
    while let Some(id) = at {
        let Some(f) = state.folders.get(id) else { break };
        out.push(f);
        if out.len() > view::MAX_FOLDER_DEPTH {
            break;
        }
        at = f.parent;
    }
    out.reverse();
    out
}

/// The folder's full path, "Contracts / Signed".
fn folder_path(state: &buildkiosk_engine::model::CompanyState, f: &Folder) -> String {
    trail(state, f).iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(" / ")
}

/// The folders `folder` could be moved into: any visible folder that isn't it or inside it.
fn move_targets<'a>(
    state: &'a buildkiosk_engine::model::CompanyState,
    actor: buildkiosk_engine::Actor,
    project: Id<Project>,
    folder: &Folder,
) -> Vec<&'a Folder> {
    let mut list: Vec<&Folder> = state
        .folders
        .iter()
        .filter(|f| f.project == project && f.deleted.is_none() && f.id != folder.id && view::can_see_folder(state, actor, f))
        .filter(|f| !trail(state, f).iter().any(|x| x.id == folder.id))
        .collect();
    list.sort_by_key(|f| folder_path(state, f).to_lowercase());
    list
}

/// Deleted documents, for admins to restore.
pub fn trash(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Documents) != Level::Admin {
        return not_found(req);
    }
    let base = format!("/projects/{id}/documents");
    let here = format!("{base}/trash");
    let mut docs: Vec<&Document> = state.documents.iter().filter(|d| d.project == id && d.deleted.is_some()).collect();
    docs.sort_by_key(|d| std::cmp::Reverse(d.deleted.map(|s| s.at)));
    let body = html! {
        div class="page-head" {
            div { h1 { "Deleted files" } div class="sub" { (p.name) " · only Documents admins see this" } }
            div class="actions" { a class="btn" href=(base.clone()) { "Back to documents" } }
        }
        @if docs.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Documents), "Nothing deleted", "Deleted documents wait here until an admin restores them.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Title" } th { "Was in" } th { "Deleted" } th { "By" } th {} } }
                tbody { @for d in &docs { tr {
                    td { (d.title) }
                    td { @if let Some(f) = d.folder.and_then(|f| state.folders.get(f)) { (folder_path(state, f)) } @else { span class="muted" { "Top level" } } }
                    td class="nowrap" { @if let Some(s) = d.deleted { (req.when(s.at)) } }
                    td { @if let Some(b) = d.deleted.and_then(|s| s.by) { (view::person_name(state, b)) } }
                    td class="right" { (button_form(req, "document.restore", &here, &[("project", id.to_string()), ("id", d.id.to_string())], "Restore", "small")) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Deleted files",
        Nav::Documents(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Documents", Some(base)), ("Deleted", None)],
        body,
    )
}

pub fn page(req: &Req<'_>, id: Id<Project>, folder: Option<Id<Folder>>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let actor = req.actor();
    let lvl = view::level(state, actor, id, Tool::Documents);
    if lvl < Level::Read {
        return not_found(req);
    }
    let current = match folder {
        Some(f) => match state.folders.get(f).filter(|x| x.project == id && x.deleted.is_none() && view::can_see_folder(state, actor, x)) {
            Some(x) => Some(x),
            None => return not_found(req),
        },
        None => None,
    };
    let base = format!("/projects/{id}/documents");
    let here = match current {
        Some(f) => format!("{base}?folder={}", f.id),
        None => base.clone(),
    };
    let q = q.trim();
    let needle = q.to_lowercase();
    let mut subfolders: Vec<&Folder> = state
        .folders
        .iter()
        .filter(|f| f.project == id && f.deleted.is_none() && f.parent == current.map(|c| c.id) && view::can_see_folder(state, actor, f))
        .collect();
    subfolders.sort_by_key(|f| f.name.to_lowercase());
    let mut docs: Vec<&Document> = state
        .documents
        .iter()
        .filter(|d| d.project == id && d.deleted.is_none() && view::can_see_document(state, actor, d))
        .filter(|d| {
            if needle.is_empty() {
                d.folder == current.map(|c| c.id)
            } else {
                d.title.to_lowercase().contains(&needle)
                    || d.description.to_lowercase().contains(&needle)
                    || d.versions.iter().any(|v| v.file.name.to_lowercase().contains(&needle))
            }
        })
        .collect();
    docs.sort_by_key(|d| d.title.to_lowercase());
    let can_add = lvl >= Level::Standard;
    let admin = lvl == Level::Admin;
    let people = if admin { user_options(req, id, true) } else { Vec::new() };
    let body = html! {
        div class="page-head" {
            div { h1 { "Documents" } div class="sub" { "Project files in folders, each with its versions" } }
            div class="actions" {
                a class="btn" href=(match current { Some(c) => format!("{base}/zip?folder={}", c.id), None => format!("{base}/zip") }) { "Download all as zip" }
                @if admin { a class="btn" href=(format!("{base}/trash")) { "Deleted files" } }
            }
        }
        form method="get" action=(base) class="filters" {
            @if let Some(c) = current { input type="hidden" name="folder" value=(c.id); }
            input type="search" name="q" value=(q) placeholder="Search every folder by title or file name" aria-label="Search documents";
            button type="submit" class="btn" { "Search" }
            @if !q.is_empty() { a class="btn" href=(here) { "Clear" } }
        }
        div class="crumbs" {
            a href=(base) { "All documents" }
            @if let Some(c) = current { @for f in trail(state, c) { span { "/" } @if f.id == c.id { span { (f.name) } } @else { a href=(format!("{base}?folder={}", f.id)) { (f.name) } } } }
        }
        @if !q.is_empty() { p class="muted" { "Results for “" (q) "” across the project (" (docs.len()) ")" } }
        @if q.is_empty() && !subfolders.is_empty() {
            div class="card" {
                h2 { "Folders" }
                ul class="plain" {
                    @for f in &subfolders {
                        li {
                            a href=(format!("{base}?folder={}", f.id)) { (tool_icon(Tool::Documents).svg()) " " (f.name) }
                            @if f.private { " " span class="pill blue" { "Private" } }
                        }
                    }
                }
            }
        }
        @if docs.is_empty() && (q.is_empty() && subfolders.is_empty() || !q.is_empty()) {
            div class="card" { (empty(tool_icon(Tool::Documents), "No documents here", if can_add { "Upload a file below or create a folder." } else { "Nothing has been added yet." })) }
        } @else if !docs.is_empty() {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Title" } th { "File" } th { "Version" } th class="right" { "Size" } th { "Updated" } th { "By" } } }
                    tbody {
                        @for d in &docs {
                            @if let Some(cur) = d.current() {
                                tr {
                                    td { a href=(format!("{base}/{}", d.id)) { (d.title) } }
                                    td { a href=(format!("{base}/{}/files/{}", d.id, cur.file.sha256)) { (cur.file.name) } }
                                    td { "v" (cur.number) }
                                    td class="right nowrap" { (human_size(cur.file.size)) }
                                    td class="nowrap" { (req.when(cur.at)) }
                                    td { (cur.by.map(|b| view::person_name(state, b)).unwrap_or_default()) }
                                }
                            }
                        }
                    }
                }
            }
        }
        @if can_add {
            div class="card" {
                h2 { "Upload a file" }
                form method="post" action=(format!("{base}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    @if let Some(c) = current { input type="hidden" name="folder" value=(c.id); }
                    div class="form-grid" {
                        (field("File", "file", true, false, html! { input type="file" id="file" name="file" required; }))
                        (field("Title (default: the file name)", "title", false, false, html! { input type="text" id="title" name="title"; }))
                        (field("Description", "description", false, true, html! { textarea id="description" name="description" {} }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Upload" } }
                }
                p class="muted small" { "Files can be at most 25 MB." }
                details class="disclosure" {
                    summary { "Upload several files at once" }
                    form method="post" action=(format!("{base}/upload-many")) enctype="multipart/form-data" class="stack" {
                        input type="hidden" name="_csrf" value=(req.csrf);
                        @if let Some(c) = current { input type="hidden" name="folder" value=(c.id); }
                        (field("Files", "files", true, false, html! { input type="file" id="files" name="file" multiple required; }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Upload files" } }
                    }
                    p class="muted small" { "Up to 50 files, 25 MB each and 100 MB in all. Each file becomes a document titled with its file name." }
                }
            }
            div class="card" {
                h2 { "New folder" }
                (command_form(req, "folder.create", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    @if let Some(c) = current { input type="hidden" name="parent" value=(c.id); }
                    div class="form-grid" {
                        (field("Name", "folder_name", true, false, html! { input type="text" id="folder_name" name="name" required; }))
                        @if admin {
                            (field("Who can open it", "folder_private", false, false, html! { select id="folder_private" name="private" { option value="false" { "Everyone with Documents access" } option value="true" { "Only the people I choose, and admins" } } }))
                            div class="field full" {
                                div class="label" { "People for a private folder" }
                                input type="hidden" name="access[]" value="";
                                details class="multi" {
                                    summary { "Choose people" }
                                    div class="options" { @for x in &people { label { input type="checkbox" name="access[]" value=(x.id); (x.name()) } } }
                                }
                            }
                        }
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Create folder" } }
                }))
            }
        }
        @if let Some(c) = current {
            @if can_add {
                div class="card" {
                    details class="disclosure" {
                        summary { "Folder settings" }
                        (command_form(req, "folder.update", &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(c.id);
                            div class="form-grid" {
                                (field("Name", "edit_name", true, false, html! { input type="text" id="edit_name" name="name" value=(c.name) required; }))
                                @if admin {
                                    (field("Who can open it", "edit_private", false, false, html! { select id="edit_private" name="private" {
                                        option value="false" selected[!c.private] { "Everyone with Documents access" }
                                        option value="true" selected[c.private] { "Only the people I choose, and admins" }
                                    } }))
                                    div class="field full" {
                                        div class="label" { "People for a private folder" }
                                        input type="hidden" name="access[]" value="";
                                        details class="multi" {
                                            summary { "Choose people" }
                                            div class="options" { @for x in &people { label { input type="checkbox" name="access[]" value=(x.id) checked[c.access.contains(&x.id)]; (x.name()) } } }
                                        }
                                    }
                                }
                            }
                            div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                        }))
                        @let targets = move_targets(state, actor, id, c);
                        @if !targets.is_empty() || c.parent.is_some() {
                            (command_form(req, "folder.update", &here, &here, html! {
                                input type="hidden" name="project" value=(id);
                                input type="hidden" name="id" value=(c.id);
                                div class="form-grid" {
                                    (field("Move this folder into", "move_to", false, false, html! { select id="move_to" name="parent" {
                                        option value="" selected[c.parent.is_none()] { "Top level" }
                                        @for t in &targets { option value=(t.id) selected[c.parent == Some(t.id)] { (folder_path(state, t)) } }
                                    } }))
                                }
                                div class="form-actions" { button type="submit" class="btn" { "Move" } }
                            }))
                        }
                        @if admin && docs.is_empty() && subfolders.is_empty() {
                            @let up = c.parent.map(|p| format!("{base}?folder={p}")).unwrap_or_else(|| base.clone());
                            (button_form(req, "folder.delete", &up, &[("project", id.to_string()), ("id", c.id.to_string())], "Delete this folder", "small danger"))
                        }
                    }
                }
            }
        }
    };
    let title = current.map(|c| c.name.clone()).unwrap_or_else(|| "Documents".into());
    layout::app(
        req,
        &title,
        Nav::Documents(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Documents", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, did: Id<Document>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let actor = req.actor();
    let Some(d) = state.documents.get(did).filter(|d| d.project == id && d.deleted.is_none() && view::can_see_document(state, actor, d)) else {
        return not_found(req);
    };
    let lvl = view::level(state, actor, id, Tool::Documents);
    let base = format!("/projects/{id}/documents");
    let here = format!("{base}/{did}");
    let folder = d.folder.and_then(|f| state.folders.get(f));
    let back = d.folder.map(|f| format!("{base}?folder={f}")).unwrap_or_else(|| base.clone());
    let can_edit = req.allowed("document.update", Some(id)) && (lvl == Level::Admin || (d.created.by.is_some() && d.created.by == actor.person()));
    let mut folders: Vec<&Folder> =
        state.folders.iter().filter(|f| f.project == id && f.deleted.is_none() && view::can_see_folder(state, actor, f)).collect();
    folders.sort_by_key(|f| f.name.to_lowercase());
    let body = html! {
        div class="page-head" {
            div { h1 { (d.title) } div class="sub" { "In " @if let Some(f) = folder { a href=(back) { (f.name) } } @else { a href=(back) { "All documents" } } } }
        }
        div class="card" {
            @if !d.description.is_empty() { p { (d.description) } }
            h2 { "Versions" }
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Version" } th { "File" } th class="right" { "Size" } th { "Uploaded" } th { "By" } th { "Note" } } }
                    tbody {
                        @for v in d.versions.iter().rev() {
                            tr {
                                td { "v" (v.number) @if v.number == d.current().map_or(0, |c| c.number) { " " span class="pill blue" { "Current" } } }
                                td { a href=(format!("{here}/files/{}", v.file.sha256)) { (v.file.name) } }
                                td class="right nowrap" { (human_size(v.file.size)) }
                                td class="nowrap" { (req.when(v.at)) }
                                td { (v.by.map(|b| view::person_name(state, b)).unwrap_or_default()) }
                                td { (dash(&v.note)) }
                            }
                        }
                    }
                }
            }
        }
        @if req.allowed("document.add_version", Some(id)) {
            div class="card" {
                h2 { "Upload a new version" }
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-grid" {
                        (field("File", "file", true, false, html! { input type="file" id="file" name="file" required; }))
                        (field("What changed", "note", false, false, html! { input type="text" id="note" name="note"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Upload version" } }
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Details" }
                (command_form(req, "document.update", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(did);
                    div class="form-grid" {
                        (field("Title", "title", true, false, html! { input type="text" id="title" name="title" value=(d.title) required; }))
                        (field("Folder", "folder", false, false, html! { select id="folder" name="folder" {
                            option value="" selected[d.folder.is_none()] { "Top level" }
                            @for f in &folders { option value=(f.id) selected[Some(f.id) == d.folder] { (f.name) } }
                        } }))
                        (field("Description", "description", false, true, html! { textarea id="description" name="description" { (d.description) } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                }))
                (button_form(req, "document.delete", &back, &[("project", id.to_string()), ("id", did.to_string())], "Delete document", "small danger"))
            }
        }
    };
    layout::app(
        req,
        &d.title,
        Nav::Documents(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Documents", Some(base)), (&d.title, None)],
        body,
    )
}
