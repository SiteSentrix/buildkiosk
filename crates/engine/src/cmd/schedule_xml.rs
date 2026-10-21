//! Importing a schedule saved as a project-planning XML file (the public Microsoft Project XML format),
//! and writing one.
//!
//! The file is turned into the lines the text import reads, so the same checks apply (loops, depth, limits).
//! What is read: each task's name, dates, percent complete, milestone flag, outline level (which makes
//! summary tasks), predecessor links with their kind and lag, and the common date constraints. On request it also
//! reads the resources each task is assigned (`resources`), what the file says there are of each resource (`limits`)
//! and the project's working days and days off (`calendar`). Costs in the file are ignored.

use std::collections::HashMap;

use buildkiosk_core::Date;
use buildkiosk_model::Level;
use buildkiosk_model::Tool;
use buildkiosk_model::project::Project;
use buildkiosk_model::schedule::{Assignment, Holiday, MAX_ASSIGNMENTS, MAX_HOLIDAYS, MAX_UNITS, WorkCalendar};
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::sched::MAX_TASKS;
use crate::{Ctx, Params, Result};

pub const COMMANDS: &[CommandSpec] = &[CommandSpec::mutate(
    "schedule.import_xml",
    "Import a project-planning XML file",
    Tool::Schedule,
    Access::Project(Level::Standard),
    "project: project id; text: the contents of a Microsoft Project XML file (up to 8 MB); resources?: bool (also take the resources assigned to each task, as units a day; a unit of 100% is one) ; limits?: bool (project admins: also set the project's resource limits from the largest number the file says there are of each resource); calendar?: bool (project admins: also replace the project's working days and days off with the file's project calendar; run schedule.recalculate afterwards). Tasks, dates, percent complete, milestones, outline levels, links (with kind and lag) and start/finish constraints are read; ids are renumbered",
    import_xml,
)];

/// The most characters of XML one import reads.
const MAX_XML: usize = 8_000_000;
/// A link's lag is in tenths of a minute; a working day is taken as eight hours.
const LAG_PER_DAY: i64 = 8 * 60 * 10;

fn child<'a, 'b>(n: roxmltree::Node<'a, 'b>, name: &str) -> Option<roxmltree::Node<'a, 'b>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn text_of<'a>(n: roxmltree::Node<'a, '_>, name: &str) -> Option<&'a str> {
    child(n, name).and_then(|c| c.text()).map(str::trim).filter(|t| !t.is_empty())
}

fn parse(xml: &str) -> std::result::Result<roxmltree::Document<'_>, String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 400_000 };
    let doc = roxmltree::Document::parse_with_options(xml, opts).map_err(|e| format!("the file is not readable XML: {e}"))?;
    if doc.root_element().tag_name().name() != "Project" {
        return Err("this is not a Microsoft Project XML file (the first element should be Project)".into());
    }
    Ok(doc)
}

/// A resource's name made safe for the text import: no separators, single spaces.
fn resource_name(raw: &str) -> String {
    let cleaned: String = raw.chars().map(|c| if matches!(c, ',' | ';' | '\t' | '\n' | '\r') || c.is_control() { ' ' } else { c }).collect();
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(50).collect::<String>().trim().to_string()
}

/// The named resources of the file by UID, leaving out the "unassigned" row (UID 0, or -65535 in assignments).
fn resource_names(root: roxmltree::Node<'_, '_>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(resources) = child(root, "Resources") else { return out };
    for r in resources.children().filter(|c| c.is_element() && c.tag_name().name() == "Resource") {
        let (Some(uid), Some(name)) = (text_of(r, "UID"), text_of(r, "Name")) else { continue };
        let name = resource_name(name);
        // A name that is only digits can't be told from a quantity.
        if uid == "0" || name.is_empty() || name.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        out.insert(uid.to_string(), name);
    }
    out
}

/// What a task is assigned, in whole units a day: 100% is one, 250% is three (rounded), and never less than one.
fn units_of(raw: Option<&str>) -> u32 {
    let units = raw.and_then(|u| u.parse::<f64>().ok()).filter(|u| u.is_finite() && *u > 0.0).unwrap_or(1.0);
    (units.round().clamp(1.0, f64::from(MAX_UNITS))) as u32
}

/// The resources assigned to each task, by the task's UID: the same resource twice adds up, up to the most one task names.
fn assignments(root: roxmltree::Node<'_, '_>) -> HashMap<String, Vec<Assignment>> {
    let names = resource_names(root);
    let mut out: HashMap<String, Vec<Assignment>> = HashMap::new();
    let Some(list) = child(root, "Assignments") else { return out };
    for a in list.children().filter(|c| c.is_element() && c.tag_name().name() == "Assignment") {
        let (Some(task), Some(res)) = (text_of(a, "TaskUID"), text_of(a, "ResourceUID")) else { continue };
        let Some(name) = names.get(res) else { continue };
        let units = units_of(text_of(a, "Units"));
        let entry = out.entry(task.to_string()).or_default();
        if let Some(x) = entry.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) {
            x.units = x.units.saturating_add(units).min(MAX_UNITS);
        } else if entry.len() < MAX_ASSIGNMENTS {
            entry.push(Assignment { name: name.clone(), units });
        }
    }
    out
}

/// How many of each resource the file says there are (`MaxUnits`, 100% being one), for resources that are assigned to a task,
/// largest first spelling kept. Resources the file gives no maximum for are left out.
pub fn limits_from_xml(xml: &str) -> std::result::Result<Vec<Assignment>, String> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    let used: std::collections::HashSet<String> = child(root, "Assignments")
        .into_iter()
        .flat_map(|l| l.children().filter(|c| c.is_element()))
        .filter_map(|a| text_of(a, "ResourceUID"))
        .map(str::to_string)
        .collect();
    let names = resource_names(root);
    let mut out: Vec<Assignment> = Vec::new();
    let Some(resources) = child(root, "Resources") else { return Ok(out) };
    for r in resources.children().filter(|c| c.is_element() && c.tag_name().name() == "Resource") {
        let Some(uid) = text_of(r, "UID") else { continue };
        let (Some(name), true) = (names.get(uid), used.contains(uid)) else { continue };
        let Some(max) = text_of(r, "MaxUnits") else { continue };
        let units = units_of(Some(max));
        if let Some(x) = out.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) {
            x.units = x.units.saturating_add(units).min(MAX_UNITS);
        } else if out.len() < MAX_ASSIGNMENTS {
            out.push(Assignment { name: name.clone(), units });
        }
    }
    Ok(out)
}

/// The project's working calendar from the file: the weekdays worked (`DayType` 1 is Sunday, 2 Monday, and so on) and the
/// days off listed as exceptions (a range of dates that are not worked). The file's own `CalendarUID` picks the calendar,
/// the first base calendar otherwise. Exceptions that make a day a working day, and days off past the most a calendar keeps,
/// are counted as left out in the second value. `None` when the file has no usable calendar.
pub fn calendar_from_xml(xml: &str) -> std::result::Result<Option<(WorkCalendar, usize)>, String> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    let Some(list) = child(root, "Calendars") else { return Ok(None) };
    let calendars: Vec<roxmltree::Node<'_, '_>> = list.children().filter(|c| c.is_element() && c.tag_name().name() == "Calendar").collect();
    let wanted = text_of(root, "CalendarUID");
    let pick = calendars
        .iter()
        .find(|c| wanted.is_some() && text_of(**c, "UID") == wanted)
        .or_else(|| calendars.iter().find(|c| text_of(**c, "IsBaseCalendar") == Some("1")))
        .or_else(|| calendars.first());
    let Some(cal) = pick else { return Ok(None) };
    let mut days = [false; 7];
    let mut seen = false;
    if let Some(week) = child(*cal, "WeekDays") {
        for d in week.children().filter(|c| c.is_element() && c.tag_name().name() == "WeekDay") {
            // The weekday's own entry; an entry with a time period is an exception to a weekday, not the weekday itself.
            if child(d, "TimePeriod").is_some() {
                continue;
            }
            let (Some(kind), Some(working)) = (text_of(d, "DayType").and_then(|t| t.parse::<usize>().ok()), text_of(d, "DayWorking")) else {
                continue;
            };
            // 1 is Sunday in the file; Monday first here.
            let at = match kind {
                1 => 6,
                2..=7 => kind - 2,
                _ => continue,
            };
            if let Some(slot) = days.get_mut(at) {
                *slot = working == "1";
                seen = true;
            }
        }
    }
    if !seen {
        days = WorkCalendar::default().days;
    }
    let mut holidays: Vec<Holiday> = Vec::new();
    let mut left_out = 0usize;
    let mut exceptions: Vec<(Date, Date, bool, String)> = Vec::new();
    if let Some(list) = child(*cal, "Exceptions") {
        for e in list.children().filter(|c| c.is_element() && c.tag_name().name() == "Exception") {
            let period = child(e, "TimePeriod");
            let (Some(from), Some(to)) = (
                period.and_then(|p| text_of(p, "FromDate")).and_then(date_part).and_then(Date::parse),
                period.and_then(|p| text_of(p, "ToDate")).and_then(date_part).and_then(Date::parse),
            ) else {
                continue;
            };
            exceptions.push((from, to, text_of(e, "DayWorking") == Some("1"), resource_name(text_of(e, "Name").unwrap_or(""))));
        }
    }
    // Weekly work-day exceptions of a calendar that also lists them under WeekDays (with a time period) are skipped above; the
    // plain list of exceptions is what is read here.
    exceptions.sort_by_key(|(from, ..)| *from);
    for (from, to, works, name) in exceptions {
        if works {
            left_out += 1;
            continue;
        }
        let mut d = from;
        let mut n = 0;
        while d <= to && n < 400 {
            if holidays.len() >= MAX_HOLIDAYS {
                left_out += 1;
                break;
            }
            if holidays.binary_search_by_key(&d, |h| h.date).is_err() {
                let at = holidays.partition_point(|h| h.date < d);
                holidays.insert(at, Holiday { date: d, name: name.chars().take(60).collect() });
            }
            n += 1;
            let Some(next) = d.add_days(1) else { break };
            d = next;
        }
    }
    let cal = WorkCalendar { days, holidays };
    Ok(cal.is_valid().then_some((cal, left_out)))
}

/// A date from `2026-10-12T08:00:00`.
fn date_part(s: &str) -> Option<&str> {
    s.get(..10).filter(|d| buildkiosk_core::Date::parse(d).is_some())
}

/// The lines of the text import for the tasks in `xml`, and how many tasks were left out (blank, or without a name or dates).
pub fn rows_from_xml(xml: &str, with_resources: bool) -> std::result::Result<(String, usize), String> {
    let doc = parse(xml)?;
    let root = doc.root_element();
    let assigned = if with_resources { assignments(root) } else { HashMap::new() };
    let tasks = child(root, "Tasks").ok_or("the file has no tasks")?;
    let mut lines = String::new();
    let (mut skipped, mut kept) = (0usize, 0usize);
    // The nearest task above at each outline level, to find a task's summary task.
    let mut stack: Vec<(i64, String)> = Vec::new();
    for t in tasks.children().filter(|c| c.is_element() && c.tag_name().name() == "Task") {
        let uid = text_of(t, "UID").unwrap_or("");
        // The project's own summary line (UID 0, level 0) and empty rows aren't tasks.
        if uid.is_empty() || uid == "0" || text_of(t, "IsNull") == Some("1") {
            continue;
        }
        let (Some(name), Some(start), Some(finish)) =
            (text_of(t, "Name"), text_of(t, "Start").and_then(date_part), text_of(t, "Finish").and_then(date_part))
        else {
            skipped += 1;
            continue;
        };
        kept += 1;
        if kept > MAX_TASKS {
            return Err(format!("the file has more than {MAX_TASKS} tasks"));
        }
        let level = text_of(t, "OutlineLevel").and_then(|l| l.parse::<i64>().ok()).unwrap_or(1).max(1);
        while stack.last().is_some_and(|(l, _)| *l >= level) {
            stack.pop();
        }
        let parent = stack.last().map(|(_, u)| u.clone()).unwrap_or_default();
        stack.push((level, uid.to_string()));
        let percent = text_of(t, "PercentComplete").and_then(|p| p.parse::<i64>().ok()).unwrap_or(0).clamp(0, 100);
        let milestone = if text_of(t, "Milestone") == Some("1") { "yes" } else { "no" };
        let links: Vec<String> = t
            .children()
            .filter(|c| c.is_element() && c.tag_name().name() == "PredecessorLink")
            .filter_map(|l| {
                let pred = text_of(l, "PredecessorUID")?;
                let kind = match text_of(l, "Type").unwrap_or("1") {
                    "0" => "FF",
                    "2" => "SF",
                    "3" => "SS",
                    _ => "FS",
                };
                // Rounded to whole days, as the schedule counts in days.
                let lag = text_of(l, "LinkLag").and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                let days = (lag as f64 / LAG_PER_DAY as f64).round() as i64;
                Some(if days == 0 { format!("{pred}{kind}") } else { format!("{pred}{kind}{days:+}") })
            })
            .collect();
        let (constraint, constraint_date) = match (text_of(t, "ConstraintType"), text_of(t, "ConstraintDate").and_then(date_part)) {
            (Some("2"), Some(d)) => ("mso", d),
            (Some("4"), Some(d)) => ("snet", d),
            (Some("7"), Some(d)) => ("fnlt", d),
            (Some("3"), Some(d)) => ("mfo", d),
            (Some("5"), Some(d)) => ("snlt", d),
            (Some("6"), Some(d)) => ("fnet", d),
            (Some("1"), _) => ("alap", ""),
            _ => ("", ""),
        };
        // Tabs separate the cells, so a name with commas is safe; tabs and line breaks in a name become spaces.
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        lines.push_str(&format!(
            "{uid}\t{}\t{start}\t{finish}\t{percent}\t{}\t{parent}\t{milestone}\t{constraint}\t{constraint_date}\t{}\n",
            clean(name),
            links.join(";"),
            assigned.get(uid).map(|a| a.iter().map(|x| format!("{} {}", x.name, x.units)).collect::<Vec<_>>().join(", ")).unwrap_or_default()
        ));
    }
    if lines.is_empty() {
        return Err("the file has no tasks with a name and dates".into());
    }
    Ok((lines, skipped))
}

fn import_xml(ctx: &mut Ctx<'_>, p: &Params<'_>) -> Result<Value> {
    let project: buildkiosk_core::Id<Project> = p.id("project")?;
    let xml = p.req_text("text", MAX_XML)?;
    let xml = xml.trim_start_matches('\u{feff}');
    let (with_resources, with_limits, with_calendar) = (p.flag_or("resources", false)?, p.flag_or("limits", false)?, p.flag_or("calendar", false)?);
    // The limits and the calendar belong to the project, so they take a project admin, which adding tasks does not.
    if (with_limits || with_calendar) && crate::view::level(ctx.state, ctx.actor, project, Tool::Schedule) < Level::Admin {
        return Err(crate::EngineError::forbidden("only project admins can take the resource limits or the calendar from a file"));
    }
    let (lines, skipped) = rows_from_xml(xml, with_resources).map_err(|e| p.bad("text", e))?;
    // Read everything before changing anything.
    let limits = if with_limits { Some(limits_from_xml(xml).map_err(|e| p.bad("text", e))?) } else { None };
    let calendar = if with_calendar { calendar_from_xml(xml).map_err(|e| p.bad("text", e))? } else { None };
    let out = super::schedule::import_body(ctx, project, &lines, p)?;
    let added = out.get("added").and_then(Value::as_u64).unwrap_or(0);
    let mut result = serde_json::Map::new();
    result.insert("added".into(), json!(added));
    result.insert("created".into(), json!(added));
    result.insert("skipped".into(), json!(skipped));
    if let Some(limits) = limits {
        result.insert("limits".into(), json!(limits.len()));
        ctx.state.projects.update(project, |x| x.resource_limits = limits);
    }
    if with_calendar {
        match calendar {
            Some((cal, left_out)) => {
                ctx.state.projects.update(project, |x| x.calendar = cal);
                result.insert("calendar".into(), json!(true));
                result.insert("calendar_left_out".into(), json!(left_out));
            }
            None => {
                result.insert("calendar".into(), json!(false));
            }
        }
    }
    Ok(Value::Object(result))
}

/// The XML escape of text for an element or attribute.
fn esc(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .flat_map(|c| match c {
            '&' => "&amp;".chars().collect::<Vec<_>>(),
            '<' => "&lt;".chars().collect(),
            '>' => "&gt;".chars().collect(),
            '"' => "&quot;".chars().collect(),
            '\'' => "&apos;".chars().collect(),
            c => vec![c],
        })
        .collect()
}

/// The schedule as a Microsoft Project XML file: the tasks in outline order with dates, percent complete,
/// milestones, links with kind and lag, and constraints. What `import_xml` reads is what this writes.
pub fn export_xml(state: &buildkiosk_model::CompanyState, project: buildkiosk_core::Id<Project>, today: buildkiosk_core::Date) -> String {
    use buildkiosk_model::schedule::{Constraint, LinkKind};
    let rows = crate::sched::rows(state, project, today);
    let name = state.projects.get(project).map(|p| p.name.clone()).unwrap_or_default();
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Project xmlns=\"http://schemas.microsoft.com/project\">\n");
    out.push_str(&format!("  <Name>{}</Name>\n  <Tasks>\n", esc(&name)));
    for (i, r) in rows.iter().enumerate() {
        let Some(task) = state.schedule_tasks.get(r.id) else { continue };
        out.push_str("    <Task>\n");
        out.push_str(&format!("      <UID>{}</UID>\n      <ID>{}</ID>\n      <Name>{}</Name>\n", task.number, i + 1, esc(&r.name)));
        out.push_str(&format!("      <OutlineLevel>{}</OutlineLevel>\n", r.depth + 1));
        out.push_str(&format!("      <Start>{}T08:00:00</Start>\n      <Finish>{}T17:00:00</Finish>\n", r.start, r.finish));
        out.push_str(&format!("      <PercentComplete>{}</PercentComplete>\n", r.percent));
        out.push_str(&format!("      <Milestone>{}</Milestone>\n      <Summary>{}</Summary>\n", u8::from(r.milestone), u8::from(r.summary)));
        for l in &task.predecessors {
            let Some(pred) = state.schedule_tasks.get(l.task).filter(|p| p.deleted.is_none()) else { continue };
            let kind = match l.kind {
                LinkKind::FinishToFinish => 0,
                LinkKind::FinishToStart => 1,
                LinkKind::StartToFinish => 2,
                LinkKind::StartToStart => 3,
            };
            out.push_str(&format!(
                "      <PredecessorLink>\n        <PredecessorUID>{}</PredecessorUID>\n        <Type>{kind}</Type>\n        <LinkLag>{}</LinkLag>\n      </PredecessorLink>\n",
                pred.number,
                i64::from(l.lag) * LAG_PER_DAY
            ));
        }
        let constraint = match (task.constraint, task.constraint_date) {
            (Constraint::MustStartOn, Some(d)) => Some((2, Some(d))),
            (Constraint::StartNoEarlierThan, Some(d)) => Some((4, Some(d))),
            (Constraint::FinishNoLaterThan, Some(d)) => Some((7, Some(d))),
            (Constraint::MustFinishOn, Some(d)) => Some((3, Some(d))),
            (Constraint::StartNoLaterThan, Some(d)) => Some((5, Some(d))),
            (Constraint::FinishNoEarlierThan, Some(d)) => Some((6, Some(d))),
            (Constraint::AsLateAsPossible, _) => Some((1, None)),
            _ => None,
        };
        match constraint {
            Some((kind, Some(d))) => {
                out.push_str(&format!("      <ConstraintType>{kind}</ConstraintType>\n      <ConstraintDate>{d}T08:00:00</ConstraintDate>\n"))
            }
            Some((kind, None)) => out.push_str(&format!("      <ConstraintType>{kind}</ConstraintType>\n")),
            None => {}
        }
        out.push_str("    </Task>\n");
    }
    out.push_str("  </Tasks>\n</Project>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Project xmlns="http://schemas.microsoft.com/project">
  <Name>Clinic</Name>
  <Tasks>
    <Task><UID>0</UID><ID>0</ID><Name>Clinic</Name><OutlineLevel>0</OutlineLevel><Start>2026-10-12T08:00:00</Start><Finish>2026-11-06T17:00:00</Finish></Task>
    <Task><UID>1</UID><ID>1</ID><Name>Site work, phase 1</Name><OutlineLevel>1</OutlineLevel><Summary>1</Summary><Start>2026-10-12T08:00:00</Start><Finish>2026-10-16T17:00:00</Finish></Task>
    <Task><UID>2</UID><ID>2</ID><Name>Clear</Name><OutlineLevel>2</OutlineLevel><Start>2026-10-12T08:00:00</Start><Finish>2026-10-13T17:00:00</Finish><PercentComplete>40</PercentComplete></Task>
    <Task><UID>3</UID><ID>3</ID><Name>Grade</Name><OutlineLevel>2</OutlineLevel><Start>2026-10-14T08:00:00</Start><Finish>2026-10-16T17:00:00</Finish>
      <PredecessorLink><PredecessorUID>2</PredecessorUID><Type>3</Type><LinkLag>4800</LinkLag></PredecessorLink>
      <ConstraintType>4</ConstraintType><ConstraintDate>2026-10-14T08:00:00</ConstraintDate></Task>
    <Task><UID>4</UID><ID>4</ID><Name>Ready</Name><OutlineLevel>1</OutlineLevel><Milestone>1</Milestone><Start>2026-10-19T08:00:00</Start><Finish>2026-10-19T08:00:00</Finish>
      <PredecessorLink><PredecessorUID>3</PredecessorUID><Type>1</Type><LinkLag>0</LinkLag></PredecessorLink></Task>
    <Task><UID>5</UID><ID>5</ID><Name>No dates</Name><OutlineLevel>1</OutlineLevel></Task>
    <Task><UID>6</UID><ID>6</ID><IsNull>1</IsNull></Task>
  </Tasks>
</Project>"#;

    #[test]
    fn reads_tasks_levels_links_and_constraints() {
        let (lines, skipped) = rows_from_xml(FILE, false).unwrap();
        let rows: Vec<Vec<&str>> = lines.lines().map(|l| l.split('\t').collect()).collect();
        assert_eq!(skipped, 1, "the task with no dates; the empty row and the project line aren't counted");
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0][..4], ["1", "Site work, phase 1", "2026-10-12", "2026-10-16"]);
        assert_eq!((rows[1][6], rows[2][6], rows[3][6]), ("1", "1", ""), "outline levels make parents");
        assert_eq!((rows[1][4], rows[2][5], rows[3][5]), ("40", "2SS+1", "3FS"));
        assert_eq!((rows[2][8], rows[2][9]), ("snet", "2026-10-14"));
        assert_eq!(rows[3][7], "yes");
    }

    #[test]
    fn refuses_what_is_not_a_project_file() {
        for bad in
            ["", "not xml", "<Other/>", "<Project/>", "<Project><Tasks/></Project>", "<!DOCTYPE x [<!ENTITY a \"b\">]><Project><Tasks/></Project>"]
        {
            assert!(rows_from_xml(bad, true).is_err(), "{bad:?}");
        }
    }
}
