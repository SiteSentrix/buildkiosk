//! The photos pages: a gallery by album, and a single photo.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::photo::visible;
use buildkiosk_engine::model::photo::Photo;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{human_size, not_found, project_for};

pub fn gallery(req: &Req<'_>, id: Id<Project>, album: &str, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Photos);
    if lvl < Level::Read {
        return not_found(req);
    }
    let all = visible(state, id);
    let mut albums: Vec<&str> = all.iter().map(|x| x.album.as_str()).filter(|a| !a.is_empty()).collect();
    albums.sort_by_key(|a| a.to_lowercase());
    albums.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    let needle = q.trim().to_lowercase();
    let shown: Vec<&Photo> = all
        .iter()
        .copied()
        .filter(|x| album.is_empty() || x.album.eq_ignore_ascii_case(album))
        .filter(|x| {
            needle.is_empty()
                || x.caption.to_lowercase().contains(&needle)
                || x.location.to_lowercase().contains(&needle)
                || x.album.to_lowercase().contains(&needle)
        })
        .collect();
    let base = format!("/projects/{id}/photos");
    let body = html! {
        div class="page-head" { div { h1 { "Photos" } div class="sub" { "Pictures of the work, by album" } } }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search caption, album or place" aria-label="Search photos";
            select name="album" aria-label="Album" {
                option value="" { "All albums" }
                @for a in &albums { option value=(a) selected[a.eq_ignore_ascii_case(album)] { (a) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Photos), "No photos", if lvl >= Level::Standard { "Upload pictures below." } else { "Nothing has been added yet." })) }
        } @else {
            div class="gallery" {
                @for x in &shown {
                    a class="shot" href=(format!("{base}/{}", x.id)) {
                        img src=(format!("{base}/{}/thumb", x.id)) alt=(if x.caption.is_empty() { x.file.name.as_str() } else { x.caption.as_str() }) loading="lazy" width="220" height="165";
                        span class="cap" { (if x.caption.is_empty() { x.file.name.as_str() } else { x.caption.as_str() }) }
                        span class="muted small" { (x.taken.map(|d| d.to_string()).unwrap_or_default()) @if !x.album.is_empty() { " · " (x.album) } }
                    }
                }
            }
        }
        @if lvl >= Level::Standard {
            div class="card" {
                h2 { "Upload photos" }
                form method="post" action=(format!("{base}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    div class="form-grid" {
                        (field("Pictures (JPEG, PNG, GIF or WebP; choose several)", "file", true, true, html! { input type="file" id="file" name="file" multiple required accept="image/jpeg,image/png,image/gif,image/webp"; }))
                        (field("Album", "album", false, false, html! { input type="text" id="album" name="album" list="albums" value=(album); datalist id="albums" { @for a in &albums { option value=(a) {} } } }))
                        (field("Where on site", "location", false, false, html! { input type="text" id="location" name="location"; }))
                        (field("Date taken", "taken", false, false, html! { input type="date" id="taken" name="taken" value=(req.today().to_string()); }))
                        (field("Caption (used for every picture)", "caption", false, false, html! { input type="text" id="caption" name="caption"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Upload" } }
                }
                p class="muted small" { "Up to 25 MB in one upload." }
            }
        }
    };
    layout::app(req, "Photos", Nav::Photos(id), &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Photos", None)], body)
}

pub fn detail(req: &Req<'_>, id: Id<Project>, pid: Id<Photo>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Photos);
    let Some(x) = state.photos.get(pid).filter(|x| x.project == id && x.deleted.is_none()).filter(|_| lvl >= Level::Read) else {
        return not_found(req);
    };
    let base = format!("/projects/{id}/photos");
    let here = format!("{base}/{pid}");
    let mine = x.created.by.is_some() && x.created.by == req.actor().person();
    let can_edit = req.allowed("photo.update", Some(id)) && (lvl == Level::Admin || mine);
    let title = if x.caption.is_empty() { x.file.name.clone() } else { x.caption.clone() };
    let body = html! {
        div class="page-head" { div { h1 { (title) } div class="sub" { @if !x.album.is_empty() { (x.album) " · " } (x.taken.map(|d| d.pretty()).unwrap_or_default()) @if !x.location.is_empty() { " · " (x.location) } } } }
        div class="card" {
            img class="photo-large" src=(format!("{here}/image")) alt=(title) width="900";
            p class="muted small" { (x.file.name) " · " (human_size(x.file.size)) " · added by " (x.created.by.map(|b| view::person_name(state, b)).unwrap_or_default()) " " (req.when(x.created.at)) " · " a href=(format!("{here}/download")) { "Download original" } }
        }
        @if can_edit {
            div class="card" {
                h2 { "Details" }
                (command_form(req, "photo.update", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(pid);
                    div class="form-grid" {
                        (field("Caption", "caption", false, true, html! { input type="text" id="caption" name="caption" value=(x.caption); }))
                        (field("Album", "album", false, false, html! { input type="text" id="album" name="album" value=(x.album); }))
                        (field("Where on site", "location", false, false, html! { input type="text" id="location" name="location" value=(x.location); }))
                        (field("Date taken", "taken", false, false, html! { input type="date" id="taken" name="taken" value=(x.taken.map(|d| d.to_string()).unwrap_or_default()); }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                }))
                (button_form(req, "photo.delete", &base, &[("project", id.to_string()), ("id", pid.to_string())], "Delete photo", "small danger"))
            }
        }
    };
    layout::app(
        req,
        &title,
        Nav::Photos(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Photos", Some(base)), (&title, None)],
        body,
    )
}
