//! Importing a schedule saved in the tab-separated exchange format of a common planning program (an `.xer` file).
//!
//! Like the project-file import, the file is turned into the lines the text import reads, so the same checks
//! apply. What is read: the work breakdown (which becomes summary tasks whose dates roll up from the tasks
//! under them), each task's name, dates (actual, else scheduled, else planned), percent complete, milestone
//! flag, links with their kind and lag, and the common date constraints. Resources, calendars and costs in
//! the file are ignored.

use std::collections::{HashMap, HashSet};

use buildkiosk_core::Date;
use buildkiosk_model::Level;
use buildkiosk_model::Tool;
use buildkiosk_model::project::Project;
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::sched::MAX_TASKS;
use crate::{Ctx, Params, Result};

pub const COMMANDS: &[CommandSpec] = &[CommandSpec::mutate(
    "schedule.import_xer",
    "Import a planning-program exchange file",
    Tool::Schedule,
    Access::Project(Level::Standard),
    "project: project id; text: the contents of an .xer file (up to 8 MB); proj_id?: which project in the file when it holds several (default the first). Tasks, work breakdown (as summary tasks), dates, percent complete, milestones, links (with kind and lag) and date constraints are read; ids are renumbered",
    import_xer,
)];

/// The most characters of an .xer file one import reads.
const MAX_XER: usize = 8_000_000;
/// The most rows read from any one table.
const MAX_ROWS: usize = 200_000;
/// A working day, in hours, for turning lags into days.
const DAY_HOURS: f64 = 8.0;

/// One table of the file: its field names and its rows.
#[derive(Default)]
struct Table {
    fields: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    fn index(&self, field: &str) -> Option<usize> {
        self.fields.iter().position(|f| f == field)
    }

    /// A row's cell by field name (empty when the field or cell is missing).
    fn get<'a>(&self, row: &'a [String], field: &str) -> &'a str {
        self.index(field).and_then(|i| row.get(i)).map_or("", |s| s.trim())
    }
}

fn read_tables(text: &str) -> std::result::Result<HashMap<String, Table>, String> {
    let mut tables: HashMap<String, Table> = HashMap::new();
    let mut current: Option<String> = None;
    let mut saw_header = false;
    for line in text.lines() {
        let mut cells = line.split('\t');
        match cells.next() {
            Some(first) if first.trim_start_matches('\u{feff}') == "ERMHDR" => saw_header = true,
            Some("%T") => {
                let name = cells.next().unwrap_or("").trim().to_string();
                tables.entry(name.clone()).or_default();
                current = Some(name);
            }
            Some("%F") => {
                if let Some(t) = current.as_ref().and_then(|c| tables.get_mut(c)) {
                    t.fields = cells.map(|c| c.trim().to_string()).collect();
                }
            }
            Some("%R") => {
                if let Some(t) = current.as_ref().and_then(|c| tables.get_mut(c)) {
                    if t.rows.len() >= MAX_ROWS {
                        return Err(format!("a table in the file has more than {MAX_ROWS} rows"));
                    }
                    t.rows.push(cells.map(str::to_string).collect());
                }
            }
            Some("%E") => current = None,
            _ => {}
        }
    }
    if !saw_header && !tables.contains_key("TASK") {
        return Err("this is not a planning-program exchange (.xer) file".into());
    }
    Ok(tables)
}

/// A date from `2026-10-12 08:00`.
fn date_part(s: &str) -> Option<Date> {
    s.get(..10).and_then(Date::parse)
}

fn first_date(t: &Table, row: &[String], fields: &[&str]) -> Option<Date> {
    fields.iter().find_map(|f| date_part(t.get(row, f)))
}

/// The lines of the text import for the tasks in `text`, and how many tasks were left out.
pub fn rows_from_xer(text: &str, which: Option<&str>) -> std::result::Result<(String, usize), String> {
    let tables = read_tables(text)?;
    let empty = Table::default();
    let tasks = tables.get("TASK").ok_or("the file has no tasks")?;
    let wbs = tables.get("PROJWBS").unwrap_or(&empty);
    let preds = tables.get("TASKPRED").unwrap_or(&empty);
    let projects = tables.get("PROJECT").unwrap_or(&empty);

    // Which project: the one asked for, else the first in the file, else the first task's.
    let proj_id: String = match which.map(str::trim).filter(|w| !w.is_empty()) {
        Some(w) => {
            if !projects.rows.is_empty() && !projects.rows.iter().any(|r| projects.get(r, "proj_id") == w) {
                return Err(format!("the file has no project {w}"));
            }
            w.to_string()
        }
        None => projects
            .rows
            .first()
            .map(|r| projects.get(r, "proj_id").to_string())
            .or_else(|| tasks.rows.first().map(|r| tasks.get(r, "proj_id").to_string()))
            .unwrap_or_default(),
    };
    let in_project = |t: &Table, r: &[String]| proj_id.is_empty() || t.get(r, "proj_id") == proj_id;

    // The work breakdown: every node but the project's root becomes a summary task.
    struct Node {
        name: String,
        parent: String,
        start: Option<Date>,
        finish: Option<Date>,
    }
    let mut nodes: Vec<(String, Node)> = Vec::new();
    let mut root: HashSet<String> = HashSet::new();
    for r in wbs.rows.iter().filter(|r| in_project(wbs, r)) {
        let id = wbs.get(r, "wbs_id").to_string();
        if id.is_empty() {
            continue;
        }
        if wbs.get(r, "proj_node_flag") == "Y" || wbs.get(r, "parent_wbs_id").is_empty() {
            root.insert(id);
            continue;
        }
        let name = match wbs.get(r, "wbs_name") {
            "" => wbs.get(r, "wbs_short_name"),
            n => n,
        };
        nodes.push((id, Node { name: name.to_string(), parent: wbs.get(r, "parent_wbs_id").to_string(), start: None, finish: None }));
    }

    // Tasks.
    struct Row {
        id: String,
        name: String,
        wbs: String,
        start: Date,
        finish: Date,
        percent: i64,
        milestone: bool,
        constraint: &'static str,
        constraint_date: String,
    }
    let mut kept: Vec<Row> = Vec::new();
    let mut skipped = 0usize;
    for r in tasks.rows.iter().filter(|r| in_project(tasks, r)) {
        let kind = tasks.get(r, "task_type");
        let id = tasks.get(r, "task_id");
        // A summary line the program draws for a work breakdown node is already a summary task.
        if id.is_empty() || kind == "TT_WBS" {
            continue;
        }
        let name = match tasks.get(r, "task_name") {
            "" => tasks.get(r, "task_code"),
            n => n,
        };
        let start = first_date(tasks, r, &["act_start_date", "early_start_date", "target_start_date"]);
        let finish = first_date(tasks, r, &["act_end_date", "early_end_date", "target_end_date"]).or(start);
        let (Some(start), Some(finish)) = (start, finish) else {
            skipped += 1;
            continue;
        };
        if name.is_empty() {
            skipped += 1;
            continue;
        }
        if kept.len() >= MAX_TASKS {
            return Err(format!("the file has more than {MAX_TASKS} tasks"));
        }
        let hours = |f: &str| tasks.get(r, f).parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0);
        let percent = match tasks.get(r, "status_code") {
            "TK_Complete" => 100,
            "TK_NotStart" => 0,
            _ => match tasks.get(r, "phys_complete_pct").parse::<f64>().ok().filter(|v| v.is_finite()) {
                Some(p) => p.round() as i64,
                None => match (hours("target_drtn_hr_cnt"), hours("remain_drtn_hr_cnt")) {
                    (Some(total), Some(left)) if total > 0.0 => (((total - left.min(total)) / total) * 100.0).round() as i64,
                    _ => 0,
                },
            },
        }
        .clamp(0, 100);
        let cdate = date_part(tasks.get(r, "cstr_date")).map(|d| d.to_string()).unwrap_or_default();
        let (constraint, constraint_date) = match (tasks.get(r, "cstr_type"), cdate.is_empty()) {
            ("CS_MSO", false) => ("mso", cdate),
            ("CS_MEO", false) => ("mfo", cdate),
            ("CS_MSOA", false) => ("snet", cdate),
            ("CS_MEOB", false) => ("fnlt", cdate),
            ("CS_MSOB", false) => ("snlt", cdate),
            ("CS_MEOA", false) => ("fnet", cdate),
            ("CS_ALAP", _) => ("alap", String::new()),
            _ => ("", String::new()),
        };
        kept.push(Row {
            id: id.to_string(),
            name: name.chars().take(200).collect(),
            wbs: tasks.get(r, "wbs_id").to_string(),
            start,
            finish: finish.max(start),
            percent,
            milestone: matches!(kind, "TT_Mile" | "TT_FinMile"),
            constraint,
            constraint_date,
        });
    }
    if kept.is_empty() {
        return Err("the file has no tasks with a name and dates".into());
    }

    // Links, written on the successor: `pred`, `predSS+2`.
    let task_ids: HashSet<&str> = kept.iter().map(|k| k.id.as_str()).collect();
    let mut links: HashMap<String, Vec<String>> = HashMap::new();
    for r in preds.rows.iter().filter(|r| in_project(preds, r)) {
        let (succ, pred) = (preds.get(r, "task_id"), preds.get(r, "pred_task_id"));
        if !task_ids.contains(succ) || !task_ids.contains(pred) {
            continue;
        }
        let kind = match preds.get(r, "pred_type") {
            "PR_SS" => "SS",
            "PR_FF" => "FF",
            "PR_SF" => "SF",
            _ => "FS",
        };
        let hours = preds.get(r, "lag_hr_cnt").parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
        let days = (hours / DAY_HOURS).round().clamp(-1_000.0, 1_000.0) as i64;
        links.entry(succ.to_string()).or_default().push(if days == 0 { format!("{pred}{kind}") } else { format!("{pred}{kind}{days:+}") });
    }

    // Summary dates roll up from the tasks under each node (and its child nodes).
    let node_index: HashMap<&str, usize> = nodes.iter().enumerate().map(|(i, (id, _))| (id.as_str(), i)).collect();
    let mut spans: Vec<(Option<Date>, Option<Date>)> = vec![(None, None); nodes.len()];
    for k in &kept {
        let mut at = node_index.get(k.wbs.as_str()).copied();
        // Walking up is bounded so a loop in the file can't hang the import.
        for _ in 0..64 {
            let Some(i) = at else { break };
            if let Some(s) = spans.get_mut(i) {
                s.0 = Some(s.0.map_or(k.start, |d| d.min(k.start)));
                s.1 = Some(s.1.map_or(k.finish, |d| d.max(k.finish)));
            }
            at = nodes.get(i).and_then(|(_, n)| node_index.get(n.parent.as_str()).copied());
        }
    }
    for (n, s) in nodes.iter_mut().zip(&spans) {
        n.1.start = s.0;
        n.1.finish = s.1;
    }

    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut lines = String::new();
    // Nodes with nothing under them are left out, and so are their children's references to them.
    let live: HashSet<&str> = nodes.iter().filter(|(_, n)| n.start.is_some()).map(|(id, _)| id.as_str()).collect();
    for (id, n) in nodes.iter().filter(|(id, _)| live.contains(id.as_str())) {
        let (Some(start), Some(finish)) = (n.start, n.finish) else { continue };
        let parent = if live.contains(n.parent.as_str()) { format!("W{}", n.parent) } else { String::new() };
        lines.push_str(&format!("W{id}\t{}\t{start}\t{finish}\t0\t\t{parent}\tno\t\t\n", clean(&n.name)));
    }
    for k in &kept {
        let parent = if live.contains(k.wbs.as_str()) { format!("W{}", k.wbs) } else { String::new() };
        lines.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            k.id,
            clean(&k.name),
            k.start,
            k.finish,
            k.percent,
            links.get(&k.id).map(|l| l.join(";")).unwrap_or_default(),
            parent,
            if k.milestone { "yes" } else { "no" },
            k.constraint,
            k.constraint_date
        ));
    }
    Ok((lines, skipped))
}

fn import_xer(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: buildkiosk_core::Id<Project> = p.id("project")?;
    let text = p.req_text("text", MAX_XER)?;
    let which = p.line("proj_id", 40)?;
    let (lines, skipped) = rows_from_xer(&text, which.as_deref()).map_err(|e| p.bad("text", e))?;
    let out = super::schedule::import_body(ctx, project, &lines, p)?;
    let added = out.get("added").and_then(Value::as_u64).unwrap_or(0);
    Ok(json!({ "added": added, "created": added, "skipped": skipped }))
}
