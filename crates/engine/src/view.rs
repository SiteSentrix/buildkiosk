//! Read helpers shared by queries and the UI.
//!
//! The UI reads the state directly (it's thin: it shows engine state and acts through commands);
//! these helpers keep the rules for what a person may see in one place, so a page and the query
//! behind the API answer the same.

use buildkiosk_core::{Id, Money};
use buildkiosk_model::daily_log::{DailyLog, LogStatus};
use buildkiosk_model::directory::{Person, Vendor};
use buildkiosk_model::finance::{Commitment, CommitmentStatus};
use buildkiosk_model::inspection::Inspection;
use buildkiosk_model::project::{CostCode, Project};
use buildkiosk_model::rfi::{Rfi, RfiStatus};
use buildkiosk_model::submittal::{Submittal, SubmittalStatus};
use buildkiosk_model::{CompanyState, Level, Tool, perm};
use serde::Serialize;
use serde_json::Value;

use crate::{Actor, EngineError};

/// Serializes a query result.
pub fn json<T: Serialize>(value: &T) -> Result<Value, EngineError> {
    serde_json::to_value(value).map_err(|e| EngineError::Internal { cmd: "view".into(), msg: e.to_string() })
}

pub fn person_name(state: &CompanyState, id: Id<Person>) -> String {
    state.people.get(id).map(Person::name).unwrap_or_else(|| format!("Unknown person {id}"))
}

pub fn people_names(state: &CompanyState, ids: &[Id<Person>]) -> String {
    ids.iter().map(|id| person_name(state, *id)).collect::<Vec<_>>().join(", ")
}

pub fn vendor_name(state: &CompanyState, id: Id<Vendor>) -> String {
    state.vendors.get(id).map(|v| v.name.clone()).unwrap_or_else(|| format!("Unknown company {id}"))
}

/// The company a person works for ("our" company when `vendor` is empty).
pub fn employer_name(state: &CompanyState, person: &Person) -> String {
    match person.vendor {
        Some(v) => vendor_name(state, v),
        None => state.company().map(|c| c.name.clone()).unwrap_or_default(),
    }
}

/// The actor's level in a project tool.
pub fn level(state: &CompanyState, actor: Actor, project: Id<Project>, tool: Tool) -> Level {
    match actor {
        Actor::System => Level::Admin,
        Actor::Person(p) => perm::project_level(state, p, project, tool),
    }
}

/// Projects the actor may open, newest first.
pub fn projects_for(state: &CompanyState, actor: Actor) -> Vec<&Project> {
    let mut list: Vec<&Project> = match actor {
        Actor::System => state.projects.iter().filter(|p| p.deleted.is_none()).collect(),
        Actor::Person(person) => perm::visible_projects(state, person).into_iter().filter_map(|id| state.projects.get(id)).collect(),
    };
    list.reverse();
    list
}

/// Whether the actor may see an RFI: Read access, and for drafts and private RFIs, being part of it
/// (or an RFI admin).
pub fn can_see_rfi(state: &CompanyState, actor: Actor, rfi: &Rfi) -> bool {
    let lvl = level(state, actor, rfi.project, Tool::Rfis);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || rfi.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match rfi.status {
        RfiStatus::Draft => rfi.created.by == Some(person) || rfi.manager == Some(person),
        _ => !rfi.private || rfi.involves(person),
    }
}

/// The RFIs of a project the actor may see, in number order.
pub fn rfis_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Rfi> {
    let mut list: Vec<&Rfi> = state.rfis.iter().filter(|r| r.project == project && can_see_rfi(state, actor, r)).collect();
    list.sort_by_key(|r| r.number);
    list
}

/// Whether the actor may see a submittal: Read access, and for drafts and private submittals,
/// being part of it (or a submittals admin).
pub fn can_see_submittal(state: &CompanyState, actor: Actor, s: &Submittal) -> bool {
    let lvl = level(state, actor, s.project, Tool::Submittals);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || s.deleted.is_some() {
        return false;
    }
    let Some(person) = actor.person() else { return false };
    match s.status {
        SubmittalStatus::Draft => s.created.by == Some(person) || s.manager == Some(person),
        _ => !s.private || s.involves(person),
    }
}

/// The submittals of a project the actor may see, in number order.
pub fn submittals_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Submittal> {
    let mut list: Vec<&Submittal> = state.submittals.iter().filter(|s| s.project == project && can_see_submittal(state, actor, s)).collect();
    list.sort_by_key(|s| s.number);
    list
}

/// Whether the actor may see a daily log: Read access, and for drafts, being the author (or a
/// daily log admin).
pub fn can_see_daily_log(state: &CompanyState, actor: Actor, log: &DailyLog) -> bool {
    let lvl = level(state, actor, log.project, Tool::DailyLog);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || log.deleted.is_some() {
        return false;
    }
    log.status == LogStatus::Submitted || (actor.person().is_some() && log.created.by == actor.person())
}

/// The daily logs of a project the actor may see, newest day first.
pub fn daily_logs_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&DailyLog> {
    let mut list: Vec<&DailyLog> = state.daily_logs.iter().filter(|l| l.project == project && can_see_daily_log(state, actor, l)).collect();
    list.sort_by_key(|l| std::cmp::Reverse(l.date));
    list
}

/// Whether the actor may see an inspection: Read access to the tool.
pub fn can_see_inspection(state: &CompanyState, actor: Actor, i: &Inspection) -> bool {
    let lvl = level(state, actor, i.project, Tool::Inspections);
    lvl == Level::Admin || (i.deleted.is_none() && lvl >= Level::Read)
}

/// The inspections of a project the actor may see, in number order.
pub fn inspections_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Inspection> {
    let mut list: Vec<&Inspection> =
        state.inspections.iter().filter(|i| i.project == project && i.deleted.is_none() && can_see_inspection(state, actor, i)).collect();
    list.sort_by_key(|i| i.number);
    list
}

/// One cost code's line of the budget view.
#[derive(Clone, Debug)]
pub struct BudgetRow {
    pub cost_code: Id<CostCode>,
    /// Whether the code has a budget line (costs can land on codes that have none).
    pub budgeted: bool,
    pub original: Money,
    /// Approved and complete commitments.
    pub committed: Money,
    pub direct: Money,
}

impl BudgetRow {
    /// Committed plus direct costs.
    pub fn cost(&self) -> Money {
        self.committed.checked_add(self.direct).unwrap_or(Money::ZERO)
    }

    /// What is left of the original budget.
    pub fn remaining(&self) -> Money {
        self.original.checked_sub(self.cost()).unwrap_or(Money::ZERO)
    }
}

/// A cost code usable in a project: a company-wide code or one of the project's own.
pub fn project_cost_codes(state: &CompanyState, project: Id<Project>) -> Vec<&CostCode> {
    let mut list: Vec<&CostCode> =
        state.cost_codes.iter().filter(|c| c.deleted.is_none() && (c.project.is_none() || c.project == Some(project))).collect();
    list.sort_by(|a, b| a.code.cmp(&b.code));
    list
}

fn too_big() -> EngineError {
    EngineError::conflict("the amounts are too large to add up")
}

fn budget_row(rows: &mut std::collections::BTreeMap<Id<CostCode>, BudgetRow>, code: Id<CostCode>) -> &mut BudgetRow {
    rows.entry(code).or_insert(BudgetRow { cost_code: code, budgeted: false, original: Money::ZERO, committed: Money::ZERO, direct: Money::ZERO })
}

/// The budget of a project by cost code: original budget, committed cost and direct cost, for every
/// code that has any of them, in code order.
pub fn budget_rows(state: &CompanyState, project: Id<Project>) -> Result<Vec<BudgetRow>, EngineError> {
    let mut rows: std::collections::BTreeMap<Id<CostCode>, BudgetRow> = std::collections::BTreeMap::new();
    for b in state.budget_lines.iter().filter(|b| b.project == project && b.deleted.is_none()) {
        let r = budget_row(&mut rows, b.cost_code);
        r.budgeted = true;
        r.original = r.original.checked_add(b.original).ok_or_else(too_big)?;
    }
    for c in state.commitments.iter().filter(|c| c.project == project && c.deleted.is_none() && c.status.counts()) {
        for l in &c.lines {
            let r = budget_row(&mut rows, l.cost_code);
            r.committed = r.committed.checked_add(l.amount).ok_or_else(too_big)?;
        }
    }
    for d in state.direct_costs.iter().filter(|d| d.project == project && d.deleted.is_none()) {
        let r = budget_row(&mut rows, d.cost_code);
        r.direct = r.direct.checked_add(d.amount).ok_or_else(too_big)?;
    }
    let mut list: Vec<BudgetRow> = rows.into_values().collect();
    list.sort_by(|a, b| {
        let code = |id: Id<CostCode>| state.cost_codes.get(id).map(|c| c.code.clone()).unwrap_or_default();
        code(a.cost_code).cmp(&code(b.cost_code))
    });
    Ok(list)
}

/// Totals of the budget rows (original, committed, direct).
pub fn budget_totals(rows: &[BudgetRow]) -> Result<(Money, Money, Money), EngineError> {
    let sum = |f: fn(&BudgetRow) -> Money| Money::sum(rows.iter().map(f)).ok_or_else(too_big);
    Ok((sum(|r| r.original)?, sum(|r| r.committed)?, sum(|r| r.direct)?))
}

/// Whether the actor may see a commitment: Read access, and drafts only for their author and admins.
pub fn can_see_commitment(state: &CompanyState, actor: Actor, c: &Commitment) -> bool {
    let lvl = level(state, actor, c.project, Tool::Commitments);
    if lvl == Level::Admin {
        return true;
    }
    if lvl < Level::Read || c.deleted.is_some() {
        return false;
    }
    c.status != CommitmentStatus::Draft || (actor.person().is_some() && c.created.by == actor.person())
}

/// The commitments of a project the actor may see, in number order.
pub fn commitments_for(state: &CompanyState, actor: Actor, project: Id<Project>) -> Vec<&Commitment> {
    let mut list: Vec<&Commitment> =
        state.commitments.iter().filter(|c| c.project == project && c.deleted.is_none() && can_see_commitment(state, actor, c)).collect();
    list.sort_by_key(|c| c.number);
    list
}
