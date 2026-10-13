//! The photos pages: a gallery by album, and a single photo.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::photo::visible;
use buildkiosk_engine::model::photo::Photo;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use buildkiosk_engine::workitem::WorkItem;
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
        div class="page-head" {
            div { h1 { "Photos" } div class="sub" { "Pictures of the work, by album" } }
            @if !shown.is_empty() {
                div class="actions" {
                    a class="btn" href=(format!("{base}/slideshow?album={}", enc(album))) { "Slideshow" }
                    a class="btn" href=(format!("{base}/album.zip?album={}", enc(album))) { (if album.is_empty() { "Download all" } else { "Download album" }) }
                }
            }
        }
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
                        (field("Date taken (empty: the date in the picture, else today)", "taken", false, false, html! { input type="date" id="taken" name="taken"; }))
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
            p class="muted small" { (x.file.name) " · " (human_size(x.file.size)) @if x.width > 0 { " · " (x.width) " × " (x.height) " px" } @if !x.camera.is_empty() { " · " (x.camera) } " · added by " (x.created.by.map(|b| view::person_name(state, b)).unwrap_or_default()) " " (req.when(x.created.at)) " · " a href=(format!("{here}/download")) { "Download original" } }
        }
        @let others: Vec<&Photo> = visible(state, id).into_iter().filter(|o| o.id != pid).take(300).collect();
        @if !others.is_empty() {
            div class="card" {
                form method="get" action=(format!("{base}/compare")) class="filters" {
                    input type="hidden" name="a" value=(pid);
                    label for="cmp_b" { "Compare with" }
                    select id="cmp_b" name="b" { @for o in &others { option value=(o.id) { (o.taken.map(|d| d.to_string()).unwrap_or_default()) " · " (if o.caption.is_empty() { o.file.name.as_str() } else { o.caption.as_str() }) @if !o.location.is_empty() { " · " (o.location) } } } }
                    button type="submit" class="btn small" { "Side by side" }
                }
            }
        }
        @let rfis: Vec<_> = x.rfis.iter().filter_map(|r| state.rfis.get(*r)).filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r)).collect();
        @let obs: Vec<_> = x.observations.iter().filter_map(|o| state.observations.get(*o)).filter(|o| o.project == id && o.deleted.is_none() && o.visible_to(state, req.actor())).collect();
        @let punch: Vec<_> = x.punch_items.iter().filter_map(|o| state.punch_items.get(*o)).filter(|o| o.project == id && o.deleted.is_none() && o.visible_to(state, req.actor())).collect();
        @if !rfis.is_empty() || !obs.is_empty() || !punch.is_empty() || can_edit {
            div class="card" {
                h2 { "Tagged to" }
                ul class="plain" {
                    @for r in &rfis { li { a href=(format!("/projects/{id}/rfis/{}", r.id)) { "RFI #" (r.number) " " (r.subject) } @if can_edit { " " (unlink(req, &here, id, pid, "rfi", r.id.raw())) } } }
                    @for o in &obs { li { a href=(format!("/projects/{id}/observations/{}", o.id)) { "Observation: " (o.title) } @if can_edit { " " (unlink(req, &here, id, pid, "observation", o.id.raw())) } } }
                    @for o in &punch { li { a href=(format!("/projects/{id}/punch-list/{}", o.id)) { "Punch item: " (o.title) } @if can_edit { " " (unlink(req, &here, id, pid, "punch", o.id.raw())) } } }
                }
                @if rfis.is_empty() && obs.is_empty() && punch.is_empty() { p class="muted" { "Not tagged to anything." } }
                @if can_edit {
                    (command_form(req, "photo.link", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        input type="hidden" name="id" value=(pid);
                        div class="form-grid" {
                            (field("Tag it to", "tag_item", true, true, html! { select id="tag_item" name="tag" required {
                                @for r in view::rfis_for(state, req.actor(), id) { option value=(format!("rfi:{}", r.id)) { "RFI #" (r.number) " " (r.subject) } }
                                @for o in state.observations.iter().filter(|o| o.project == id && o.deleted.is_none() && o.visible_to(state, req.actor())) { option value=(format!("observation:{}", o.id)) { "Observation: " (o.title) } }
                                @for o in state.punch_items.iter().filter(|o| o.project == id && o.deleted.is_none() && o.visible_to(state, req.actor())) { option value=(format!("punch:{}", o.id)) { "Punch item: " (o.title) } }
                            } }))
                        }
                        div class="form-actions" { button type="submit" class="btn small" { "Tag" } }
                    }))
                }
            }
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

/// Minimal percent-encoding for an album name in a link.
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

fn unlink(req: &Req<'_>, next: &str, project: Id<Project>, photo: Id<Photo>, kind: &str, item: u64) -> Markup {
    button_form(
        req,
        "photo.link",
        next,
        &[
            ("project", project.to_string()),
            ("id", photo.to_string()),
            ("kind", kind.to_string()),
            ("item", item.to_string()),
            ("remove", "true".to_string()),
        ],
        "Remove tag",
        "small",
    )
}

/// The photos tagged to an item, as a card with thumbnails, for the item's own page.
pub fn tagged_card(req: &Req<'_>, project: Id<Project>, kind: &str, item: u64) -> Markup {
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), project, Tool::Photos);
    if lvl < Level::Read {
        return html! {};
    }
    let shots: Vec<&Photo> = visible(state, project)
        .into_iter()
        .filter(|x| match kind {
            "rfi" => x.rfis.iter().any(|r| r.raw() == item),
            "observation" => x.observations.iter().any(|r| r.raw() == item),
            _ => x.punch_items.iter().any(|r| r.raw() == item),
        })
        .collect();
    let base = format!("/projects/{project}/photos");
    html! {
        @if !shots.is_empty() {
            div class="card" {
                h2 { "Photos" }
                div class="gallery" { @for x in &shots {
                    a class="shot" href=(format!("{base}/{}", x.id)) {
                        img src=(format!("{base}/{}/thumb", x.id)) alt=(if x.caption.is_empty() { x.file.name.as_str() } else { x.caption.as_str() }) loading="lazy" width="220" height="165";
                        span class="cap" { (if x.caption.is_empty() { x.file.name.as_str() } else { x.caption.as_str() }) }
                    }
                } }
            }
        }
    }
}

/// One picture at a time, large, with previous and next links; "Play" moves on by itself every few seconds
/// (the page asks the browser to reload, so no script is involved).
pub fn slideshow(req: &Req<'_>, id: Id<Project>, album: &str, at: usize, play: bool) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Photos) < Level::Read {
        return not_found(req);
    }
    let mut shots: Vec<&Photo> = visible(state, id).into_iter().filter(|x| album.is_empty() || x.album.eq_ignore_ascii_case(album)).collect();
    shots.sort_by_key(|x| (x.taken, x.id));
    let base = format!("/projects/{id}/photos");
    let Some(n) = Some(shots.len()).filter(|n| *n > 0) else {
        return layout::app(
            req,
            "Slideshow",
            Nav::Photos(id),
            &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Photos", Some(base)), ("Slideshow", None)],
            html! {
                div class="card" { (empty(tool_icon(Tool::Photos), "No photos", "There is nothing to show.")) }
            },
        );
    };
    let at = at % n;
    let Some(x) = shots.get(at).copied() else { return not_found(req) };
    let link = |i: usize, play: bool| format!("{base}/slideshow?album={}&i={}{}", enc(album), i % n, if play { "&play=1" } else { "" });
    let (prev, next) = ((at + n - 1) % n, (at + 1) % n);
    let head = html! { @if play { meta http-equiv="refresh" content=(format!("5;url={}", link(next, true))); } };
    let title = if x.caption.is_empty() { x.file.name.as_str() } else { x.caption.as_str() };
    let body = html! {
        div class="page-head" {
            div { h1 { (if album.is_empty() { "All photos" } else { album }) } div class="sub" { "Picture " (at + 1) " of " (n) } }
            div class="actions" {
                a class="btn" href=(link(prev, play)) { "← Previous" }
                @if play { a class="btn" href=(link(at, false)) { "Pause" } } @else { a class="btn" href=(link(at, true)) { "Play" } }
                a class="btn" href=(link(next, play)) { "Next →" }
                a class="btn" href=(if album.is_empty() { base.clone() } else { format!("{base}?album={}", enc(album)) }) { "Close" }
            }
        }
        div class="card" {
            a href=(format!("{base}/{}", x.id)) { img class="photo-large" src=(format!("{base}/{}/image", x.id)) alt=(title) width="900"; }
            p { strong { (title) } }
            p class="muted small" { (x.taken.map(|d| d.pretty()).unwrap_or_default()) @if !x.location.is_empty() { " · " (x.location) } @if !x.album.is_empty() { " · " (x.album) } }
        }
    };
    layout::app_with_head(
        req,
        "Slideshow",
        Nav::Photos(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Photos", Some(base)), ("Slideshow", None)],
        body,
        head,
    )
}

/// Two pictures side by side with their days and places: the same spot before and after, or two views of one problem.
pub fn compare(req: &Req<'_>, id: Id<Project>, a: Id<Photo>, b: Id<Photo>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Photos) < Level::Read {
        return not_found(req);
    }
    let live = |x: Id<Photo>| state.photos.get(x).filter(|x| x.project == id && x.deleted.is_none());
    let (Some(pa), Some(pb)) = (live(a), live(b)) else { return not_found(req) };
    let base = format!("/projects/{id}/photos");
    let title = |x: &Photo| if x.caption.is_empty() { x.file.name.clone() } else { x.caption.clone() };
    let gap = match (pa.taken, pb.taken) {
        (Some(x), Some(y)) if x != y => Some(x.days_until(y).abs()),
        _ => None,
    };
    let body = html! {
        div class="page-head" {
            div { h1 { "Compare photos" } @if let Some(g) = gap { div class="sub" { (g) " day" @if g != 1 { "s" } " apart" } } }
            div class="actions" { a class="btn" href=(format!("{base}/compare?a={b}&b={a}")) { "Swap sides" } a class="btn" href=(base.clone()) { "Back to photos" } }
        }
        div class="compare" {
            @for x in [pa, pb] {
                div class="card" {
                    a href=(format!("{base}/{}", x.id)) { img class="photo-large" src=(format!("{base}/{}/image", x.id)) alt=(title(x)) width="600"; }
                    p { strong { (title(x)) } }
                    p class="muted small" { (x.taken.map(|d| d.pretty()).unwrap_or_default()) @if !x.location.is_empty() { " · " (x.location) } @if !x.album.is_empty() { " · " (x.album) } }
                }
            }
        }
    };
    layout::app(
        req,
        "Compare photos",
        Nav::Photos(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Photos", Some(base)), ("Compare", None)],
        body,
    )
}
