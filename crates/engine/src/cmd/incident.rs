//! Incidents: injuries, near misses and damage. The workflow is shared with the punch list and
//! observations (see [`crate::workitem`]); an incident adds when it happened, how serious it was,
//! whether it is recordable, who was involved, the root cause and the corrective action.
//!
//! Incidents are sensitive: the Incidents tool is hidden from the subcontractor and owner
//! templates, and only people with access to the tool see them. An incident can also be marked
//! private, and is then seen only by the people it concerns. A serious report tells the tool's
//! admins at once, and the injury log adds up the year's cases into the usual incident rates.

use buildkiosk_core::{Date, Id, text};
use buildkiosk_model::directory::Person;
use buildkiosk_model::incident::{Incident, Severity};
use buildkiosk_model::project::{Project, YearHours};
use buildkiosk_model::timesheet::TimeStatus;
use buildkiosk_model::work::{WorkCore, WorkStatus};
use buildkiosk_model::{Choice, CompanyState, Level, Priority, Table, Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::ctx::Read;
use crate::workitem::WorkItem;
use crate::{Actor, Ctx, EngineError, Params, Result, View, view};

pub const COMMANDS: &[CommandSpec] = crate::work_commands!(
    "incident",
    "incident",
    Tool::Incidents,
    Incident,
    "; occurred?: YYYY-MM-DD (default: today); severity?: near_miss|property_damage|first_aid|medical_treatment|lost_time|fatality; recordable?: bool; people_involved?: text; root_cause?: text; corrective_action?: text; \
     private?: bool (only the author, manager, assignees, the people in `involved` and the tool's admins see it); involved?: [person ids]"
);

/// The injury log and the hours behind the rates.
pub const LOG: &[CommandSpec] = &[
    CommandSpec::query(
        "incident.log",
        "Injury log and incident rates",
        Tool::Incidents,
        Access::Project(Level::Read),
        "project: project id; year?: calendar year (default: this year). Counts by severity, the recordable cases, the hours worked (approved timesheets plus hours set for the year) and the rates per 200,000 hours; only incidents you can see",
        log,
    ),
    CommandSpec::query(
        "incident.company_log",
        "Injury log across projects",
        Tool::Incidents,
        Access::Member,
        "year?: calendar year (default: this year). One line for every project whose incidents you can read (counts, hours, rates per 200,000 hours) and the company's totals; only incidents you can see",
        company_log,
    ),
    CommandSpec::mutate(
        "incident.set_hours",
        "Set other hours worked",
        Tool::Incidents,
        Access::Project(Level::Admin),
        "project: project id; year: calendar year; hours: whole hours worked that year by people who are not on timesheets, such as subcontractors (0 removes them)",
        set_hours,
    ),
];

fn next_number(state: &CompanyState, project: Id<Project>) -> u32 {
    state.incidents.iter().filter(|i| i.project == project).map(|i| i.number).max().unwrap_or(0).saturating_add(1)
}

impl WorkItem for Incident {
    const TOOL: Tool = Tool::Incidents;
    const NOUN: &'static str = "incident";
    const LABEL: &'static str = "Incident";
    const PATH: &'static str = "incidents";
    const ATTACH_CMD: &'static str = "incident.attach";
    const TABLE: &'static str = "incidents";

    fn table(state: &CompanyState) -> &Table<Self> {
        &state.incidents
    }

    fn table_mut(state: &mut CompanyState) -> &mut Table<Self> {
        &mut state.incidents
    }

    fn blank(ctx: &Ctx<'_>, project: Id<Project>) -> Self {
        let stamp = ctx.stamp();
        Incident {
            id: ctx.state.incidents.peek_next(),
            project,
            core: WorkCore {
                number: next_number(ctx.state, project),
                title: String::new(),
                description: String::new(),
                location: String::new(),
                drawing_number: String::new(),
                kind: String::new(),
                priority: Priority::default(),
                status: WorkStatus::Open,
                manager: ctx.person(),
                assignees: Vec::new(),
                ball_in_court: Vec::new(),
                due: None,
                work_done: None,
                closed: None,
                attachments: Vec::new(),
                comments: Vec::new(),
                created: stamp,
                updated: stamp,
                deleted: None,
            },
            occurred: Some(ctx.today()),
            severity: Severity::default(),
            recordable: false,
            people_involved: String::new(),
            root_cause: String::new(),
            corrective_action: String::new(),
            private: false,
            involved: Vec::new(),
        }
    }

    fn set_id(&mut self, id: Id<Self>) {
        self.id = id;
    }

    fn apply_extra(&mut self, p: &Params<'_>) -> Result<()> {
        if p.present("occurred") {
            self.occurred = p.date("occurred")?;
        }
        if let Some(s) = p.choice::<Severity>("severity")? {
            self.severity = s;
        }
        if let Some(r) = p.flag("recordable")? {
            self.recordable = r;
        }
        for (key, field) in
            [("people_involved", &mut self.people_involved), ("root_cause", &mut self.root_cause), ("corrective_action", &mut self.corrective_action)]
        {
            if p.present(key) {
                *field = p.text_or_empty(key, text::LONG)?;
            }
        }
        if let Some(private) = p.flag("private")? {
            self.private = private;
        }
        if p.present("involved") {
            self.involved = p.ids("involved")?;
        }
        Ok(())
    }

    fn validate(&self, ctx: &Ctx<'_>) -> Result<()> {
        ctx.check_people(&self.involved)
    }

    /// A private incident is for the people it concerns and the tool's admins.
    fn visible_to(&self, _state: &CompanyState, actor: Actor) -> bool {
        if !self.private {
            return true;
        }
        match actor.person() {
            Some(me) => self.created.by == Some(me) || self.manager == Some(me) || self.assignees.contains(&me) || self.involved.contains(&me),
            None => matches!(actor, Actor::System),
        }
    }

    /// A serious report (medical treatment or worse, or a recordable case) tells the tool's admins on the
    /// project, once: when it is first reported that way.
    fn after_save(&self, before: Option<&Self>, ctx: &mut Ctx<'_>) -> Result<()> {
        let serious = |i: &Incident| i.severity.is_serious() || i.recordable;
        if !serious(self) || before.is_some_and(serious) {
            return Ok(());
        }
        let admins: Vec<Id<Person>> = ctx
            .state
            .members
            .iter()
            .filter(|m| m.project == self.project && m.removed.is_none())
            .map(|m| m.person)
            .filter(|p| perm::project_level(ctx.state, *p, self.project, Tool::Incidents) == Level::Admin)
            .collect();
        crate::cmd::notification::notify(
            ctx,
            &admins,
            self.project,
            &format!("Serious incident reported ({}): {}", self.severity.label(), self.title),
            &format!("/projects/{}/incidents/{}", self.project, self.id),
        );
        Ok(())
    }
}

/// Cases per 200,000 hours worked (about 100 people for a year), to two decimals; none without hours.
pub fn rate(cases: u64, hours_tenths: u64) -> Option<String> {
    if hours_tenths == 0 {
        return None;
    }
    let hundredths = u128::from(cases) * 200_000 * 10 * 100 / u128::from(hours_tenths);
    Some(format!("{}.{:02}", hundredths / 100, hundredths % 100))
}

/// What the actor can see of a project's incidents that happened in `year`, oldest first.
pub fn incidents_in_year(state: &CompanyState, actor: Actor, project: Id<Project>, year: i32) -> Vec<&Incident> {
    let mut list: Vec<&Incident> =
        crate::workitem::items_for::<Incident>(state, actor, project).into_iter().filter(|i| i.occurred.is_some_and(|d| d.ymd().0 == year)).collect();
    list.sort_by_key(|i| (i.occurred, i.number));
    list
}

/// Hours worked on a project in a year: approved timesheets, and the other hours set for the year.
/// Tenths of an hour.
pub fn hours_worked(state: &CompanyState, project: Id<Project>, year: i32) -> (u64, u64) {
    let timesheets: u64 = state
        .time_entries
        .iter()
        .filter(|e| e.project == project && e.deleted.is_none() && e.status == TimeStatus::Approved && e.date.ymd().0 == year)
        .map(|e| u64::from(e.tenths))
        .sum();
    let other = state.projects.get(project).and_then(|p| p.incident_hours.iter().find(|h| h.year == year)).map_or(0, |h| u64::from(h.hours) * 10);
    (timesheets, other)
}

/// One project's line of the company injury log.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProjectLog {
    pub project: Id<Project>,
    pub name: String,
    pub incidents: u64,
    pub recordable: u64,
    pub lost_time: u64,
    pub fatalities: u64,
    /// Tenths of an hour.
    pub hours: u64,
}

/// The injury log of every project the actor can read incidents on, in name order, and the company's total line.
pub fn company_injury_log(state: &CompanyState, actor: Actor, year: i32) -> (Vec<ProjectLog>, ProjectLog) {
    let mut lines: Vec<ProjectLog> = view::projects_for(state, actor)
        .into_iter()
        .filter(|p| view::level(state, actor, p.id, Tool::Incidents) >= Level::Read)
        .map(|p| {
            let list = incidents_in_year(state, actor, p.id, year);
            let (sheets, other) = hours_worked(state, p.id, year);
            ProjectLog {
                project: p.id,
                name: p.name.clone(),
                incidents: list.len() as u64,
                recordable: list.iter().filter(|i| i.recordable).count() as u64,
                lost_time: list.iter().filter(|i| i.recordable && i.severity.is_lost_time()).count() as u64,
                fatalities: list.iter().filter(|i| i.severity == Severity::Fatality).count() as u64,
                hours: sheets.saturating_add(other),
            }
        })
        .collect();
    lines.sort_by_key(|l| l.name.to_lowercase());
    let total = lines.iter().fold(
        ProjectLog { project: Id::new(0), name: "All projects".into(), incidents: 0, recordable: 0, lost_time: 0, fatalities: 0, hours: 0 },
        |t, l| ProjectLog {
            incidents: t.incidents.saturating_add(l.incidents),
            recordable: t.recordable.saturating_add(l.recordable),
            lost_time: t.lost_time.saturating_add(l.lost_time),
            fatalities: t.fatalities.saturating_add(l.fatalities),
            hours: t.hours.saturating_add(l.hours),
            ..t
        },
    );
    (lines, total)
}

fn company_log(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let year = year_param(p, v.today())?;
    let (lines, total) = company_injury_log(v.state, v.actor, year);
    let with_rates = |l: &ProjectLog| {
        json!({
            "project": l.project, "name": l.name, "incidents": l.incidents, "recordable": l.recordable, "lost_time": l.lost_time,
            "fatalities": l.fatalities, "hours": l.hours as f64 / 10.0,
            "recordable_rate": rate(l.recordable, l.hours), "lost_time_rate": rate(l.lost_time, l.hours),
        })
    };
    view::json(&json!({ "year": year, "projects": lines.iter().map(with_rates).collect::<Vec<_>>(), "total": with_rates(&total) }))
}

fn year_param(p: &Params<'_>, today: Date) -> Result<i32> {
    let year = p.int("year", i64::from(Date::MIN_YEAR)..=i64::from(Date::MAX_YEAR))?;
    Ok(year.and_then(|y| i32::try_from(y).ok()).unwrap_or_else(|| today.ymd().0))
}

fn log(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let year = year_param(p, v.today())?;
    let list = incidents_in_year(v.state, v.actor, project, year);
    let count = |f: &dyn Fn(&Incident) -> bool| list.iter().filter(|i| f(i)).count() as u64;
    let by_severity: Vec<Value> =
        Severity::ALL.iter().map(|s| json!({ "severity": s, "label": s.label(), "count": count(&|i| i.severity == *s) })).collect();
    let recordable = count(&|i| i.recordable);
    let lost_time = count(&|i| i.recordable && i.severity.is_lost_time());
    let (timesheets, other) = hours_worked(v.state, project, year);
    let tenths = timesheets.saturating_add(other);
    let rows: Vec<Value> = list
        .iter()
        .filter(|i| i.recordable)
        .map(|i| {
            json!({
                "id": i.id, "number": i.number, "title": i.title, "occurred": i.occurred, "severity": i.severity, "location": i.location,
                "people_involved": i.people_involved, "status": i.status,
            })
        })
        .collect();
    view::json(&json!({
        "year": year,
        "incidents": list.len(),
        "by_severity": by_severity,
        "recordable": recordable,
        "lost_time": lost_time,
        "fatalities": count(&|i| i.severity == Severity::Fatality),
        "hours": { "timesheets": timesheets as f64 / 10.0, "other": other as f64 / 10.0, "total": tenths as f64 / 10.0 },
        "recordable_rate": rate(recordable, tenths),
        "lost_time_rate": rate(lost_time, tenths),
        "recordable_cases": rows,
    }))
}

fn set_hours(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let year = p
        .int("year", i64::from(Date::MIN_YEAR)..=i64::from(Date::MAX_YEAR))?
        .and_then(|y| i32::try_from(y).ok())
        .ok_or_else(|| p.bad("year", "is required"))?;
    let hours = p.int("hours", 0..=1_000_000_000)?.and_then(|h| u32::try_from(h).ok()).ok_or_else(|| p.bad("hours", "is required"))?;
    if !ctx.state.projects.get(project).is_some_and(|x| x.deleted.is_none()) {
        return Err(EngineError::not_found(format!("project {project} not found")));
    }
    ctx.state.projects.update(project, |x| {
        x.incident_hours.retain(|h| h.year != year);
        if hours > 0 {
            x.incident_hours.push(YearHours { year, hours });
            x.incident_hours.sort_by_key(|h| h.year);
        }
    });
    Ok(json!({ "year": year, "hours": hours }))
}
