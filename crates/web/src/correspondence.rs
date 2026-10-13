//! Correspondence pages: list, detail with replies, and the new/edit form.

use buildkiosk_core::Id;
use buildkiosk_engine::cmd::correspondence::{can_see, types, visible};
use buildkiosk_engine::model::correspondence::{Correspondence, CorrespondenceStatus};
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Choice, Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

fn status_pill(s: CorrespondenceStatus) -> Markup {
    let class = match s {
        CorrespondenceStatus::Draft => "violet",
        CorrespondenceStatus::Open => "blue",
        CorrespondenceStatus::Closed => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

pub fn list(req: &Req<'_>, id: Id<Project>, status: Option<CorrespondenceStatus>, q: &str) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let lvl = view::level(state, req.actor(), id, Tool::Correspondence);
    if lvl < Level::Read {
        return not_found(req);
    }
    let needle = q.trim().to_lowercase();
    let shown: Vec<&Correspondence> = visible(state, req.actor(), id)
        .into_iter()
        .filter(|c| status.is_none_or(|s| c.status == s))
        .filter(|c| {
            needle.is_empty()
                || c.subject.to_lowercase().contains(&needle)
                || c.kind.to_lowercase().contains(&needle)
                || c.body.to_lowercase().contains(&needle)
        })
        .collect();
    let today = req.today();
    let base = format!("/projects/{id}/correspondence");
    let body = html! {
        div class="page-head" {
            div { h1 { "Correspondence" } div class="sub" { "Letters, notices and emails kept on the project record" } }
            div class="actions" {
                @if lvl == Level::Admin { a class="btn" href=(format!("{base}/types")) { "Types" } }
                @if lvl >= Level::Standard { a class="btn" href=(format!("{base}/incoming")) { "File incoming" } a class="btn primary" href=(format!("{base}/new")) { "New correspondence" } }
            }
        }
        form method="get" action=(base) class="filters" {
            input type="search" name="q" value=(q) placeholder="Search subject, type or text" aria-label="Search correspondence";
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in CorrespondenceStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Correspondence), "No correspondence", "Write a letter or notice to keep it on the record.")) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "#" } th { "Subject" } th { "Type" } th { "Date" } th { "To" } th { "Reply by" } th { "Replies" } th { "Status" } } }
                tbody { @for c in &shown { tr {
                    td class="nowrap" { (if c.reference.is_empty() { c.number.to_string() } else { c.reference.clone() }) }
                    td { a href=(format!("{base}/{}", c.id)) { (c.subject) } @if c.incoming.is_some() { " " span class="pill violet" { "Incoming" } } @if c.private { " " span class="pill blue" { "Private" } } }
                    td { (dash(&c.kind)) }
                    td class="nowrap" { (c.date.to_string()) }
                    td { (view::people_names(state, &c.to)) }
                    td class="nowrap" { @if let Some(d) = c.reply_by { @if d < today && c.status == CorrespondenceStatus::Open && c.replies.is_empty() { span class="overdue" { (d.to_string()) } } @else { (d.to_string()) } } }
                    td { (c.replies.len()) }
                    td { (status_pill(c.status)) }
                } } }
            } }
        }
    };
    layout::app(
        req,
        "Correspondence",
        Nav::Correspondence(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Correspondence", None)],
        body,
    )
}

pub fn detail(req: &Req<'_>, id: Id<Project>, cid: Id<Correspondence>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(c) = state.correspondence.get(cid).filter(|c| c.project == id && c.deleted.is_none() && can_see(state, req.actor(), c)) else {
        return not_found(req);
    };
    let lvl = view::level(state, req.actor(), id, Tool::Correspondence);
    let admin = lvl == Level::Admin;
    let me = req.actor().person();
    let author = me.is_some() && c.created.by == me;
    let draft = c.status == CorrespondenceStatus::Draft;
    let can_edit = req.allowed("correspondence.update", Some(id)) && (admin || (draft && author));
    let can_reply = c.status == CorrespondenceStatus::Open && (admin || author || me.is_some_and(|m| c.to.contains(&m) || c.cc.contains(&m)));
    let base = format!("/projects/{id}/correspondence");
    let here = format!("{base}/{cid}");
    let ids = [("project", id.to_string()), ("id", cid.to_string())];
    let body = html! {
        div class="page-head" {
            div { h1 { (c.label()) ": " (c.subject) " " (status_pill(c.status)) } div class="sub" { @if !c.kind.is_empty() { (c.kind) " · " } (c.date.pretty()) @if let Some(s) = c.sent { " · sent by " (s.by.map(|b| view::person_name(state, b)).unwrap_or_default()) } } }
            div class="actions" {
                a class="btn" href=(format!("{here}/letter.pdf")) { "Print as letter" }
                a class="btn" href=(format!("{here}/pdf")) { "PDF" }
                @if can_edit { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if draft && can_edit { (button_form(req, "correspondence.send", &here, &ids, "Send", "primary")) }
                @if c.status == CorrespondenceStatus::Open && (admin || author) { (button_form(req, "correspondence.close", &here, &ids, "Close", "")) }
                @if c.status == CorrespondenceStatus::Closed && (admin || author) { (button_form(req, "correspondence.reopen", &here, &ids, "Reopen", "")) }
                @if can_edit { (button_form(req, "correspondence.delete", &base, &ids, "Delete", "danger")) }
            }
        }
        div class="card" {
            dl class="facts" {
                @if let Some(sn) = &c.incoming { dt { "From" } dd { (sn.name) @if !sn.email.is_empty() { " <" (sn.email) ">" } " (outside the company)" } }
                @let rfis: Vec<_> = c.rfis.iter().filter_map(|r| state.rfis.get(*r)).filter(|r| r.project == id && view::can_see_rfi(state, req.actor(), r)).collect();
                @let docs: Vec<_> = c.documents.iter().filter_map(|d| state.documents.get(*d)).filter(|d| d.project == id && view::can_see_document(state, req.actor(), d)).collect();
                @if !rfis.is_empty() { dt { "RFIs" } dd { @for (i, r) in rfis.iter().enumerate() { @if i > 0 { ", " } a href=(format!("/projects/{id}/rfis/{}", r.id)) { "#" (r.number) " " (r.subject) } } } }
                @if !docs.is_empty() { dt { "Documents" } dd { @for (i, d) in docs.iter().enumerate() { @if i > 0 { ", " } a href=(format!("/projects/{id}/documents/{}", d.id)) { (d.title) } } } }
                dt { "To" } dd { (dash(&view::people_names(state, &c.to))) }
                dt { "Copied" } dd { (dash(&view::people_names(state, &c.cc))) }
                dt { "Reply wanted by" } dd { (c.reply_by.map(|d| d.pretty()).unwrap_or_else(|| "—".into())) }
            }
            @if !c.body.is_empty() { p class="prose" { (c.body) } }
        }
        div class="card" {
            h2 { "Replies" }
            @if c.replies.is_empty() { p class="muted" { "No replies yet." } } @else {
                ul class="timeline" { @for r in &c.replies { li {
                    div { strong { (r.by.map(|b| view::person_name(state, b)).unwrap_or_default()) } }
                    div class="when" { (req.when(r.at)) }
                    p class="prose" { (r.body) }
                } } }
            }
            @if can_reply {
                (command_form(req, "correspondence.reply", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    input type="hidden" name="id" value=(cid);
                    (field("Your reply", "body", true, true, html! { textarea id="body" name="body" required {} }))
                    div class="form-actions" { button type="submit" class="btn primary" { "Reply" } }
                }))
            }
        }
        div class="card" {
            h2 { "Files" }
            @if c.attachments.is_empty() { p class="muted" { "No files attached." } } @else {
                ul class="plain" { @for f in &c.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if can_edit { " " (button_form(req, "correspondence.detach", &here, &[("project", id.to_string()), ("id", cid.to_string()), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
                } } }
            }
            @if can_edit {
                form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                    input type="hidden" name="_csrf" value=(req.csrf);
                    input type="file" name="file" required aria-label="File to attach";
                    button type="submit" class="btn small" { "Attach file" }
                }
            }
        }
    };
    layout::app(
        req,
        &c.label(),
        Nav::Correspondence(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Correspondence", Some(base)), (&c.label(), None)],
        body,
    )
}

pub fn form(req: &Req<'_>, id: Id<Project>, current: Option<Id<Correspondence>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let cur = match current {
        Some(c) => match state.correspondence.get(c).filter(|x| x.project == id && x.deleted.is_none() && can_see(state, req.actor(), x)) {
            Some(x) => Some(x),
            None => return not_found(req),
        },
        None => None,
    };
    let command = if cur.is_some() { "correspondence.update" } else { "correspondence.create" };
    if !req.allowed(command, Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/correspondence");
    let back = cur.map(|c| format!("{base}/{}/edit", c.id)).unwrap_or_else(|| format!("{base}/new"));
    let next = cur.map(|c| format!("{base}/{}", c.id)).unwrap_or_else(|| format!("{base}/{{id}}"));
    let people = user_options(req, id, true);
    let kinds = types(state, id);
    let rfis: Vec<_> = view::rfis_for(state, req.actor(), id);
    let docs: Vec<_> =
        state.documents.iter().filter(|d| d.project == id && d.deleted.is_none() && view::can_see_document(state, req.actor(), d)).collect();
    let picker = |name: &str, label: &str, chosen: &[Id<buildkiosk_engine::model::directory::Person>]| {
        html! {
            div class="field full" {
                div class="label" { (label) }
                input type="hidden" name=(format!("{name}[]")) value="";
                details class="multi" {
                    summary { "Choose people" }
                    div class="options" { @for x in &people { label { input type="checkbox" name=(format!("{name}[]")) value=(x.id) checked[chosen.contains(&x.id)]; (x.name()) } } }
                }
            }
        }
    };
    let body = html! {
        div class="page-head" { div { h1 { (if cur.is_some() { "Edit correspondence" } else { "New correspondence" }) } } }
        div class="card" {
            (command_form(req, command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(c) = cur { input type="hidden" name="id" value=(c.id); }
                div class="form-grid" {
                    (field("Subject", "subject", true, true, html! { input type="text" id="subject" name="subject" required value=(cur.map(|c| c.subject.as_str()).unwrap_or_default()); }))
                    @if kinds.is_empty() {
                        (field("Type", "kind", false, false, html! { input type="text" id="kind" name="kind" placeholder="Letter, Notice, Email…" value=(cur.map(|c| c.kind.as_str()).unwrap_or_default()); }))
                    } @else {
                        (field("Type", "kind", true, false, html! { select id="kind" name="kind" required { @for t in &kinds { option value=(t.name) selected[cur.is_some_and(|c| c.kind.eq_ignore_ascii_case(&t.name))] { (t.name) " (" (t.prefix) ")" } } } }))
                    }
                    (field("Date", "date", false, false, html! { input type="date" id="date" name="date" value=(cur.map(|c| c.date).unwrap_or_else(|| req.today()).to_string()); }))
                    (field("Reply wanted by", "reply_by", false, false, html! { input type="date" id="reply_by" name="reply_by" value=(cur.and_then(|c| c.reply_by).map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Visible to", "private", false, false, html! { select id="private" name="private" {
                        option value="false" selected[cur.is_none_or(|c| !c.private)] { "Everyone with Correspondence access" }
                        option value="true" selected[cur.is_some_and(|c| c.private)] { "Only me, the recipients and admins" }
                    } }))
                    (picker("to", "To", cur.map(|c| c.to.as_slice()).unwrap_or_default()))
                    (picker("cc", "Copy", cur.map(|c| c.cc.as_slice()).unwrap_or_default()))
                    @if !rfis.is_empty() {
                        div class="field full" { div class="label" { "Linked RFIs" } input type="hidden" name="rfis[]" value="";
                            details class="multi" { summary { "Choose RFIs" } div class="options" { @for r in &rfis { label { input type="checkbox" name="rfis[]" value=(r.id) checked[cur.is_some_and(|c| c.rfis.contains(&r.id))]; "#" (r.number) " " (r.subject) } } } } }
                    }
                    @if !docs.is_empty() {
                        div class="field full" { div class="label" { "Linked documents" } input type="hidden" name="documents[]" value="";
                            details class="multi" { summary { "Choose documents" } div class="options" { @for d in &docs { label { input type="checkbox" name="documents[]" value=(d.id) checked[cur.is_some_and(|c| c.documents.contains(&d.id))]; (d.title) } } } } }
                    }
                    (field("Text", "body", false, true, html! { textarea id="body" name="body" { (cur.map(|c| c.body.as_str()).unwrap_or_default()) } }))
                }
                div class="form-actions" {
                    a class="btn" href=(cur.map(|c| format!("{base}/{}", c.id)).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { "Save draft" }
                }
            }))
        }
    };
    layout::app(
        req,
        "Correspondence",
        Nav::Correspondence(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Correspondence", Some(base)), ("Correspondence", None)],
        body,
    )
}

/// The project's correspondence types: names and the prefixes that start their references.
pub fn types_page(req: &Req<'_>, id: Id<Project>, editing: Option<Id<buildkiosk_engine::model::correspondence::CorrespondenceType>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Correspondence) < Level::Admin {
        return not_found(req);
    }
    let base = format!("/projects/{id}/correspondence");
    let here = format!("{base}/types");
    let list = types(state, id);
    let cur = editing.and_then(|e| list.iter().copied().find(|t| t.id == e));
    let used = |t: &buildkiosk_engine::model::correspondence::CorrespondenceType| {
        state.correspondence.iter().filter(|c| c.type_id == Some(t.id) && c.deleted.is_none()).count()
    };
    let body = html! {
        div class="page-head" { div { h1 { "Correspondence types" } div class="sub" { "Each type is numbered on its own: LTR-001, LTR-002… With types set, every item must be one of them." } } }
        @if list.is_empty() { div class="card" { p class="muted" { "No types yet, so the type of an item is free text and items are numbered together. Add the first type below." } } } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Type" } th { "Prefix" } th class="num" { "Items" } th {} } }
                tbody { @for t in &list { tr {
                    td { (t.name) } td { (t.prefix) } td class="num" { (used(t)) }
                    td class="right" { a class="btn small" href=(format!("{here}/{}", t.id)) { "Edit" } " " (button_form(req, "correspondence_type.delete", &here, &[("project", id.to_string()), ("id", t.id.to_string())], "Delete", "small danger")) }
                } } }
            } }
        }
        div class="card" {
            h2 { (if cur.is_some() { "Edit type" } else { "New type" }) }
            (command_form(req, "correspondence_type.save", &here, &here, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(t) = cur { input type="hidden" name="id" value=(t.id); }
                div class="form-grid" {
                    (field("Name", "ty_name", true, false, html! { input type="text" id="ty_name" name="name" required value=(cur.map(|t| t.name.as_str()).unwrap_or_default()); }))
                    (field("Prefix", "ty_prefix", true, false, html! { input type="text" id="ty_prefix" name="prefix" required maxlength="8" placeholder="LTR" value=(cur.map(|t| t.prefix.as_str()).unwrap_or_default()); }))
                }
                p class="muted small" { "Changing a prefix affects new items only; references already given stay." }
                div class="form-actions" { button type="submit" class="btn primary" { "Save type" } }
            }))
        }
    };
    layout::app(
        req,
        "Correspondence types",
        Nav::Correspondence(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Correspondence", Some(base)), ("Types", None)],
        body,
    )
}

/// Filing something that came from outside: typed in, or an email saved from a mail program pasted in.
pub fn incoming_page(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if !req.allowed("correspondence.file_incoming", Some(id)) {
        return not_found(req);
    }
    let base = format!("/projects/{id}/correspondence");
    let here = format!("{base}/incoming");
    let people = user_options(req, id, true);
    let kinds = types(state, id);
    let kind_field = |suffix: &str, required: bool| -> Markup {
        let fid = format!("kind_{suffix}");
        if kinds.is_empty() {
            field("Type", &fid, false, false, html! { input type="text" id=(fid) name="kind" placeholder="Letter, Notice, Email…"; })
        } else {
            field("Type", &fid, required, false, html! { select id=(fid) name="kind" { @for t in &kinds { option value=(t.name) { (t.name) } } } })
        }
    };
    let who = |name: &str| -> Markup {
        html! {
            div class="field full" {
                div class="label" { "Who should see it" }
                input type="hidden" name=(format!("{name}[]")) value="";
                details class="multi" { summary { "Choose people" } div class="options" { @for x in &people { label { input type="checkbox" name=(format!("{name}[]")) value=(x.id); (x.name()) } } } }
            }
        }
    };
    let body = html! {
        div class="page-head" { div { h1 { "File incoming correspondence" } div class="sub" { "Keep a letter, notice or email that came from outside the company on the project record" } } }
        div class="card" {
            h2 { "Paste an email" }
            p class="muted small" { "Save the email from your mail program as a .eml file, open it in a text editor and paste everything here. The subject, date, sender and text are read from it, and people whose email address is on the To or Cc line are told." }
            (command_form(req, "correspondence.import_email", &format!("{base}/{{id}}"), &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (kind_field("e", false))
                    (field("The email", "raw", true, true, html! { textarea id="raw" name="raw" rows="12" required placeholder="From: …\nTo: …\nSubject: …\n\nText" {} }))
                }
                div class="form-actions" { button type="submit" class="btn primary" { "File email" } }
            }))
        }
        div class="card" {
            h2 { "Type it in" }
            (command_form(req, "correspondence.file_incoming", &format!("{base}/{{id}}"), &here, html! {
                input type="hidden" name="project" value=(id);
                div class="form-grid" {
                    (field("From (name)", "from_name", true, false, html! { input type="text" id="from_name" name="from_name" required; }))
                    (field("From (email)", "from_email", false, false, html! { input type="text" id="from_email" name="from_email"; }))
                    (field("Subject", "in_subject", true, true, html! { input type="text" id="in_subject" name="subject" required; }))
                    (kind_field("t", true))
                    (field("Date on it", "in_date", false, false, html! { input type="date" id="in_date" name="date" value=(req.today().to_string()); }))
                    (who("to"))
                    (field("Text", "in_body", false, true, html! { textarea id="in_body" name="body" {} }))
                }
                div class="form-actions" { button type="submit" class="btn primary" { "File it" } }
            }))
        }
    };
    layout::app(
        req,
        "File incoming",
        Nav::Correspondence(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Correspondence", Some(base)), ("File incoming", None)],
        body,
    )
}
