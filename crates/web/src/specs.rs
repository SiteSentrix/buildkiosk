//! The specifications page: a project's section list and the submittals filed under each section.

use buildkiosk_core::Id;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use maud::{Markup, html};

use crate::icons::tool_icon;
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{not_found, project_for};
use crate::submittals::slug;

pub fn page(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Tool::Specifications) < Level::Read {
        return not_found(req);
    }
    let mut sections: Vec<_> = state.spec_sections.iter().filter(|s| s.project == id && s.deleted.is_none()).collect();
    sections.sort_by(|a, b| a.number.cmp(&b.number));
    let submittals = view::submittals_for(state, req.actor(), id);
    let here = format!("/projects/{id}/specifications");
    let can_edit = req.allowed("spec.add", Some(id));
    let body = html! {
        div class="page-head" { div { h1 { "Specifications" } div class="sub" { "The project's section list; submittals link to a section by its number" } } }
        @if sections.is_empty() {
            div class="card" { (empty(tool_icon(Tool::Specifications), "No sections yet", "Add sections one at a time or import your own list below.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Section" } th { "Title" } th { "Submittals" } @if can_edit { th {} } } }
                    tbody {
                        @for sec in &sections {
                            @let linked: Vec<_> = submittals.iter().filter(|s| s.spec_section == sec.number).collect();
                            tr id=(format!("s-{}", slug(&sec.number))) {
                                td class="nowrap" { (sec.number) }
                                td { (sec.title) }
                                td { @for s in &linked { a href=(format!("/projects/{id}/submittals/{}", s.id)) { "#" (s.number) } " " } @if linked.is_empty() { span class="muted" { "—" } } }
                                @if can_edit { td class="right" { (button_form(req, "spec.delete", &here, &[("project", id.to_string()), ("id", sec.id.to_string())], "Delete", "small danger")) } }
                            }
                        }
                    }
                }
            }
        }
        @if can_edit {
            div class="card" {
                h2 { "Add a section" }
                (command_form(req, "spec.add", &here, &here, html! {
                    input type="hidden" name="project" value=(id);
                    div class="form-grid" {
                        (field("Section number", "number", true, false, html! { input type="text" id="number" name="number" required; }))
                        (field("Title", "title", false, false, html! { input type="text" id="title" name="title"; }))
                    }
                    div class="form-actions" { button type="submit" class="btn primary" { "Add section" } }
                }))
            }
            div class="card" {
                details class="disclosure" {
                    summary { "Import a list of sections" }
                    p class="muted small" { "One section per line: number, then title, separated by a comma or a tab. Sections already in the list are skipped." }
                    (command_form(req, "spec.import", &here, &here, html! {
                        input type="hidden" name="project" value=(id);
                        (field("Sections", "spec_text", true, true, html! { textarea id="spec_text" name="text" required {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
    };
    layout::app(
        req,
        "Specifications",
        Nav::Specs(id),
        &[("Projects", Some("/".into())), (&p.name, Some(format!("/projects/{id}"))), ("Specifications", None)],
        body,
    )
}
