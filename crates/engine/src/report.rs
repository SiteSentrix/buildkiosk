//! Cross-tool summaries: how many items each tool has and how many are open or late, and what is
//! waiting on a person. Pages and the `report.*` queries share these, and each only counts a tool
//! the actor may read (and, within it, items the actor may see).

use buildkiosk_core::{Date, Id};
use buildkiosk_model::finance::{ChangeEventStatus, CommitmentStatus};
use buildkiosk_model::form::FormStatus;
use buildkiosk_model::incident::Incident;
use buildkiosk_model::observation::Observation;
use buildkiosk_model::project::Project;
use buildkiosk_model::punch::PunchItem;
use buildkiosk_model::rfi::RfiStatus;
use buildkiosk_model::submittal::SubmittalStatus;
use buildkiosk_model::task::Task;
use buildkiosk_model::timesheet::TimeStatus;
use buildkiosk_model::work::WorkStatus;
use buildkiosk_model::{CompanyState, Level, Tool};
use serde::Serialize;

use crate::Actor;
use crate::view;
use crate::workitem::WorkItem;

/// One tool's line of a project summary.
#[derive(Clone, Debug, Serialize)]
pub struct SummaryRow {
    pub tool: &'static str,
    pub path: &'static str,
    pub total: usize,
    /// Still needing action (not closed, complete or approved).
    pub open: usize,
    /// Open and past its due date.
    pub overdue: usize,
}

fn row(tool: &'static str, path: &'static str, total: usize, open: usize, overdue: usize) -> SummaryRow {
    SummaryRow { tool, path, total, open, overdue }
}

fn readable(state: &CompanyState, actor: Actor, project: Id<Project>, tool: Tool) -> bool {
    view::level(state, actor, project, tool) >= Level::Read
}

fn work_row<T: WorkItem>(state: &CompanyState, project: Id<Project>, today: Date, label: &'static str) -> SummaryRow {
    let items: Vec<&T> = T::table(state).iter().filter(|i| i.project() == project && i.deleted.is_none()).collect();
    let open = items.iter().filter(|i| i.status != WorkStatus::Closed).count();
    let overdue = items.iter().filter(|i| i.status != WorkStatus::Closed && i.due.is_some_and(|d| d < today)).count();
    row(label, T::PATH, items.len(), open, overdue)
}

/// The project's tools with their counts, for the tools the actor may read.
pub fn project_summary(state: &CompanyState, actor: Actor, project: Id<Project>, today: Date) -> Vec<SummaryRow> {
    let mut out = Vec::new();
    if readable(state, actor, project, Tool::Rfis) {
        let l = view::rfis_for(state, actor, project);
        out.push(row(
            "RFIs",
            "rfis",
            l.len(),
            l.iter().filter(|r| r.status == RfiStatus::Open).count(),
            l.iter().filter(|r| r.is_overdue(today)).count(),
        ));
    }
    if readable(state, actor, project, Tool::Submittals) {
        let l = view::submittals_for(state, actor, project);
        out.push(row(
            "Submittals",
            "submittals",
            l.len(),
            l.iter().filter(|s| s.status == SubmittalStatus::InReview).count(),
            l.iter().filter(|s| s.is_overdue(today)).count(),
        ));
    }
    if readable(state, actor, project, Tool::Tasks) {
        out.push(work_row::<Task>(state, project, today, "Tasks"));
    }
    if readable(state, actor, project, Tool::PunchList) {
        out.push(work_row::<PunchItem>(state, project, today, "Punch list"));
    }
    if readable(state, actor, project, Tool::Observations) {
        out.push(work_row::<Observation>(state, project, today, "Observations"));
    }
    if readable(state, actor, project, Tool::Incidents) {
        out.push(work_row::<Incident>(state, project, today, "Incidents"));
    }
    if readable(state, actor, project, Tool::Inspections) {
        let l = view::inspections_for(state, actor, project);
        out.push(row(
            "Inspections",
            "inspections",
            l.len(),
            l.iter().filter(|i| i.status == buildkiosk_model::inspection::InspectionStatus::Open).count(),
            l.iter().filter(|i| i.is_overdue(today)).count(),
        ));
    }
    if readable(state, actor, project, Tool::Meetings) {
        let m: Vec<_> = state.meetings.iter().filter(|m| m.project == project && m.deleted.is_none()).collect();
        let open = m.iter().flat_map(|m| m.actions.iter()).filter(|a| a.is_open()).count();
        let overdue = m.iter().flat_map(|m| m.actions.iter()).filter(|a| a.is_open() && a.due.is_some_and(|d| d < today)).count();
        out.push(row("Meeting action items", "meetings", m.iter().map(|m| m.actions.len()).sum(), open, overdue));
    }
    if readable(state, actor, project, Tool::Transmittals) {
        let l = transmittal_counts(state, actor, project);
        out.push(row("Transmittals", "transmittals", l.0, l.1, 0));
    }
    if readable(state, actor, project, Tool::Correspondence) {
        let l = crate::cmd::correspondence::visible(state, actor, project);
        let open = l.iter().filter(|c| c.status == buildkiosk_model::correspondence::CorrespondenceStatus::Open).count();
        let overdue = l
            .iter()
            .filter(|c| {
                c.status == buildkiosk_model::correspondence::CorrespondenceStatus::Open
                    && c.replies.is_empty()
                    && c.reply_by.is_some_and(|d| d < today)
            })
            .count();
        out.push(row("Correspondence", "correspondence", l.len(), open, overdue));
    }
    if readable(state, actor, project, Tool::Forms) {
        let l: Vec<_> = state.form_records.iter().filter(|r| r.project == project && r.deleted.is_none()).collect();
        out.push(row("Forms", "forms", l.len(), l.iter().filter(|r| r.status == FormStatus::Draft).count(), 0));
    }
    if readable(state, actor, project, Tool::Schedule) {
        let rows = crate::sched::rows(state, project, today);
        let leaves: Vec<_> = rows.iter().filter(|r| !r.summary).collect();
        out.push(row(
            "Schedule tasks",
            "schedule",
            leaves.len(),
            leaves.iter().filter(|r| r.percent < 100).count(),
            leaves.iter().filter(|r| r.late).count(),
        ));
    }
    if readable(state, actor, project, Tool::Commitments) {
        let l = view::commitments_for(state, actor, project);
        out.push(row(
            "Commitments",
            "commitments",
            l.len(),
            l.iter().filter(|c| matches!(c.status, CommitmentStatus::Draft | CommitmentStatus::Approved)).count(),
            0,
        ));
    }
    if readable(state, actor, project, Tool::ChangeEvents) {
        let l: Vec<_> = state.change_events.iter().filter(|e| e.project == project && e.deleted.is_none()).collect();
        out.push(row("Change events", "change-events", l.len(), l.iter().filter(|e| e.status == ChangeEventStatus::Open).count(), 0));
    }
    if view::level(state, actor, project, Tool::Timesheets) == Level::Admin {
        let l: Vec<_> = state.time_entries.iter().filter(|e| e.project == project && e.deleted.is_none()).collect();
        out.push(row("Hours waiting for approval", "timesheets", l.len(), l.iter().filter(|e| e.status == TimeStatus::Submitted).count(), 0));
    }
    out
}

fn transmittal_counts(state: &CompanyState, actor: Actor, project: Id<Project>) -> (usize, usize) {
    let l = crate::cmd::transmittal::visible(state, actor, project);
    let drafts = l.iter().filter(|t| t.status == buildkiosk_model::transmittal::TransmittalStatus::Draft).count();
    (l.len(), drafts)
}

/// One thing waiting on a person.
#[derive(Clone, Debug, Serialize)]
pub struct Todo {
    pub project: Id<Project>,
    /// "RFI #4".
    pub label: String,
    pub title: String,
    /// A path inside the app.
    pub link: String,
    pub due: Option<Date>,
}

fn work_todos<T: WorkItem>(
    state: &CompanyState,
    project: Id<Project>,
    me: Id<buildkiosk_model::directory::Person>,
    label: impl Fn(&T) -> String,
    out: &mut Vec<Todo>,
) {
    for i in T::table(state)
        .iter()
        .filter(|i| i.project() == project && i.deleted.is_none() && i.status != WorkStatus::Closed && i.ball_in_court.contains(&me))
    {
        out.push(Todo {
            project,
            label: label(i),
            title: i.title.clone(),
            link: format!("/projects/{project}/{}/{}", T::PATH, i.id().raw()),
            due: i.due,
        });
    }
}

/// Items in `project` waiting on `me`: RFIs and submittals whose ball is in their court, tasks,
/// punch items, observations and incidents assigned to them, and meeting actions given to them.
pub fn waiting_on(state: &CompanyState, actor: Actor, project: Id<Project>, me: Id<buildkiosk_model::directory::Person>) -> Vec<Todo> {
    let mut out = Vec::new();
    if readable(state, actor, project, Tool::Rfis) {
        for r in view::rfis_for(state, actor, project).into_iter().filter(|r| r.status == RfiStatus::Open && r.ball_in_court.contains(&me)) {
            out.push(Todo { project, label: r.label(), title: r.subject.clone(), link: format!("/projects/{project}/rfis/{}", r.id), due: r.due });
        }
    }
    if readable(state, actor, project, Tool::Submittals) {
        for s in
            view::submittals_for(state, actor, project).into_iter().filter(|s| s.status == SubmittalStatus::InReview && s.ball_in_court.contains(&me))
        {
            out.push(Todo {
                project,
                label: s.label(),
                title: s.title.clone(),
                link: format!("/projects/{project}/submittals/{}", s.id),
                due: s.due,
            });
        }
    }
    if readable(state, actor, project, Tool::Tasks) {
        work_todos::<Task>(state, project, me, |i| i.label(), &mut out);
    }
    if readable(state, actor, project, Tool::PunchList) {
        work_todos::<PunchItem>(state, project, me, |i| i.label(), &mut out);
    }
    if readable(state, actor, project, Tool::Observations) {
        work_todos::<Observation>(state, project, me, |i| i.label(), &mut out);
    }
    if readable(state, actor, project, Tool::Incidents) {
        work_todos::<Incident>(state, project, me, |i| i.label(), &mut out);
    }
    if readable(state, actor, project, Tool::Meetings) {
        for m in state.meetings.iter().filter(|m| m.project == project && m.deleted.is_none()) {
            for a in m.actions.iter().filter(|a| a.is_open() && a.assignee == Some(me)) {
                out.push(Todo {
                    project,
                    label: format!("{} action", m.label()),
                    title: a.description.clone(),
                    link: format!("/projects/{project}/meetings/{}", m.id),
                    due: a.due,
                });
            }
        }
    }
    out.sort_by_key(|t| (t.due.is_none(), t.due));
    out
}

/// One project's line of the portfolio view.
#[derive(Clone, Debug, Serialize)]
pub struct PortfolioRow {
    pub project: Id<Project>,
    pub number: String,
    pub name: String,
    pub stage: String,
    /// Items needing action across the tools the actor reads.
    pub open: usize,
    pub overdue: usize,
    /// The revised budget and the projected over or under (negative is an overrun), when the
    /// actor reads the budget.
    pub revised: Option<buildkiosk_core::Money>,
    pub variance: Option<buildkiosk_core::Money>,
}

/// Every project the actor may open, with its open and past-due counts and budget position.
pub fn portfolio(state: &CompanyState, actor: Actor, today: Date) -> Vec<PortfolioRow> {
    use buildkiosk_model::Choice;
    view::projects_for(state, actor)
        .into_iter()
        .map(|p| {
            let rows = project_summary(state, actor, p.id, today);
            let (open, overdue) = rows.iter().fold((0, 0), |a, r| (a.0 + r.open, a.1 + r.overdue));
            let budget = if readable(state, actor, p.id, Tool::Budget) {
                view::budget_rows(state, p.id).ok().and_then(|r| view::budget_totals(&r).ok())
            } else {
                None
            };
            PortfolioRow {
                project: p.id,
                number: p.number.clone(),
                name: p.name.clone(),
                stage: p.stage.label().to_string(),
                open,
                overdue,
                revised: budget.map(|t| t.revised()),
                variance: budget.map(|t| t.variance()),
            }
        })
        .collect()
}

/// One cell of the company report: what a column holds for a project.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum Cell {
    Text(String),
    Count(usize),
    Money(buildkiosk_core::Money),
    Date(Date),
    /// The person can't see what this column is made of (a tool or budget they don't read), or the project has no value.
    Blank,
}

/// One project's line of a company report.
#[derive(Clone, Debug, Serialize)]
pub struct CompanyRow {
    pub project: Id<Project>,
    pub name: String,
    pub cells: Vec<Cell>,
}

fn count_of(rows: &[SummaryRow], tool: &str) -> Cell {
    rows.iter().find(|r| r.tool == tool).map_or(Cell::Blank, |r| Cell::Count(r.open))
}

/// The projects the actor may open, one line each with the chosen columns; the projects can be limited to a stage and
/// sorted by a column. What the actor can't see of a project is blank.
pub fn company_report(
    state: &CompanyState,
    actor: Actor,
    today: Date,
    columns: &[buildkiosk_model::saved_report::ReportColumn],
    stage: Option<buildkiosk_model::project::ProjectStage>,
    sort: Option<buildkiosk_model::saved_report::ReportColumn>,
    descending: bool,
) -> Vec<CompanyRow> {
    use buildkiosk_model::Choice;
    use buildkiosk_model::saved_report::ReportColumn as C;
    let mut rows: Vec<CompanyRow> = view::projects_for(state, actor)
        .into_iter()
        .filter(|p| stage.is_none_or(|s| p.stage == s))
        .map(|p| {
            let summary = project_summary(state, actor, p.id, today);
            let (open, overdue) = summary.iter().fold((0, 0), |a, r| (a.0 + r.open, a.1 + r.overdue));
            let budget = if readable(state, actor, p.id, Tool::Budget) {
                view::budget_rows(state, p.id).ok().and_then(|r| view::budget_totals(&r).ok())
            } else {
                None
            };
            let late = if readable(state, actor, p.id, Tool::Schedule) {
                Cell::Count(crate::sched::rows(state, p.id, today).iter().filter(|r| !r.summary && r.late).count())
            } else {
                Cell::Blank
            };
            // How old the open RFIs are, counted in days from the day each was made.
            let open_rfis: Option<Vec<i64>> = readable(state, actor, p.id, Tool::Rfis).then(|| {
                view::rfis_for(state, actor, p.id)
                    .into_iter()
                    .filter(|r| r.status == RfiStatus::Open)
                    .map(|r| r.created.at.date_at(state.utc_offset()).days_until(today).max(0))
                    .collect()
            });
            let rfis_overdue = readable(state, actor, p.id, Tool::Rfis).then(|| {
                view::rfis_for(state, actor, p.id).into_iter().filter(|r| r.status == RfiStatus::Open && r.due.is_some_and(|d| d < today)).count()
            });
            let money = |m: Option<buildkiosk_core::Money>| m.map_or(Cell::Blank, Cell::Money);
            let cells = columns
                .iter()
                .map(|c| match c {
                    C::Number => Cell::Text(p.number.clone()),
                    C::Stage => Cell::Text(p.stage.label().to_string()),
                    C::Start => p.start.map_or(Cell::Blank, Cell::Date),
                    C::Completion => p.completion.map_or(Cell::Blank, Cell::Date),
                    C::Open => Cell::Count(open),
                    C::Overdue => Cell::Count(overdue),
                    C::Rfis => count_of(&summary, "RFIs"),
                    C::RfisOverdue => rfis_overdue.map_or(Cell::Blank, Cell::Count),
                    C::RfiOldest => {
                        open_rfis.as_ref().and_then(|a| a.iter().max()).map_or(Cell::Blank, |n| Cell::Count(usize::try_from(*n).unwrap_or(0)))
                    }
                    C::RfiAge => open_rfis.as_ref().filter(|a| !a.is_empty()).map_or(Cell::Blank, |a| {
                        Cell::Count(usize::try_from(a.iter().sum::<i64>() / i64::try_from(a.len()).unwrap_or(1)).unwrap_or(0))
                    }),
                    C::Submittals => count_of(&summary, "Submittals"),
                    C::Punch => count_of(&summary, "Punch list"),
                    C::LateTasks => late.clone(),
                    C::Tasks => count_of(&summary, "Tasks"),
                    C::Observations => count_of(&summary, "Observations"),
                    C::Inspections => count_of(&summary, "Inspections"),
                    C::Incidents => count_of(&summary, "Incidents"),
                    C::MeetingActions => count_of(&summary, "Meeting action items"),
                    C::Correspondence => count_of(&summary, "Correspondence"),
                    C::Transmittals => count_of(&summary, "Transmittals"),
                    C::Forms => count_of(&summary, "Forms"),
                    C::ScheduleOpen => count_of(&summary, "Schedule tasks"),
                    C::Commitments => count_of(&summary, "Commitments"),
                    C::ChangeEvents => count_of(&summary, "Change events"),
                    C::HoursWaiting => count_of(&summary, "Hours waiting for approval"),
                    C::Original => money(budget.map(|t| t.original)),
                    C::Revised => money(budget.map(|t| t.revised())),
                    C::Committed => money(budget.map(|t| t.committed)),
                    C::Cost => money(budget.map(|t| t.cost())),
                    C::Projected => money(budget.map(|t| t.projected)),
                    C::Variance => money(budget.map(|t| t.variance())),
                })
                .collect();
            CompanyRow { project: p.id, name: p.name.clone(), cells }
        })
        .collect();
    let key = |r: &CompanyRow| -> (u8, i128, String) {
        let at = sort.and_then(|s| columns.iter().position(|c| *c == s));
        match at.and_then(|i| r.cells.get(i)) {
            Some(Cell::Count(n)) => (0, *n as i128, r.name.to_lowercase()),
            Some(Cell::Money(m)) => (0, i128::from(m.cents()), r.name.to_lowercase()),
            Some(Cell::Date(d)) => (0, i128::from(d.days_since_epoch()), r.name.to_lowercase()),
            Some(Cell::Text(t)) => (0, 0, format!("{}\u{0}{}", t.to_lowercase(), r.name.to_lowercase())),
            // Projects with nothing to show in the sort column go last either way.
            Some(Cell::Blank) => (1, 0, r.name.to_lowercase()),
            None => (0, 0, r.name.to_lowercase()),
        }
    };
    rows.sort_by_key(|r| key(r));
    if descending {
        // Reverse the order of what has a value, keeping the blanks at the end.
        let (mut with, without): (Vec<_>, Vec<_>) = rows.into_iter().partition(|r| key(r).0 == 0);
        with.reverse();
        with.extend(without);
        rows = with;
    }
    rows
}

/// The totals row: the sum of every column that adds up (blank where nothing was visible to add).
pub fn company_totals(rows: &[CompanyRow], columns: &[buildkiosk_model::saved_report::ReportColumn]) -> Vec<Cell> {
    columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            if !c.adds_up() {
                return Cell::Blank;
            }
            let mut count: Option<usize> = None;
            let mut money: Option<buildkiosk_core::Money> = None;
            for r in rows {
                match r.cells.get(i) {
                    Some(Cell::Count(n)) => count = Some(count.unwrap_or(0).saturating_add(*n)),
                    Some(Cell::Money(m)) => money = Some(money.unwrap_or(buildkiosk_core::Money::ZERO).checked_add(*m).unwrap_or(*m)),
                    _ => {}
                }
            }
            match (count, money) {
                (Some(n), _) => Cell::Count(n),
                (_, Some(m)) => Cell::Money(m),
                _ => Cell::Blank,
            }
        })
        .collect()
}
