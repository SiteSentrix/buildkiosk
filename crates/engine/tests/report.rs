//! Cross-tool reports respect what each person may see.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

fn tool(list: &serde_json::Value, name: &str) -> Option<serde_json::Value> {
    list.as_array().unwrap().iter().find(|r| r["tool"] == name).cloned()
}

#[test]
fn summary_and_waiting_on_me() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");

    w.ok(pm, "task.create", json!({"project": p, "title": "Order hardware", "assignees": [sub.raw()], "due": "2020-01-01"}));
    w.ok(pm, "task.create", json!({"project": p, "title": "File permit"}));
    w.ok(pm, "punch.create", json!({"project": p, "title": "Fix door", "assignees": [sub.raw()]}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Excavate", "start": "2020-01-06", "days": 2}));
    w.ok(pm, "meeting.create", json!({"project": p, "title": "Kickoff", "date": "2026-10-20", "invitees": [sub.raw()], "actions": [{"description": "Send insurance", "assignee": sub.raw()}]}));

    let mine = w.ok(pm, "report.summary", json!({"project": p}));
    let tasks = tool(&mine, "Tasks").unwrap();
    assert_eq!((tasks["total"].as_u64(), tasks["open"].as_u64(), tasks["overdue"].as_u64()), (Some(2), Some(2), Some(1)));
    assert_eq!(tool(&mine, "Schedule tasks").unwrap()["overdue"], 1);
    assert!(tool(&mine, "Commitments").is_some());

    // The subcontractor has no financial tools, so their report has none either.
    let theirs = w.ok(sub, "report.summary", json!({"project": p}));
    assert!(tool(&theirs, "Commitments").is_none() && tool(&theirs, "Tasks").is_some());

    let todo = w.ok(sub, "report.waiting_on_me", json!({"project": p}));
    let titles: Vec<&str> = todo.as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap()).collect();
    assert_eq!(titles.len(), 3, "{titles:?}");
    assert_eq!(titles[0], "Order hardware", "the one with the earliest due date comes first");
    let mine_todo = w.ok(pm, "report.waiting_on_me", json!({"project": p}));
    assert_eq!(mine_todo.as_array().unwrap().len(), 1, "an unassigned task waits on its manager");
    assert_eq!(mine_todo[0]["title"], "File permit");
    let other = w.project("Other");
    w.err(pm, "report.summary", json!({"project": other}));
}

#[test]
fn portfolio_lists_only_your_projects() {
    let mut w = World::new();
    let a = w.project("Riverside Clinic");
    let b = w.project("Harbor Lofts");
    let pm = w.member(a, "Paula", "Manager", "Project manager");
    w.ok(pm, "task.create", json!({"project": a, "title": "Order hardware", "due": "2020-01-01"}));
    let rows = w.ok(pm, "report.portfolio", json!({}));
    let rows = rows.as_array().unwrap();
    assert_eq!(rows.len(), 1, "only the project Paula is on");
    assert_eq!((rows[0]["name"].as_str(), rows[0]["overdue"].as_u64()), (Some("Riverside Clinic"), Some(1)));
    assert!(rows[0]["revised"].is_string());
    let all = w.ok(w.admin(), "report.portfolio", json!({}));
    assert_eq!(all.as_array().unwrap().len(), 2);
    let _ = b;
}
