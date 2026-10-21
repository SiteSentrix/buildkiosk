//! Reports: summaries across a project's tools. Read-only; each tool's figures are only what the
//! caller may see in that tool.

use buildkiosk_core::{Id, text};
use buildkiosk_model::project::{Project, ProjectStage};
use buildkiosk_model::saved_report::{DeliveryRhythm, MAX_RECIPIENTS, ReportColumn, ReportDelivery, SavedReport};
use buildkiosk_model::{Choice, Level, Tool, perm};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::report;
use crate::view::json;
use crate::{Actor, Ctx, EngineError, Params, Read, Result, View};

pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec::query(
        "report.summary",
        "Project summary",
        Tool::Reports,
        Access::Project(Level::Read),
        "project: project id (every tool you can read, with its total, open and overdue counts)",
        summary,
    ),
    CommandSpec::query(
        "report.portfolio",
        "Portfolio report",
        Tool::CompanyReports,
        Access::Member,
        "(no parameters) every project you can open with its open and past-due counts and, where you can read the budget, the revised budget and projected over or under",
        portfolio,
    ),
    CommandSpec::mutate(
        "company_report.save",
        "Save company report",
        Tool::CompanyReports,
        Access::Member,
        "id?: report id (changes your own; without it a new one is made); name: text; columns: list or comma text of number, stage, start, completion, open, overdue, rfis, rfis_overdue, rfi_oldest, rfi_age, submittals, punch, late_tasks, revised, committed, cost, projected, variance; stage?: only projects in this stage; sort?: a column to sort by; descending?: bool; shared?: bool (everyone in the company can run it)",
        save,
    ),
    CommandSpec::mutate(
        "company_report.delete",
        "Delete company report",
        Tool::CompanyReports,
        Access::Member,
        "id: report id (its owner, or a company admin)",
        delete,
    ),
    CommandSpec::mutate(
        "company_report.schedule",
        "Email a report on a schedule",
        Tool::CompanyReports,
        Access::Member,
        "id: report id (its owner, or a company admin); rhythm: off|daily|weekly|monthly (off stops it); weekday?: mon to sun (weekly; default monday); day?: 1-28 (monthly; default 1); people?: [person ids] who get it, up to 25 (each person gets it as they themselves would see it, so what they can't see of a project is blank; they must be active users with an email address, and on a report that isn't shared only its owner and company admins can get it). The first email is the next time the rhythm comes round; emails go out from about six in the morning",
        schedule,
    ),
    CommandSpec::query(
        "company_report.due",
        "Reports due to be emailed",
        Tool::CompanyReports,
        Access::System,
        "(none) (the saved reports whose delivery has come due, with the people to send each to; the server sends them and then records it with company_report.mark_sent)",
        due,
    ),
    CommandSpec::mutate(
        "company_report.mark_sent",
        "Record that a report was emailed",
        Tool::CompanyReports,
        Access::System,
        "id: report id (counts today's delivery as made)",
        mark_sent,
    ),
    CommandSpec::query(
        "company_report.list",
        "List company reports",
        Tool::CompanyReports,
        Access::Member,
        "(none) (yours and the shared ones)",
        list,
    ),
    CommandSpec::query(
        "company_report.run",
        "Run a company report",
        Tool::CompanyReports,
        Access::Member,
        "id?: a saved report; or columns?, stage?, sort?, descending? to run one without saving. Every project you can open, with the chosen columns and a totals row; what you can't see of a project is blank",
        run,
    ),
    CommandSpec::query(
        "report.waiting_on_me",
        "Waiting on me",
        Tool::Reports,
        Access::Project(Level::Read),
        "project: project id (RFIs, submittals, tasks, punch items, observations, incidents and meeting actions waiting on you, soonest due first)",
        waiting,
    ),
];

fn summary(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    json(&report::project_summary(v.state, v.actor, project, v.today()))
}

fn waiting(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let project: Id<Project> = p.id("project")?;
    let Some(me) = v.person() else { return json(&Vec::<report::Todo>::new()) };
    json(&report::waiting_on(v.state, v.actor, project, me))
}

fn portfolio(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    json(&report::portfolio(v.state, v.actor, v.today()))
}

const MAX_REPORTS: usize = 200;

fn columns_of(p: &Params<'_>) -> Result<Vec<ReportColumn>> {
    let mut out: Vec<ReportColumn> = Vec::new();
    for w in p.words("columns", 40)? {
        let c = ReportColumn::parse(&w).ok_or_else(|| p.bad("columns", format!("`{w}` is not a report column")))?;
        if !out.contains(&c) {
            out.push(c);
        }
    }
    // In the table's own order, whatever order they were named in.
    Ok(ReportColumn::ALL.iter().copied().filter(|c| out.contains(c)).collect())
}

fn sees(state: &buildkiosk_model::CompanyState, actor: Actor, r: &SavedReport) -> bool {
    r.deleted.is_none()
        && (r.shared
            || match actor {
                Actor::System => true,
                Actor::Person(p) => p == r.owner || perm::company_level(state, p, Tool::CompanyAdmin) == Level::Admin,
            })
}

fn save(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("reports are saved by a person"))?;
    let name = p.req_line("name", text::SHORT)?;
    let columns = columns_of(p)?;
    if columns.is_empty() {
        return Err(p.bad("columns", "needs at least one column"));
    }
    let stage = p.choice::<ProjectStage>("stage")?;
    let sort = p.choice::<ReportColumn>("sort")?;
    if sort.is_some_and(|s| !columns.contains(&s)) {
        return Err(p.bad("sort", "must be one of the columns"));
    }
    let descending = p.flag_or("descending", false)?;
    let shared = p.flag_or("shared", false)?;
    let stamp = ctx.stamp();
    let existing = p.opt_id::<SavedReport>("id")?;
    let admin = perm::company_level(ctx.state, me, Tool::CompanyAdmin) == Level::Admin;
    let mine = |r: &&SavedReport| r.deleted.is_none() && r.owner == me;
    if ctx.state.saved_reports.iter().filter(mine).any(|r| Some(r.id) != existing && r.name.eq_ignore_ascii_case(&name)) {
        return Err(p.bad("name", "is already one of your reports"));
    }
    let id = match existing {
        Some(id) => {
            let r = ctx
                .state
                .saved_reports
                .get(id)
                .filter(|r| r.deleted.is_none())
                .ok_or_else(|| EngineError::not_found(format!("report {id} not found")))?;
            if r.owner != me && !admin {
                return Err(EngineError::forbidden("only its owner or a company admin can change a report"));
            }
            ctx.state.saved_reports.update(id, |r| {
                (r.name, r.columns, r.stage, r.sort, r.descending, r.shared, r.updated) = (name, columns, stage, sort, descending, shared, stamp)
            });
            id
        }
        None => {
            if ctx.state.saved_reports.iter().filter(|r| r.deleted.is_none()).count() >= MAX_REPORTS {
                return Err(EngineError::conflict(format!("the company keeps at most {MAX_REPORTS} saved reports")));
            }
            let id = ctx.state.saved_reports.alloc();
            ctx.state.saved_reports.put(SavedReport {
                id,
                name,
                columns,
                stage,
                sort,
                descending,
                shared,
                owner: me,
                delivery: None,
                created: stamp,
                updated: stamp,
                deleted: None,
            });
            id
        }
    };
    Ok(json!({ "id": id }))
}

fn delete(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("reports are deleted by a person"))?;
    let id: Id<SavedReport> = p.id("id")?;
    let r = ctx
        .state
        .saved_reports
        .get(id)
        .filter(|r| r.deleted.is_none() && sees(ctx.state, ctx.actor, r))
        .ok_or_else(|| EngineError::not_found(format!("report {id} not found")))?;
    if r.owner != me && perm::company_level(ctx.state, me, Tool::CompanyAdmin) != Level::Admin {
        return Err(EngineError::forbidden("only its owner or a company admin can delete a report"));
    }
    let stamp = ctx.stamp();
    ctx.state.saved_reports.update(id, |r| r.deleted = Some(stamp));
    Ok(json!({ "id": id }))
}

/// A weekday from a name (`mon`, `Monday`) or a number 0 (Monday) to 6.
fn weekday_of(p: &Params<'_>, raw: &str) -> Result<u8> {
    const NAMES: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];
    let raw = raw.trim().to_ascii_lowercase();
    if raw.is_empty() {
        return Ok(0);
    }
    if raw.len() >= 3
        && let Some(i) = NAMES.iter().position(|n| n.starts_with(&raw))
    {
        return u8::try_from(i).map_err(|_| p.bad("weekday", "is not a weekday"));
    }
    Err(p.bad("weekday", "must be a weekday such as monday"))
}

fn schedule(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let me = ctx.person().ok_or_else(|| EngineError::forbidden("reports are scheduled by a person"))?;
    let id: Id<SavedReport> = p.id("id")?;
    let r = ctx
        .state
        .saved_reports
        .get(id)
        .filter(|r| r.deleted.is_none() && sees(ctx.state, ctx.actor, r))
        .ok_or_else(|| EngineError::not_found(format!("report {id} not found")))?
        .clone();
    let admin = perm::company_level(ctx.state, me, Tool::CompanyAdmin) == Level::Admin;
    if r.owner != me && !admin {
        return Err(EngineError::forbidden("only its owner or a company admin can email a report"));
    }
    let rhythm_text = p.req_line("rhythm", 20)?;
    let delivery = if rhythm_text.eq_ignore_ascii_case("off") || rhythm_text.eq_ignore_ascii_case("none") {
        None
    } else {
        let rhythm = DeliveryRhythm::parse(&rhythm_text).ok_or_else(|| p.bad("rhythm", "must be off, daily, weekly or monthly"))?;
        let weekday = weekday_of(p, &p.line_or_empty("weekday", 20)?)?;
        let day = p.int("day", 1..=28)?.unwrap_or(1);
        let mut people = p.ids::<buildkiosk_model::directory::Person>("people")?;
        people.sort();
        people.dedup();
        if people.is_empty() {
            return Err(p.bad("people", "name at least one person to send it to"));
        }
        if people.len() > MAX_RECIPIENTS {
            return Err(p.bad("people", format!("a report goes to at most {MAX_RECIPIENTS} people")));
        }
        for person in &people {
            let ok = ctx.state.people.get(*person).is_some_and(|x| x.is_user && x.active && x.email.is_some());
            if !ok {
                return Err(p.bad("people", format!("person {person} is not an active user with an email address")));
            }
            let may_see = r.shared || *person == r.owner || perm::company_level(ctx.state, *person, Tool::CompanyAdmin) == Level::Admin;
            if !may_see {
                return Err(p.bad("people", format!("person {person} can't run this report: share it first")));
            }
        }
        Some(ReportDelivery { rhythm, weekday, day: u8::try_from(day).unwrap_or(1), recipients: people, last_sent: Some(ctx.today()) })
    };
    let stamp = ctx.stamp();
    ctx.state.saved_reports.update(id, |x| {
        x.delivery = delivery;
        x.updated = stamp;
    });
    Ok(json!({ "id": id }))
}

/// Six in the morning, company time: reports go out after it.
const FIRST_HOUR: i64 = 6;

fn due(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    const MS_PER_DAY: i64 = 86_400_000;
    let local_ms = v.at().millis().saturating_add(i64::from(v.state.utc_offset()).saturating_mul(60_000));
    if local_ms.rem_euclid(MS_PER_DAY) / 3_600_000 < FIRST_HOUR {
        return Ok(json!([]));
    }
    let today = v.today();
    let mut out = Vec::new();
    for r in v.state.saved_reports.iter().filter(|r| r.deleted.is_none()) {
        let Some(d) = r.delivery.as_ref().filter(|d| d.due(today)) else { continue };
        let recipients: Vec<Value> = d
            .recipients
            .iter()
            .filter_map(|id| v.state.people.get(*id))
            .filter(|x| x.is_user && x.active)
            .filter_map(|x| Some(json!({ "person": x.id, "email": x.email.clone()? })))
            .collect();
        out.push(json!({ "id": r.id, "name": r.name, "recipients": recipients }));
    }
    Ok(Value::Array(out))
}

fn mark_sent(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let id: Id<SavedReport> = p.id("id")?;
    let today = ctx.today();
    let r = ctx.state.saved_reports.get(id).filter(|r| r.deleted.is_none() && r.delivery.is_some());
    if r.is_none() {
        return Err(EngineError::not_found(format!("report {id} is not emailed")));
    }
    ctx.state.saved_reports.update(id, |x| {
        if let Some(d) = x.delivery.as_mut() {
            d.last_sent = Some(today);
        }
    });
    Ok(json!({ "id": id }))
}

/// The saved reports the actor may run, by name.
pub fn visible_reports(state: &buildkiosk_model::CompanyState, actor: Actor) -> Vec<&SavedReport> {
    let mut v: Vec<&SavedReport> = state.saved_reports.iter().filter(|r| sees(state, actor, r)).collect();
    v.sort_by_key(|r| r.name.to_lowercase());
    v
}

fn list(v: &View<'_>, _p: &Params<'_>) -> Result<Value> {
    json(&visible_reports(v.state, v.actor))
}

fn run(v: &View<'_>, p: &Params<'_>) -> Result<Value> {
    let (columns, stage, sort, descending, name) = match p.opt_id::<SavedReport>("id")? {
        Some(id) => {
            let r = v
                .state
                .saved_reports
                .get(id)
                .filter(|r| sees(v.state, v.actor, r))
                .ok_or_else(|| EngineError::not_found(format!("report {id} not found")))?;
            (r.columns.clone(), r.stage, r.sort, r.descending, Some(r.name.clone()))
        }
        None => {
            let columns = if p.present("columns") { columns_of(p)? } else { ReportColumn::STANDARD.to_vec() };
            if columns.is_empty() {
                return Err(p.bad("columns", "needs at least one column"));
            }
            (columns, p.choice::<ProjectStage>("stage")?, p.choice::<ReportColumn>("sort")?, p.flag_or("descending", false)?, None)
        }
    };
    let rows = report::company_report(v.state, v.actor, v.today(), &columns, stage, sort, descending);
    let totals = report::company_totals(&rows, &columns);
    json(&json!({
        "name": name,
        "columns": columns.iter().map(|c| json!({ "key": c.key(), "label": c.label() })).collect::<Vec<_>>(),
        "rows": rows,
        "totals": totals,
    }))
}
