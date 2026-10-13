//! Importing a schedule saved as a project-planning XML file (the public Microsoft Project XML format).
//!
//! The file is turned into the lines the text import reads, so the same checks apply (loops, depth, limits).
//! What is read: each task's name, dates, percent complete, milestone flag, outline level (which makes
//! summary tasks), predecessor links with their kind and lag, and the common date constraints. Resources,
//! calendars and costs in the file are ignored.

use buildkiosk_model::Level;
use buildkiosk_model::Tool;
use buildkiosk_model::project::Project;
use serde_json::{Value, json};

use super::{Access, CommandSpec};
use crate::sched::MAX_TASKS;
use crate::{Ctx, Params, Result};

pub const COMMANDS: &[CommandSpec] = &[CommandSpec::mutate(
    "schedule.import_xml",
    "Import a project-planning XML file",
    Tool::Schedule,
    Access::Project(Level::Standard),
    "project: project id; text: the contents of a Microsoft Project XML file (up to 8 MB). Tasks, dates, percent complete, milestones, outline levels, links (with kind and lag) and start/finish constraints are read; ids are renumbered",
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

/// A date from `2026-10-12T08:00:00`.
fn date_part(s: &str) -> Option<&str> {
    s.get(..10).filter(|d| buildkiosk_core::Date::parse(d).is_some())
}

/// The lines of the text import for the tasks in `xml`, and how many tasks were left out (blank, or without a name or dates).
pub fn rows_from_xml(xml: &str) -> std::result::Result<(String, usize), String> {
    let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 400_000 };
    let doc = roxmltree::Document::parse_with_options(xml, opts).map_err(|e| format!("the file is not readable XML: {e}"))?;
    let root = doc.root_element();
    if root.tag_name().name() != "Project" {
        return Err("this is not a Microsoft Project XML file (the first element should be Project)".into());
    }
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
            _ => ("", ""),
        };
        // Tabs separate the cells, so a name with commas is safe; tabs and line breaks in a name become spaces.
        let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
        lines.push_str(&format!(
            "{uid}\t{}\t{start}\t{finish}\t{percent}\t{}\t{parent}\t{milestone}\t{constraint}\t{constraint_date}\n",
            clean(name),
            links.join(";")
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
    let (lines, skipped) = rows_from_xml(xml).map_err(|e| p.bad("text", e))?;
    let out = super::schedule::import_body(ctx, project, &lines, p)?;
    let added = out.get("added").and_then(Value::as_u64).unwrap_or(0);
    Ok(json!({ "added": added, "created": added, "skipped": skipped }))
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
        let (lines, skipped) = rows_from_xml(FILE).unwrap();
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
            assert!(rows_from_xml(bad).is_err(), "{bad:?}");
        }
    }
}
