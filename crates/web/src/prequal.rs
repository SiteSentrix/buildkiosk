//! The prequalification page: every company's record, who needs attention, and review.

use buildkiosk_core::{Date, Id};
use buildkiosk_engine::cmd::prequal::{forms_list, visible};
use buildkiosk_engine::model::directory::Vendor;
use buildkiosk_engine::model::prequal::{PrequalForm, PrequalStatus, Prequalification, Standing};
use buildkiosk_engine::model::{Choice, Level, Tool, perm};
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{human_size, not_found};

fn status_pill(s: PrequalStatus) -> Markup {
    let class = match s {
        PrequalStatus::Pending => "violet",
        PrequalStatus::Submitted => "amber",
        PrequalStatus::Approved => "green",
        PrequalStatus::Rejected => "red",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn expiry(d: Option<Date>, today: Date) -> Markup {
    html! {
        @if let Some(d) = d {
            @if d < today { span class="overdue" { (d.to_string()) " (lapsed)" } }
            @else if today.days_until(d) <= Prequalification::WARN_DAYS { span class="pill amber" { (d.to_string()) } }
            @else { (d.to_string()) }
        } @else { span class="muted" { "—" } }
    }
}

pub fn page(
    req: &Req<'_>,
    status: Option<PrequalStatus>,
    attention: bool,
    edit: Option<buildkiosk_core::Id<buildkiosk_engine::model::directory::Vendor>>,
) -> Markup {
    let state = req.engine.state();
    let lvl = perm::company_level(state, req.person, Tool::Prequalification);
    if lvl < Level::Read {
        return not_found(req);
    }
    let today = req.today();
    let all = visible(state);
    let lapsing = all.iter().filter(|r| matches!(r.standing(today), Standing::ExpiringSoon | Standing::Lapsed)).count();
    let shown: Vec<&Prequalification> = all
        .iter()
        .copied()
        .filter(|r| status.is_none_or(|s| r.status == s))
        .filter(|r| !attention || matches!(r.standing(today), Standing::ExpiringSoon | Standing::Lapsed))
        .collect();
    let mut vendors: Vec<_> = state.vendors.iter().filter(|v| v.deleted.is_none()).collect();
    vendors.sort_by_key(|v| v.name.to_lowercase());
    let cur = edit.and_then(|v| all.iter().copied().find(|r| r.vendor == v));
    let can_edit = lvl >= Level::Standard;
    let admin = lvl == Level::Admin;
    let here = "/prequalification";
    let vname = |v| buildkiosk_engine::view::vendor_name(state, v);
    let body = html! {
        div class="page-head" {
            div { h1 { "Prequalification" } div class="sub" { "Which companies have been vetted to work for you, and when their approval, insurance or licence lapses" } }
            div class="actions" { a class="btn" href=(format!("{here}/forms")) { "Questionnaires" } }
        }
        @if lapsing > 0 { div class="flash error" role="status" { (lapsing) " approved compan" @if lapsing == 1 { "y has" } @else { "ies have" } " something that has lapsed or lapses within " (Prequalification::WARN_DAYS) " days. " a href="/prequalification?attention=1" { "Show them" } } }
        form method="get" action=(here) class="filters" {
            select name="status" aria-label="Status" {
                option value="" { "Any status" }
                @for s in PrequalStatus::ALL { option value=(s.key()) selected[status == Some(*s)] { (s.label()) } }
            }
            label { input type="checkbox" name="attention" value="1" checked[attention]; " Needs attention" }
            button type="submit" class="btn" { "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Prequalification), "No records", if can_edit { "Add a company's prequalification below." } else { "Nothing recorded yet." })) }
        } @else {
            div class="table-wrap" { table class="data" {
                thead { tr { th { "Company" } th { "Status" } th class="num" { "Score" } th { "Approved until" } th { "Insurance" } th { "Licence" } th class="num" { "EMR" } th class="num" { "Bonding" } th {} } }
                tbody { @for r in &shown { tr {
                    td { a class="row-link" href=(format!("{here}/{}", r.vendor)) { (vname(r.vendor)) } @if !r.notes.is_empty() { br; span class="muted small" { (r.notes) } } }
                    td { (status_pill(r.status)) }
                    td class="num" { (r.score.map(|s| s.to_string()).unwrap_or_default()) }
                    td class="nowrap" { (expiry(r.valid_until, today)) }
                    td class="nowrap" { (expiry(r.insurance_expires, today)) }
                    td class="nowrap" { (expiry(r.license_expires, today)) }
                    td class="num" { (r.emr_hundredths.map(|e| format!("{}.{:02}", e / 100, e % 100)).unwrap_or_default()) }
                    td class="num" { (r.bonding_capacity.map(|m| m.to_string()).unwrap_or_default()) }
                    td class="right" {
                        @if can_edit { a class="btn small" href=(format!("{here}?edit={}", r.vendor)) { "Edit" } " " }
                        @if can_edit && r.status == PrequalStatus::Pending { (button_form(req, "prequal.submit", here, &[("vendor", r.vendor.to_string())], "Put forward", "small")) " " }
                        @if admin && matches!(r.status, PrequalStatus::Pending | PrequalStatus::Submitted) {
                            (button_form(req, "prequal.review", here, &[("vendor", r.vendor.to_string()), ("decision", "approved".into())], "Approve", "small primary")) " "
                            (button_form(req, "prequal.review", here, &[("vendor", r.vendor.to_string()), ("decision", "rejected".into())], "Reject", "small danger")) " "
                        }
                        @if admin && matches!(r.status, PrequalStatus::Approved | PrequalStatus::Rejected) { (button_form(req, "prequal.reset", here, &[("vendor", r.vendor.to_string())], "Reset", "small")) }
                    }
                } } }
            } }
        }
        @if can_edit {
            div class="card" {
                h2 { (if cur.is_some() { "Edit record" } else { "Add or update a record" }) }
                (command_form(req, "prequal.save", here, here, html! {
                    div class="form-grid" {
                        (field("Company", "vendor", true, false, html! { select id="vendor" name="vendor" required { @for v in &vendors { option value=(v.id) selected[cur.is_some_and(|r| r.vendor == v.id)] { (v.name) } } } }))
                        (field("Approved until", "valid_until", false, false, html! { input type="date" id="valid_until" name="valid_until" value=(cur.and_then(|r| r.valid_until).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Insurance expires", "insurance_expires", false, false, html! { input type="date" id="insurance_expires" name="insurance_expires" value=(cur.and_then(|r| r.insurance_expires).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Licence expires", "license_expires", false, false, html! { input type="date" id="license_expires" name="license_expires" value=(cur.and_then(|r| r.license_expires).map(|d| d.to_string()).unwrap_or_default()); }))
                        (field("Experience modification rate", "emr", false, false, html! { input type="text" id="emr" name="emr" inputmode="decimal" placeholder="0.85" value=(cur.and_then(|r| r.emr_hundredths).map(|e| format!("{}.{:02}", e / 100, e % 100)).unwrap_or_default()); }))
                        (field("Bonding capacity", "bonding_capacity", false, false, html! { input type="text" id="bonding_capacity" name="bonding_capacity" inputmode="decimal" value=(cur.and_then(|r| r.bonding_capacity).map(|m| m.plain()).unwrap_or_default()); }))
                        (field("Notes", "notes", false, true, html! { textarea id="notes" name="notes" class="short" { (cur.map(|r| r.notes.as_str()).unwrap_or_default()) } }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Save" } }
                }))
            }
        }
    };
    layout::app(req, "Prequalification", Nav::Prequalification, &[("Prequalification", None)], body)
}

/// The standing of a record as a pill.
pub fn standing_pill(s: Standing) -> Markup {
    let (class, text) = match s {
        Standing::Current => ("green", "Prequalified"),
        Standing::ExpiringSoon => ("amber", "Expiring soon"),
        Standing::Lapsed => ("red", "Lapsed"),
        Standing::NotApproved => ("violet", "Not approved"),
    };
    html! { span class=(format!("pill {class}")) { (text) } }
}

/// The text of a questionnaire as the editor takes it: a `# Section` line, then `Question | points` lines.
fn form_text(f: &PrequalForm) -> String {
    f.sections
        .iter()
        .map(|s| format!("# {}\n{}", s.title, s.questions.iter().map(|q| format!("{} | {}", q.text, q.max_points)).collect::<Vec<_>>().join("\n")))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Writing the questionnaires, and the company's rule about requiring prequalification.
pub fn forms_page(req: &Req<'_>, edit: Option<Id<PrequalForm>>) -> Markup {
    let state = req.engine.state();
    let lvl = perm::company_level(state, req.person, Tool::Prequalification);
    if lvl < Level::Read {
        return not_found(req);
    }
    let admin = lvl == Level::Admin;
    let all = forms_list(state);
    let cur = edit.and_then(|id| all.iter().copied().find(|f| f.id == id));
    let here = "/prequalification/forms";
    let required = state.company().is_some_and(|c| c.require_prequalification);
    let body = html! {
        div class="page-head" { div { h1 { "Prequalification questionnaires" } div class="sub" { "Sections of questions, each worth points; the share earned becomes a company's score" } } div class="actions" { a class="btn" href="/prequalification" { "Back to prequalification" } } }
        div class="card" {
            h2 { "Questionnaires" }
            @if all.is_empty() { p class="muted" { "None yet." } } @else {
                table class="data" {
                    thead { tr { th { "Name" } th class="num" { "Sections" } th class="num" { "Questions" } th class="num" { "Points" } th {} } }
                    tbody { @for f in &all { tr {
                        td { strong { (f.name) } }
                        td class="num" { (f.sections.len()) }
                        td class="num" { (f.sections.iter().map(|s| s.questions.len()).sum::<usize>()) }
                        td class="num" { (f.possible()) }
                        td class="right" { @if admin { a class="btn small" href=(format!("{here}?edit={}", f.id)) { "Edit" } " " (button_form(req, "prequal.form_delete", here, &[("id", f.id.to_string())], "Delete", "small danger")) } }
                    } } }
                }
            }
        }
        @if admin {
            div class="card" {
                h2 { (if cur.is_some() { "Edit questionnaire" } else { "New questionnaire" }) }
                p class="muted small" { "One section per `# Title` line; under it, one question per line as `Question | points` (points default to 10)." }
                (command_form(req, "prequal.form_save", here, here, html! {
                    @if let Some(f) = cur { input type="hidden" name="id" value=(f.id); }
                    (field("Name", "form_name", true, true, html! { input type="text" id="form_name" name="name" required value=(cur.map(|f| f.name.as_str()).unwrap_or_default()); }))
                    (field("Questions", "form_text", true, true, html! { textarea id="form_text" name="text" required placeholder="# Safety\nDo you have a written safety program? | 20" { (cur.map(form_text).unwrap_or_default()) } }))
                    div class="form-actions" { button type="submit" class="btn primary" { "Save questionnaire" } }
                }))
            }
            div class="card" {
                h2 { "Require prequalification" }
                p { "Approving a commitment or awarding a bid to a company: " strong { (if required { "needs a current prequalification" } else { "doesn't need one" }) } "." }
                (command_form(req, "prequal.require", here, here, html! {
                    input type="hidden" name="required" value=(if required { "false" } else { "true" });
                    div class="form-actions" { button type="submit" class="btn" { (if required { "Stop requiring it" } else { "Require it" }) } }
                }))
            }
        }
    };
    layout::app(
        req,
        "Questionnaires",
        Nav::Prequalification,
        &[("Prequalification", Some("/prequalification".into())), ("Questionnaires", None)],
        body,
    )
}

/// One company's record: its figures, questionnaire answers and scoring, and files.
pub fn detail(req: &Req<'_>, vendor: Id<Vendor>) -> Markup {
    let state = req.engine.state();
    let lvl = perm::company_level(state, req.person, Tool::Prequalification);
    if lvl < Level::Read {
        return not_found(req);
    }
    let Some(r) = visible(state).into_iter().find(|r| r.vendor == vendor) else { return not_found(req) };
    let today = req.today();
    let name = buildkiosk_engine::view::vendor_name(state, vendor);
    let here = format!("/prequalification/{vendor}");
    let can_edit = lvl >= Level::Standard && (r.status != PrequalStatus::Approved || lvl == Level::Admin);
    let admin = lvl == Level::Admin;
    let form = r.form.and_then(|f| state.prequal_forms.get(f)).filter(|f| f.deleted.is_none());
    let all_forms = forms_list(state);
    let answer_of = |s: usize, q: usize| r.answers.iter().find(|a| a.section as usize == s && a.question as usize == q);
    let v = ("vendor", vendor.to_string());
    let body = html! {
        div class="page-head" {
            div { h1 { (name) " " (status_pill(r.status)) " " (standing_pill(r.standing(today))) } div class="sub" { "Prequalification" @if let Some(s) = r.score { " · score " (s) } } }
            div class="actions" {
                a class="btn" href="/prequalification" { "All companies" }
                @if can_edit && r.status == PrequalStatus::Pending { (button_form(req, "prequal.submit", &here, std::slice::from_ref(&v), "Put forward", "")) }
                @if admin && matches!(r.status, PrequalStatus::Pending | PrequalStatus::Submitted) {
                    (button_form(req, "prequal.review", &here, &[v.clone(), ("decision", "approved".into())], "Approve", "primary"))
                    (button_form(req, "prequal.review", &here, &[v.clone(), ("decision", "rejected".into())], "Reject", "danger"))
                }
            }
        }
        div class="card" {
            dl class="facts" {
                dt { "Approved until" } dd { (expiry(r.valid_until, today)) }
                dt { "Insurance expires" } dd { (expiry(r.insurance_expires, today)) }
                dt { "Licence expires" } dd { (expiry(r.license_expires, today)) }
                dt { "Experience modification rate" } dd { (r.emr_hundredths.map(|e| format!("{}.{:02}", e / 100, e % 100)).unwrap_or_else(|| "—".into())) }
                dt { "Bonding capacity" } dd { (r.bonding_capacity.map(|m| m.to_string()).unwrap_or_else(|| "—".into())) }
            }
            @if !r.notes.is_empty() { p class="prose" { (r.notes) } }
        }
        div class="card" {
            h2 { "Questionnaire" }
            @if can_edit {
                (command_form(req, "prequal.answer", &here, &here, html! {
                    input type="hidden" name="vendor" value=(vendor);
                    div class="form-grid" {
                        (field("Questionnaire", "qform", false, false, html! { select id="qform" name="form" {
                            @if form.is_none() { option value="" { "Choose one" } }
                            @for f in &all_forms { option value=(f.id) selected[r.form == Some(f.id)] { (f.name) } }
                        } }))
                    }
                    p class="muted small" { "Choosing a different questionnaire clears the answers." }
                    div class="form-actions" { button type="submit" class="btn small" { "Use this questionnaire" } }
                }))
            }
            @if let Some(f) = form {
                @let (earned, possible) = r.points(f, None);
                p { strong { (earned) " of " (possible) " points" } @if possible > 0 { " (" ((u64::from(earned) * 100 / u64::from(possible))) "%)" } }
                @if can_edit {
                    h3 { "Answers" }
                    (command_form(req, "prequal.answer", &here, &here, html! {
                        input type="hidden" name="vendor" value=(vendor);
                        @for (si, s) in f.sections.iter().enumerate() {
                            h4 { (s.title) }
                            @for (qi, q) in s.questions.iter().enumerate() {
                                @let n = f.sections.iter().take(si).map(|x| x.questions.len()).sum::<usize>() + qi;
                                @let a = answer_of(si, qi);
                                div class="field full" {
                                    div class="label" { (q.text) }
                                    input type="hidden" name=(format!("replies.{n}.section")) value=(si);
                                    input type="hidden" name=(format!("replies.{n}.question")) value=(qi);
                                    textarea name=(format!("replies.{n}.text")) class="short" aria-label=(q.text.clone()) { (a.map(|a| a.text.as_str()).unwrap_or_default()) }
                                }
                            }
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Save answers" } }
                    }))
                }
                @if admin {
                    h3 { "Points" }
                    (command_form(req, "prequal.grade", &here, &here, html! {
                        input type="hidden" name="vendor" value=(vendor);
                        @for (si, s) in f.sections.iter().enumerate() {
                            @let (se, sp) = r.points(f, Some(si as u32));
                            h4 { (s.title) " " span class="muted small" { (se) " of " (sp) } }
                            @for (qi, q) in s.questions.iter().enumerate() {
                                @let n = f.sections.iter().take(si).map(|x| x.questions.len()).sum::<usize>() + qi;
                                @let a = answer_of(si, qi);
                                div class="field full" {
                                    div class="label" { (q.text) " " span class="muted small" { "(" (q.max_points) " points)" } }
                                    @if let Some(t) = a.map(|a| a.text.as_str()).filter(|t| !t.is_empty()) { p class="prose" { (t) } } @else { p class="muted" { "No answer recorded." } }
                                    input type="hidden" name=(format!("points.{n}.section")) value=(si);
                                    input type="hidden" name=(format!("points.{n}.question")) value=(qi);
                                    label { "Points " input type="number" min="0" max=(q.max_points) name=(format!("points.{n}.points")) value=(a.and_then(|a| a.points).map(|p| p.to_string()).unwrap_or_default()); }
                                }
                            }
                        }
                        div class="form-actions" { button type="submit" class="btn primary" { "Save points and score" } }
                    }))
                }
                @if !can_edit && !admin {
                    @for (si, s) in f.sections.iter().enumerate() {
                        h4 { (s.title) }
                        @for (qi, q) in s.questions.iter().enumerate() { p { strong { (q.text) } br; (answer_of(si, qi).map(|a| a.text.as_str()).unwrap_or("—")) } }
                    }
                }
            } @else { p class="muted" { "No questionnaire chosen." } }
        }
        div class="card" {
            h2 { "Files" }
            @if r.attachments.is_empty() { p class="muted" { "No files yet. Attach the insurance certificate, financial statements or licences." } } @else {
                ul class="plain" { @for f in &r.attachments { li {
                    a href=(format!("{here}/files/{}", f.sha256)) { (f.name) } " " span class="muted small" { (human_size(f.size)) }
                    @if admin { " " (button_form(req, "prequal.detach", &here, &[v.clone(), ("sha256", f.sha256.clone())], "Remove", "small danger")) }
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
        @if !r.reminders.is_empty() {
            div class="card" {
                h2 { "Warnings sent" }
                ul class="plain" { @for x in r.reminders.iter().rev().take(10) { li { (x.what) " " (x.date.pretty()) ": " (if x.days == 0 { "warned that it had lapsed".to_string() } else { format!("warned {} days ahead", x.days) }) } } }
            }
        }
    };
    layout::app(req, &name, Nav::Prequalification, &[("Prequalification", Some("/prequalification".into())), (&name, None)], body)
}
