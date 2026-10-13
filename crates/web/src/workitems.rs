//! Pages for the tools that share the work item workflow (punch list and observations): list,
//! detail, form and CSV export, written once over [`WorkPage`].

use buildkiosk_core::Id;
use buildkiosk_engine::model::incident::{Incident, Severity};
use buildkiosk_engine::model::observation::Observation;
use buildkiosk_engine::model::project::Project;
use buildkiosk_engine::model::punch::PunchItem;
use buildkiosk_engine::model::task::Task;
use buildkiosk_engine::model::work::WorkStatus;
use buildkiosk_engine::model::{Choice, Level, Priority};
use buildkiosk_engine::workitem::{WorkItem, can_edit, can_see, items_for, label};
use buildkiosk_engine::{JournalEntry, find_command, view};
use maud::{Markup, html};

use crate::icons::{Icon, tool_icon};
use crate::layout::{self, Nav, Req, button_form, command_form, empty, field};
use crate::pages::{dash, human_size, not_found, project_for, user_options};

/// How one work item tool presents itself.
pub trait WorkPage: WorkItem {
    /// The command prefix: `punch` for `punch.create`.
    const PREFIX: &'static str;
    /// The page title: "Punch list".
    const TITLE: &'static str;
    const SUBTITLE: &'static str;
    const ADD: &'static str;
    /// What the free-text kind field is called, with an example.
    const KIND_LABEL: &'static str;
    const KIND_HINT: &'static str;
    const EMPTY_HINT: &'static str;
    /// Whether the tool has a trends page at `/trends`.
    const TRENDS: bool = false;

    fn nav(project: Id<Project>) -> Nav;
    /// The categories the project offers for the free-text kind field (empty: any text).
    fn types(_project: &Project) -> &[String] {
        &[]
    }
    /// Cards shown above the comments (kind-specific).
    fn extra_cards(&self) -> Markup {
        html! {}
    }
    /// Rows of the details panel (kind-specific).
    fn extra_facts(&self) -> Markup {
        html! {}
    }
    /// Form fields (kind-specific); `item` is `None` on the new form.
    fn extra_fields(_item: Option<&Self>, _req: &Req<'_>) -> Markup {
        html! {}
    }
}

impl WorkPage for PunchItem {
    const PREFIX: &'static str = "punch";
    const TITLE: &'static str = "Punch list";
    const SUBTITLE: &'static str = "Small items to fix or finish before closeout";
    const ADD: &'static str = "Add punch item";
    const KIND_LABEL: &'static str = "Type";
    const KIND_HINT: &'static str = "Paint, electrical, glazing…";
    const EMPTY_HINT: &'static str = "Add the first item to get started.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Punch(project)
    }
}

impl WorkPage for Task {
    const PREFIX: &'static str = "task";
    const TITLE: &'static str = "Tasks";
    const SUBTITLE: &'static str = "Things to get done, with an owner and a due date";
    const ADD: &'static str = "Add task";
    const KIND_LABEL: &'static str = "Category";
    const KIND_HINT: &'static str = "Closeout, procurement, permits…";
    const EMPTY_HINT: &'static str = "Add the first task to get started.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Tasks(project)
    }
}

impl WorkPage for Incident {
    const PREFIX: &'static str = "incident";
    const TITLE: &'static str = "Incidents";
    const SUBTITLE: &'static str = "Injuries, near misses and damage, with the investigation and what was done about it";
    const ADD: &'static str = "Report incident";
    const KIND_LABEL: &'static str = "Type";
    const KIND_HINT: &'static str = "Fall, struck by, equipment, spill…";
    const EMPTY_HINT: &'static str = "Report an incident to record what happened and what was done.";

    fn nav(project: Id<Project>) -> Nav {
        Nav::Incidents(project)
    }

    fn extra_cards(&self) -> Markup {
        html! {
            @if !self.people_involved.is_empty() { div class="card" { h2 { "People involved" } p class="prose" { (self.people_involved) } } }
            @if !self.root_cause.is_empty() { div class="card" { h2 { "Root cause" } p class="prose" { (self.root_cause) } } }
            @if !self.corrective_action.is_empty() { div class="card" { h2 { "Corrective action" } p class="prose" { (self.corrective_action) } } }
        }
    }

    fn extra_facts(&self) -> Markup {
        html! {
            dt { "Occurred" } dd { @if let Some(d) = self.occurred { (d.pretty()) } @else { "—" } }
            dt { "Severity" } dd { (self.severity.label()) }
            dt { "Recordable" } dd { (if self.recordable { "Yes" } else { "No" }) }
        }
    }

    fn extra_fields(item: Option<&Self>, req: &Req<'_>) -> Markup {
        let occurred = item.map(|i| i.occurred).unwrap_or_else(|| Some(req.today())).map(|d| d.to_string()).unwrap_or_default();
        let severity = item.map(|i| i.severity).unwrap_or_default();
        let recordable = item.is_some_and(|i| i.recordable);
        let text = |f: fn(&Incident) -> &str| item.map(|i| f(i).to_string()).unwrap_or_default();
        html! {
            (field("Date it happened", "occurred", false, false, html! { input type="date" id="occurred" name="occurred" value=(occurred); }))
            (field("Severity", "severity", false, false, html! { select id="severity" name="severity" { @for s in Severity::ALL { option value=(s.key()) selected[*s == severity] { (s.label()) } } } }))
            (field("Recordable", "recordable", false, false, html! { select id="recordable" name="recordable" {
                option value="false" selected[!recordable] { "No" }
                option value="true" selected[recordable] { "Yes, enter it on the injury log" }
            } }))
            (field("People involved and witnesses", "people_involved", false, true, html! { textarea id="people_involved" name="people_involved" class="short" { (text(|i| &i.people_involved)) } }))
            (field("Root cause", "root_cause", false, true, html! { textarea id="root_cause" name="root_cause" class="short" { (text(|i| &i.root_cause)) } }))
            (field("Corrective action", "corrective_action", false, true, html! { textarea id="corrective_action" name="corrective_action" class="short" { (text(|i| &i.corrective_action)) } }))
        }
    }
}

impl WorkPage for Observation {
    const PREFIX: &'static str = "observation";
    const TITLE: &'static str = "Observations";
    const SUBTITLE: &'static str = "Safety, quality and housekeeping findings";
    const ADD: &'static str = "Add observation";
    const KIND_LABEL: &'static str = "Category";
    const KIND_HINT: &'static str = "Safety, quality, housekeeping…";
    const EMPTY_HINT: &'static str = "Add the first observation to get started.";
    const TRENDS: bool = true;

    fn nav(project: Id<Project>) -> Nav {
        Nav::Observations(project)
    }

    fn types(project: &Project) -> &[String] {
        &project.observation_types
    }

    fn extra_cards(&self) -> Markup {
        html! { @if !self.corrective_action.is_empty() { div class="card" { h2 { "Corrective action" } p class="prose" { (self.corrective_action) } } } }
    }

    fn extra_facts(&self) -> Markup {
        html! { dt { "Observed" } dd { @if let Some(d) = self.observed { (d.pretty()) } @else { "—" } } }
    }

    fn extra_fields(item: Option<&Self>, req: &Req<'_>) -> Markup {
        let observed = item.map(|i| i.observed).unwrap_or_else(|| Some(req.today())).map(|d| d.to_string()).unwrap_or_default();
        let action = item.map(|i| i.corrective_action.clone()).unwrap_or_default();
        html! {
            (field("Date observed", "observed", false, false, html! { input type="date" id="observed" name="observed" value=(observed); }))
            (field("Corrective action", "corrective_action", false, true, html! { textarea id="corrective_action" name="corrective_action" class="short" { (action) } }))
        }
    }
}

fn status_pill(s: WorkStatus) -> Markup {
    let class = match s {
        WorkStatus::Open => "blue",
        WorkStatus::ReadyForReview => "amber",
        WorkStatus::Closed => "green",
    };
    html! { span class=(format!("pill {class}")) { (s.label()) } }
}

fn priority_tag(p: Priority) -> Markup {
    let class = match p {
        Priority::Urgent => "pill red",
        Priority::High => "pill amber",
        _ => "tag",
    };
    html! { span class=(class) { (p.label()) } }
}

/// How a list is divided into sections.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    None,
    Location,
    Assignee,
    Kind,
    Status,
}

impl Group {
    pub fn parse(s: &str) -> Group {
        match s {
            "location" => Group::Location,
            "assignee" => Group::Assignee,
            "kind" => Group::Kind,
            "status" => Group::Status,
            _ => Group::None,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Group::None => "",
            Group::Location => "location",
            Group::Assignee => "assignee",
            Group::Kind => "kind",
            Group::Status => "status",
        }
    }
}

/// Filters of the list, from the query string.
#[derive(Default)]
pub struct Filter {
    pub status: Option<WorkStatus>,
    pub q: String,
    pub mine: bool,
    /// Only items assigned to this person (a person id).
    pub assignee: Option<u64>,
    /// Only items at exactly this location, of exactly this type (case ignored); empty for any.
    pub location: String,
    pub kind: String,
    pub group: Group,
}

impl Filter {
    /// The query string that reproduces this filter (for links that keep it).
    pub fn query(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let enc =
            |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
        if let Some(s) = self.status {
            parts.push(format!("status={}", s.key()));
        }
        if !self.q.is_empty() {
            parts.push(format!("q={}", enc(&self.q)));
        }
        if self.mine {
            parts.push("mine=1".into());
        }
        if let Some(a) = self.assignee {
            parts.push(format!("assignee={a}"));
        }
        if !self.location.is_empty() {
            parts.push(format!("location={}", enc(&self.location)));
        }
        if !self.kind.is_empty() {
            parts.push(format!("kind={}", enc(&self.kind)));
        }
        if self.group != Group::None {
            parts.push(format!("group={}", self.group.key()));
        }
        parts.join("&")
    }
}

/// The items the filter lets through, in number order.
pub fn filtered<'a, T: WorkPage>(req: &Req<'_>, all: &[&'a T], filter: &Filter) -> Vec<&'a T> {
    let q = filter.q.to_lowercase();
    all.iter()
        .copied()
        .filter(|i| filter.status.is_none_or(|s| i.status == s))
        .filter(|i| !filter.mine || i.ball_in_court.contains(&req.person))
        .filter(|i| filter.assignee.is_none_or(|a| i.assignees.iter().any(|x| x.raw() == a)))
        .filter(|i| filter.location.is_empty() || i.location.eq_ignore_ascii_case(&filter.location))
        .filter(|i| filter.kind.is_empty() || i.kind.eq_ignore_ascii_case(&filter.kind))
        .filter(|i| q.is_empty() || i.title.to_lowercase().contains(&q) || i.location.to_lowercase().contains(&q) || i.number.to_string() == q)
        .collect()
}

/// The items split into titled sections (one section with no title when not grouping). Sections
/// are in alphabetical order; an item with several assignees appears under each.
pub fn sections<'a, T: WorkPage>(req: &Req<'_>, shown: &[&'a T], group: Group) -> Vec<(String, Vec<&'a T>)> {
    let state = req.engine.state();
    let mut map: std::collections::BTreeMap<String, Vec<&'a T>> = std::collections::BTreeMap::new();
    for i in shown {
        let keys: Vec<String> = match group {
            Group::None => vec![String::new()],
            Group::Location => vec![if i.location.is_empty() { "No location".into() } else { i.location.clone() }],
            Group::Kind => vec![if i.kind.is_empty() { format!("No {}", T::KIND_LABEL.to_lowercase()) } else { i.kind.clone() }],
            Group::Status => vec![i.status.label().to_string()],
            Group::Assignee if i.assignees.is_empty() => vec!["Unassigned".into()],
            Group::Assignee => i.assignees.iter().map(|a| view::person_name(state, *a)).collect(),
        };
        for k in keys {
            map.entry(k).or_default().push(i);
        }
    }
    map.into_iter().collect()
}

fn crumbs<'a, T: WorkPage>(p: &'a Project, last: Option<(&'a str, String)>) -> Vec<(&'a str, Option<String>)> {
    let mut c = vec![("Projects", Some("/".to_string())), (p.name.as_str(), Some(format!("/projects/{}", p.id)))];
    match last {
        Some((l, list)) => {
            c.push((T::TITLE, Some(list)));
            c.push((l, None));
        }
        None => c.push((T::TITLE, None)),
    }
    c
}

fn cmd<T: WorkPage>(verb: &str) -> String {
    format!("{}.{verb}", T::PREFIX)
}

pub fn list<T: WorkPage>(req: &Req<'_>, id: Id<Project>, filter: &Filter) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, T::TOOL) < Level::Read {
        return not_found(req);
    }
    let all = items_for::<T>(state, req.actor(), id);
    let count = |s: WorkStatus| all.iter().filter(|x| x.status == s).count();
    let today = req.today();
    let shown = filtered::<T>(req, &all, filter);
    let base = format!("/projects/{id}/{}", T::PATH);
    // Tabs keep the other filters; only the status changes.
    let rest = Filter {
        status: None,
        q: filter.q.clone(),
        mine: false,
        assignee: filter.assignee,
        location: filter.location.clone(),
        kind: filter.kind.clone(),
        group: filter.group,
    };
    let tab = |label: &str, status: Option<WorkStatus>, n: usize| {
        let q =
            Filter { status, ..Filter { q: rest.q.clone(), location: rest.location.clone(), kind: rest.kind.clone(), ..Filter::default() } }.query();
        let extra = [rest.assignee.map(|a| format!("assignee={a}")), (rest.group != Group::None).then(|| format!("group={}", rest.group.key()))];
        let q = std::iter::once(q).chain(extra.into_iter().flatten()).filter(|x| !x.is_empty()).collect::<Vec<_>>().join("&");
        let href = if q.is_empty() { base.clone() } else { format!("{base}?{q}") };
        html! { a href=(href) class=(if filter.status == status && !filter.mine { "active" } else { "" }) { (label) span class="count" { (n) } } }
    };
    let mut locations: Vec<&str> = all.iter().map(|i| i.location.as_str()).filter(|l| !l.is_empty()).collect();
    locations.sort_by_key(|l| l.to_lowercase());
    locations.dedup_by_key(|l| l.to_lowercase());
    let mut kinds: Vec<&str> = all.iter().map(|i| i.kind.as_str()).filter(|l| !l.is_empty()).collect();
    kinds.sort_by_key(|l| l.to_lowercase());
    kinds.dedup_by_key(|l| l.to_lowercase());
    let mut people: Vec<_> = all.iter().flat_map(|i| i.assignees.iter().copied()).collect();
    people.sort_by_key(|p| view::person_name(state, *p).to_lowercase());
    people.dedup();
    let query = filter.query();
    let with_query = |path: String| if query.is_empty() { path } else { format!("{path}?{query}") };
    let sections = sections::<T>(req, &shown, filter.group);
    let body = html! {
        div class="page-head" {
            div { h1 { (T::TITLE) } div class="sub" { (T::SUBTITLE) } }
            div class="actions" {
                @if T::TRENDS { a class="btn" href=(format!("{base}/trends")) { "Trends" } }
                a class="btn" href=(with_query(format!("{base}.pdf"))) { "PDF" }
                a class="btn" href=(format!("{base}.csv")) { (Icon::Download.svg()) "Export CSV" }
                @if req.allowed(&cmd::<T>("create"), Some(id)) { a class="btn primary" href=(format!("{base}/new")) { (Icon::Plus.svg()) (T::ADD) } }
            }
        }
        div class="tabs" {
            (tab("All", None, all.len()))
            (tab("Open", Some(WorkStatus::Open), count(WorkStatus::Open)))
            (tab("Ready for review", Some(WorkStatus::ReadyForReview), count(WorkStatus::ReadyForReview)))
            (tab("Closed", Some(WorkStatus::Closed), count(WorkStatus::Closed)))
        }
        form method="get" action=(base) class="filters" {
            @if let Some(s) = filter.status { input type="hidden" name="status" value=(s.key()); }
            input type="search" name="q" value=(filter.q) placeholder="Search title, location or number" aria-label=(format!("Search {}", T::TITLE.to_lowercase()));
            select name="assignee" aria-label="Assigned to" { option value="" { "Anyone" } @for a in &people { option value=(a) selected[filter.assignee == Some(a.raw())] { (view::person_name(state, *a)) } } }
            select name="location" aria-label="Location" { option value="" { "Any location" } @for l in &locations { option value=(l) selected[filter.location.eq_ignore_ascii_case(l)] { (l) } } }
            select name="kind" aria-label=(T::KIND_LABEL) { option value="" { "Any " (T::KIND_LABEL.to_lowercase()) } @for k in &kinds { option value=(k) selected[filter.kind.eq_ignore_ascii_case(k)] { (k) } } }
            select name="group" aria-label="Group by" {
                option value="" selected[filter.group == Group::None] { "No grouping" }
                option value="location" selected[filter.group == Group::Location] { "Group by location" }
                option value="assignee" selected[filter.group == Group::Assignee] { "Group by assignee" }
                option value="kind" selected[filter.group == Group::Kind] { "Group by " (T::KIND_LABEL.to_lowercase()) }
                option value="status" selected[filter.group == Group::Status] { "Group by status" }
            }
            label class="check" { input type="checkbox" name="mine" value="1" checked[filter.mine]; "Waiting on me" }
            button type="submit" class="btn small" { (Icon::Search.svg()) "Filter" }
        }
        @if shown.is_empty() {
            div class="card" { (empty(tool_icon(T::TOOL), "No items here", if all.is_empty() { T::EMPTY_HINT } else { "No item matches these filters." })) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "#" } th { "Title" } th { "Location" } th { "Priority" } th { "Status" } th { "Ball in court" } th { "Due" } } }
                    @for (title, items) in &sections {
                        tbody {
                            @if filter.group != Group::None { tr class="group-row" { th colspan="7" { (title) " " span class="muted" { "(" (items.len()) ")" } } } }
                            @for i in items {
                                tr {
                                    td class="nowrap" { (i.number) }
                                    td { a class="row-link" href=(format!("{base}/{}", i.id())) { (i.title) } }
                                    td { (i.location) }
                                    td { (priority_tag(i.priority)) }
                                    td { (status_pill(i.status)) }
                                    td { (view::people_names(state, &i.ball_in_court)) }
                                    td class="nowrap" { @if let Some(d) = i.due { span class=(if i.is_overdue(today) { "overdue" } else { "" }) { (d.pretty()) } } }
                                }
                            }
                        }
                    }
                }
            }
        }
        @if req.allowed(&cmd::<T>("import"), Some(id)) {
            div class="card" {
                details class="disclosure" {
                    summary { "Import from CSV" }
                    p class="muted small" { "One per line: title, location, " (T::KIND_LABEL.to_lowercase()) ", priority (low, medium, high, urgent), due date (YYYY-MM-DD), assignees (email addresses separated by semicolons). A header line starting with title is skipped. Nothing is added unless every line is good." }
                    form method="post" action=(format!("{base}/import")) enctype="multipart/form-data" class="stack" {
                        input type="hidden" name="_csrf" value=(req.csrf);
                        (field("CSV file", "import_file", true, true, html! { input type="file" id="import_file" name="file" accept=".csv,text/csv,text/plain" required; }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import file" } }
                    }
                    p class="muted small" { "Or paste the lines:" }
                    (command_form(req, &cmd::<T>("import"), &base, &base, html! {
                        input type="hidden" name="project" value=(id);
                        (field("CSV text", "import_text", true, true, html! { textarea id="import_text" name="text" required placeholder="title,location,kind,priority,due,assignees" {} }))
                        div class="form-actions" { button type="submit" class="btn primary" { "Import" } }
                    }))
                }
            }
        }
    };
    layout::app(req, T::TITLE, T::nav(id), &crumbs::<T>(p, None), body)
}

pub fn detail<T: WorkPage>(req: &Req<'_>, id: Id<Project>, iid: Id<T>, history: &[JournalEntry]) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let Some(i) = T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i)) else { return not_found(req) };
    let base = format!("/projects/{id}/{}", T::PATH);
    let here = format!("{base}/{}", i.id());
    let ids = [("project", id.to_string()), ("id", i.id().to_string())];
    let level = view::level(state, req.actor(), id, T::TOOL);
    let editable = can_edit(state, req.actor(), i);
    let manager_like = level == Level::Admin || i.manager == Some(req.person);
    let assignee = i.assignees.contains(&req.person);
    let standard = level >= Level::Standard;
    let can_attach = i.status != WorkStatus::Closed && (editable || (standard && assignee));
    let name = label(i);
    let body = html! {
        div class="page-head" {
            div {
                h1 { (name) ": " (i.title) " " (status_pill(i.status)) }
                div class="sub" { "Added " (req.when(i.created.at)) @if let Some(b) = i.created.by { " by " (view::person_name(state, b)) } }
            }
            div class="actions" {
                @if editable { a class="btn" href=(format!("{here}/edit")) { "Edit" } }
                @if standard && i.status == WorkStatus::Open && (assignee || manager_like) { (button_form(req, &cmd::<T>("complete"), &here, &ids, "Mark work done", "primary")) }
                @if standard && manager_like && i.status != WorkStatus::Closed { (button_form(req, &cmd::<T>("close"), &here, &ids, "Close item", if i.status == WorkStatus::ReadyForReview { "primary" } else { "" })) }
                @if standard && manager_like && i.status == WorkStatus::Closed { (button_form(req, &cmd::<T>("reopen"), &here, &ids, "Reopen", "")) }
                @if standard && (level == Level::Admin || (i.status == WorkStatus::Open && i.created.by == Some(req.person))) { (button_form(req, &cmd::<T>("delete"), &base, &ids, "Delete", "danger")) }
            }
        }
        div class="detail" {
            div {
                @if !i.description.is_empty() { div class="card" { h2 { "Description" } p class="prose" { (i.description) } } }
                (i.extra_cards())
                @if i.status == WorkStatus::ReadyForReview && standard && manager_like {
                    div class="card" {
                        h2 { "Not right yet?" }
                        (command_form(req, &cmd::<T>("send_back"), &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id());
                            (field("What still needs doing", "sb_comment", true, true, html! { textarea id="sb_comment" name="comment" class="short" required {} }))
                            div class="form-actions" { button type="submit" class="btn" { "Send back" } }
                        }))
                    }
                }
                div class="card" {
                    h2 { "Comments" }
                    @if i.comments.is_empty() { p class="muted" { "No comments yet." } }
                    @for c in &i.comments {
                        div class="response" {
                            div class="meta" { span { strong { @if let Some(b) = c.by { (view::person_name(state, b)) } @else { "System" } } " · " (req.when(c.at)) } }
                            p class="prose" { (c.body) }
                        }
                    }
                    @if standard && i.status != WorkStatus::Closed {
                        (command_form(req, &cmd::<T>("comment"), &here, &here, html! {
                            input type="hidden" name="project" value=(id);
                            input type="hidden" name="id" value=(i.id());
                            (field("Add a comment", "body", true, true, html! { textarea id="body" name="body" class="short" required {} }))
                            div class="form-actions" { button type="submit" class="btn primary" { "Post comment" } }
                        }))
                    }
                }
            }
            div class="stack" {
                div class="card" {
                    h2 { "Details" }
                    dl class="facts" {
                        dt { "Priority" } dd { (priority_tag(i.priority)) }
                        dt { "Location" } dd { (dash(&i.location)) }
                        @if !i.drawing_number.is_empty() {
                            dt { "Drawing sheet" } dd {
                                @if let Some(d) = state.drawings.iter().find(|d| d.project == id && d.deleted.is_none() && d.number.eq_ignore_ascii_case(&i.drawing_number)) { a href=(format!("/projects/{id}/drawings/{}", d.id)) { (i.drawing_number) } } @else { (i.drawing_number) }
                            }
                        }
                        dt { (T::KIND_LABEL) } dd { (dash(&i.kind)) }
                        (i.extra_facts())
                        dt { "Manager" } dd { @if let Some(m) = i.manager { (view::person_name(state, m)) } @else { "—" } }
                        dt { "Assigned to" } dd { @if i.assignees.is_empty() { "—" } @else { (view::people_names(state, &i.assignees)) } }
                        dt { "Ball in court" } dd { @if i.ball_in_court.is_empty() { "—" } @else { (view::people_names(state, &i.ball_in_court)) } }
                        dt { "Due" } dd { @if let Some(d) = i.due { span class=(if i.is_overdue(req.today()) { "overdue" } else { "" }) { (d.pretty()) } } @else { "—" } }
                        @if let Some(w) = i.work_done { dt { "Work done" } dd { (req.when(w.at)) } }
                        @if let Some(c) = i.closed { dt { "Closed" } dd { (req.when(c.at)) } }
                    }
                }
                div class="card" {
                    h2 { "Files" }
                    @if i.attachments.is_empty() { p class="muted" { "No files attached." } }
                    @for a in &i.attachments {
                        div class="row" {
                            a href=(format!("{here}/files/{}", a.sha256)) { (a.name) }
                            span class="muted small" { (human_size(a.size)) }
                            @if can_attach { (button_form(req, &cmd::<T>("detach"), &here, &[("project", id.to_string()), ("id", i.id().to_string()), ("sha256", a.sha256.clone())], "Remove", "small danger")) }
                        }
                    }
                    @if can_attach {
                        form method="post" action=(format!("{here}/upload")) enctype="multipart/form-data" class="stack" {
                            input type="hidden" name="_csrf" value=(req.csrf);
                            input type="file" name="file" required aria-label="File to attach";
                            button type="submit" class="btn small" { "Attach file" }
                        }
                    }
                }
                div class="card" {
                    h2 { "History" }
                    ul class="timeline" {
                        @for e in history.iter().rev() {
                            li {
                                div { (find_command(&e.cmd).map(|c| c.label).unwrap_or(e.cmd.as_str())) " by " @if let Some(a) = e.actor { (view::person_name(state, a)) } @else { "the system" } }
                                div class="when" { (req.when(e.at)) }
                            }
                        }
                    }
                }
            }
        }
    };
    layout::app(req, &name, T::nav(id), &crumbs::<T>(p, Some((&name, base.clone()))), body)
}

pub fn form<T: WorkPage>(req: &Req<'_>, id: Id<Project>, existing: Option<Id<T>>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    let current = match existing {
        Some(iid) => {
            match T::table(state).get(iid).filter(|i| i.project() == id && can_see(state, req.actor(), *i) && can_edit(state, req.actor(), *i)) {
                Some(i) => Some(i),
                None => return not_found(req),
            }
        }
        None => {
            if !req.allowed(&cmd::<T>("create"), Some(id)) {
                return not_found(req);
            }
            None
        }
    };
    let people = user_options(req, id, true);
    let base = format!("/projects/{id}/{}", T::PATH);
    let (command, next, back) = match current {
        Some(i) => (cmd::<T>("update"), format!("{base}/{}", i.id()), format!("{base}/{}/edit", i.id())),
        None => (cmd::<T>("create"), format!("{base}/{{id}}"), format!("{base}/new")),
    };
    let text = |f: fn(&T) -> String| current.map(f).unwrap_or_default();
    let manager = current.and_then(|i| i.manager).unwrap_or(req.person);
    let priority = current.map(|i| i.priority).unwrap_or_default();
    let title = if current.is_some() { format!("Edit {}", T::NOUN) } else { T::ADD.to_string() };
    let body = html! {
        div class="page-head" { h1 { (title) } }
        div class="card" {
            (command_form(req, &command, &next, &back, html! {
                input type="hidden" name="project" value=(id);
                @if let Some(i) = current { input type="hidden" name="id" value=(i.id()); }
                div class="form-grid" {
                    (field("Title", "title", true, true, html! { input type="text" id="title" name="title" required maxlength="200" value=(text(|i| i.title.clone())); }))
                    (field("Description", "description", false, true, html! { textarea id="description" name="description" class="short" { (text(|i| i.description.clone())) } }))
                    (field("Location", "location", false, false, html! { input type="text" id="location" name="location" value=(text(|i| i.location.clone())); }))
                    (field("Drawing sheet", "drawing_number", false, false, html! {
                        input type="text" id="drawing_number" name="drawing_number" list="sheet-numbers" value=(text(|i| i.drawing_number.clone()));
                        datalist id="sheet-numbers" { @for d in req.engine.state().drawings.iter().filter(|d| d.project == id && d.deleted.is_none()) { option value=(d.number) { (d.title) } } }
                    }))
                    @let types = T::types(p);
                    @let kind = text(|i| i.kind.clone());
                    @if types.is_empty() {
                        (field(T::KIND_LABEL, "kind", false, false, html! { input type="text" id="kind" name="kind" placeholder=(T::KIND_HINT) value=(kind); }))
                    } @else {
                        (field(T::KIND_LABEL, "kind", false, false, html! { select id="kind" name="kind" {
                            option value="" { "—" }
                            @for t in types { option value=(t) selected[t.eq_ignore_ascii_case(&kind)] { (t) } }
                            @if !kind.is_empty() && !types.iter().any(|t| t.eq_ignore_ascii_case(&kind)) { option value=(kind) selected { (kind) } }
                        } }))
                    }
                    (T::extra_fields(current, req))
                    (field("Priority", "priority", false, false, html! { select id="priority" name="priority" { @for pr in [Priority::Low, Priority::Medium, Priority::High, Priority::Urgent] { option value=(pr.key()) selected[pr == priority] { (pr.label()) } } } }))
                    (field("Due date", "due", false, false, html! { input type="date" id="due" name="due" value=(current.and_then(|i| i.due).map(|d| d.to_string()).unwrap_or_default()); }))
                    (field("Manager (accepts the work)", "manager", false, false, html! { select id="manager" name="manager" { @for x in &people { option value=(x.id) selected[x.id == manager] { (x.name()) } } } }))
                    div class="field full" {
                        div class="label" { "Assigned to" }
                        input type="hidden" name="assignees[]" value="";
                        details class="multi" {
                            summary { "Choose who does the work" }
                            div class="options" { @for x in &people {
                                label { input type="checkbox" name="assignees[]" value=(x.id) checked[current.is_some_and(|i| i.assignees.contains(&x.id))]; (x.name()) }
                            } }
                        }
                    }
                }
                div class="form-actions" {
                    a class="btn" href=(current.map(|i| format!("{base}/{}", i.id())).unwrap_or_else(|| base.clone())) { "Cancel" }
                    button type="submit" class="btn primary" { (if current.is_some() { "Save changes" } else { T::ADD }) }
                }
            }))
        }
    };
    layout::app(req, &title, T::nav(id), &crumbs::<T>(p, Some((&title, base.clone()))), body)
}

/// The items as CSV.
pub fn csv<T: WorkPage>(req: &Req<'_>, id: Id<Project>) -> Option<String> {
    project_for(req, id)?;
    let state = req.engine.state();
    let day = |t: buildkiosk_core::Timestamp| t.date_at(state.utc_offset()).to_string();
    let mut out = format!("Number,Title,Location,{},Priority,Status,Manager,Assigned to,Due,Created,Closed\r\n", T::KIND_LABEL);
    for i in items_for::<T>(state, req.actor(), id) {
        let row = [
            i.number.to_string(),
            i.title.clone(),
            i.location.clone(),
            i.kind.clone(),
            i.priority.label().to_string(),
            i.status.label().to_string(),
            i.manager.map(|m| view::person_name(state, m)).unwrap_or_default(),
            view::people_names(state, &i.assignees),
            i.due.map(|d| d.to_string()).unwrap_or_default(),
            day(i.created.at),
            i.closed.map(|c| day(c.at)).unwrap_or_default(),
        ];
        out.push_str(&row.iter().map(|c| layout::csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    Some(out)
}

/// Observations by category and month over the last six months, to show where hazards cluster
/// and whether they are getting better. Months run from the first of the month; an observation
/// counts in the month it was observed (else the month it was added).
pub fn observation_trends(req: &Req<'_>, id: Id<Project>) -> Markup {
    let Some(p) = project_for(req, id) else { return not_found(req) };
    let state = req.engine.state();
    if view::level(state, req.actor(), id, Observation::TOOL) < Level::Read {
        return not_found(req);
    }
    let all = items_for::<Observation>(state, req.actor(), id);
    let today = req.today();
    let this_month = today.month_start();
    // The first day of each of the six months, oldest first.
    let mut months = vec![this_month];
    for _ in 0..5 {
        let first = months.first().copied().unwrap_or(this_month);
        match first.add_days(-1).map(|d| d.month_start()) {
            Some(prev) => months.insert(0, prev),
            None => break,
        }
    }
    let day_of = |o: &Observation| o.observed.unwrap_or_else(|| o.created.at.date_at(state.utc_offset()));
    let in_window: Vec<&&Observation> = all.iter().filter(|o| day_of(o) >= months.first().copied().unwrap_or(this_month)).collect();
    let mut categories: Vec<String> = in_window.iter().map(|o| if o.kind.is_empty() { "No category".to_string() } else { o.kind.clone() }).collect();
    categories.sort_by_key(|c| c.to_lowercase());
    categories.dedup_by_key(|c| c.to_lowercase());
    let count = |cat: &str, month: buildkiosk_core::Date| {
        in_window
            .iter()
            .filter(|o| day_of(o).month_start() == month)
            .filter(|o| (if o.kind.is_empty() { "No category" } else { o.kind.as_str() }).eq_ignore_ascii_case(cat))
            .count()
    };
    let month_total = |month: buildkiosk_core::Date| in_window.iter().filter(|o| day_of(o).month_start() == month).count();
    let open = all.iter().filter(|o| o.status != WorkStatus::Closed).count();
    let overdue = all.iter().filter(|o| o.is_overdue(today)).count();
    let base = format!("/projects/{id}/{}", Observation::PATH);
    let label = |d: buildkiosk_core::Date| format!("{} {}", d.pretty().split(' ').next().unwrap_or(""), d.ymd().0);
    let body = html! {
        div class="page-head" {
            div { h1 { "Observation trends" } div class="sub" { (p.name) " · last six months" } }
            div class="actions" { a class="btn" href=(base.clone()) { "Back to observations" } }
        }
        div class="stat-row" {
            div class="stat" { div class="label" { "Observations (6 months)" } div class="value" { (in_window.len()) } }
            div class="stat" { div class="label" { "Not closed" } div class="value" { (open) } }
            div class=(if overdue > 0 { "stat alert" } else { "stat" }) { div class="label" { "Overdue" } div class="value" { (overdue) } }
        }
        @if in_window.is_empty() {
            div class="card" { (empty(tool_icon(Observation::TOOL), "Nothing to show yet", "Trends appear once observations are recorded.")) }
        } @else {
            div class="table-wrap" {
                table class="data" {
                    thead { tr { th { "Category" } @for m in &months { th class="num" { (label(*m)) } } th class="num" { "Total" } } }
                    tbody {
                        @for c in &categories { tr {
                            td { a href=(format!("{base}?kind={}", if c == "No category" { String::new() } else { c.clone() })) { (c) } }
                            @for m in &months { td class="num" { (count(c, *m)) } }
                            td class="num" { strong { (months.iter().map(|m| count(c, *m)).sum::<usize>()) } }
                        } }
                        tr { th { "All categories" } @for m in &months { th class="num" { (month_total(*m)) } } th class="num" { (in_window.len()) } }
                    }
                }
            }
        }
    };
    layout::app(req, "Observation trends", Observation::nav(id), &crumbs::<Observation>(p, Some(("Trends", base))), body)
}
