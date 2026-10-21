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

fn names(run: &serde_json::Value) -> Vec<String> {
    run["rows"].as_array().unwrap().iter().map(|r| r["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn company_reports_choose_columns_and_respect_visibility() {
    let mut w = World::new();
    let admin = w.admin();
    let a = w.project("Riverside Clinic");
    let b = w.project("Harbor Office");
    let c = w.project("Annex");
    w.ok(admin, "project.update", json!({"project": b, "stage": "closeout"}));
    let pm = w.member(a, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(admin, "member.add", json!({"project": b, "person": pm, "template": template}));
    let sub = w.member(a, "Sam", "Sub", "Subcontractor");
    let code = w.engine.state().cost_codes.iter().find(|x| x.code == "03-300").unwrap().id.raw();
    w.ok(pm, "budget.import", json!({"project": a, "text": "03-300, 100000"}));
    w.ok(pm, "budget.import", json!({"project": b, "text": "03-300, 50000"}));
    w.ok(pm, "direct_cost.create", json!({"project": a, "cost_code": code, "amount": "30000", "description": "Pour", "date": "2026-10-01"}));
    for n in 0..3 {
        w.ok(pm, "task.create", json!({"project": a, "title": format!("Task {n}")}));
    }
    w.ok(pm, "task.create", json!({"project": b, "title": "One task"}));
    w.ok(pm, "rfi.create", json!({"project": a, "subject": "Footing", "question": "Depth?"}));
    let _ = c;

    // The chosen columns come back in the table's order, with a totals row; projects sort by a column either way round.
    w.err(pm, "company_report.run", json!({"columns": "revised, nonsense"}));
    w.err(pm, "company_report.run", json!({"columns": ""}));
    let run = w.ok(pm, "company_report.run", json!({"columns": ["variance", "revised", "open", "rfis"], "sort": "open", "descending": true}));
    let labels: Vec<&str> = run["columns"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap()).collect();
    assert_eq!(labels, ["open", "rfis", "revised", "variance"]);
    assert_eq!(names(&run), ["Riverside Clinic", "Harbor Office"], "Annex isn't one of Paula's projects; most open items first");
    let first = &run["rows"][0]["cells"];
    assert_eq!(
        (first[0].as_u64(), first[1].as_u64(), first[2].as_str()),
        (Some(3), Some(0), Some("100000.00")),
        "three tasks; the new RFI is still a draft"
    );
    assert_eq!(run["totals"][0].as_u64(), Some(4));
    assert_eq!(run["totals"][2].as_str(), Some("150000.00"));
    let asc = w.ok(pm, "company_report.run", json!({"columns": "open", "sort": "open"}));
    assert_eq!(names(&asc), ["Harbor Office", "Riverside Clinic"]);
    // A stage limits the projects; a project with nothing to show in the sort column goes last either way.
    let closeout = w.ok(pm, "company_report.run", json!({"columns": "stage,completion", "stage": "closeout"}));
    assert_eq!(names(&closeout), ["Harbor Office"]);
    assert_eq!(closeout["rows"][0]["cells"][0], "Closeout");
    assert!(closeout["rows"][0]["cells"][1].is_null() || closeout["rows"][0]["cells"][1].as_str().is_none(), "no completion date");
    let by_date = w.ok(pm, "company_report.run", json!({"columns": "completion", "sort": "completion", "descending": true}));
    assert_eq!(names(&by_date).len(), 2);
    // Someone who can't read the budget sees blanks, not zeros.
    let blind = w.ok(sub, "company_report.run", json!({"columns": "open,revised"}));
    assert_eq!(names(&blind), ["Riverside Clinic"]);
    assert!(blind["rows"][0]["cells"][1].is_null(), "{blind}");
    assert!(blind["totals"][1].is_null());

    // Saved reports: yours, shared ones, and who may change them.
    w.err(admin, "company_report.save", json!({"name": "x", "columns": ""}));
    w.err(pm, "company_report.save", json!({"name": "Bad sort", "columns": "open", "sort": "revised"}));
    w.err(pm, "company_report.save", json!({"name": "Bad stage", "columns": "open", "stage": "someday"}));
    let mine = w.ok(pm, "company_report.save", json!({"name": "Budget watch", "columns": "revised,variance,open", "sort": "variance"}))["id"]
        .as_u64()
        .unwrap();
    let shared = w.ok(pm, "company_report.save", json!({"name": "Everyone", "columns": "stage,open", "shared": true}))["id"].as_u64().unwrap();
    w.err(pm, "company_report.save", json!({"name": "budget WATCH", "columns": "open"}));
    assert_eq!(w.ok(pm, "company_report.list", json!({})).as_array().unwrap().len(), 2);
    assert_eq!(w.ok(sub, "company_report.list", json!({})).as_array().unwrap().len(), 1, "only the shared one");
    w.err(sub, "company_report.run", json!({"id": mine}));
    assert_eq!(names(&w.ok(sub, "company_report.run", json!({"id": shared}))), ["Riverside Clinic"]);
    assert_eq!(names(&w.ok(pm, "company_report.run", json!({"id": mine}))).len(), 2);
    assert_eq!(w.ok(admin, "company_report.list", json!({})).as_array().unwrap().len(), 2, "a company admin sees them all");
    w.err(sub, "company_report.save", json!({"id": shared, "name": "Mine now", "columns": "open"}));
    w.err(sub, "company_report.delete", json!({"id": shared}));
    w.ok(pm, "company_report.save", json!({"id": shared, "name": "Everyone", "columns": "stage,open,overdue", "shared": true}));
    w.ok(admin, "company_report.delete", json!({"id": mine}));
    w.err(pm, "company_report.run", json!({"id": mine}));
    w.ok(pm, "company_report.delete", json!({"id": shared}));
    w.assert_replay();
}

#[test]
fn rfi_ageing_columns_count_days_open_and_past_due() {
    let mut w = World::new();
    let a = w.project("Riverside Clinic");
    let pm = w.member(a, "Paula", "Manager", "Project manager");
    let sub = w.member(a, "Sam", "Sub", "Subcontractor");
    let make = |w: &mut World, subject: &str, due: Option<&str>| {
        let mut params = json!({"project": a, "subject": subject, "question": "?", "assignees": [pm.raw()]});
        if let Some(d) = due {
            params["due"] = json!(d);
        }
        let id = w.ok(pm, "rfi.create", params)["id"].as_u64().unwrap();
        w.ok(pm, "rfi.open", json!({"project": a, "id": id}));
        id
    };
    // Today is the world's clock; one RFI is made now, another ten days later, one of them with a past due date.
    make(&mut w, "Old one", Some("2026-01-01"));
    w.advance_days(10);
    make(&mut w, "Newer one", None);
    let run = w.ok(pm, "company_report.run", json!({"columns": ["rfis", "rfis_overdue", "rfi_oldest", "rfi_age"]}));
    let cells = &run["rows"][0]["cells"];
    assert_eq!((cells[0].as_u64(), cells[1].as_u64(), cells[2].as_u64(), cells[3].as_u64()), (Some(2), Some(1), Some(10), Some(5)), "{run}");
    // Counts add up across projects; ages (the oldest, the average) do not.
    assert_eq!(run["totals"][1].as_u64(), Some(1));
    assert!(run["totals"][2].is_null() && run["totals"][3].is_null(), "{run}");
    // A project with no open RFIs has nothing to average; someone who can't read RFIs sees blanks, not zeros.
    let b = w.project("Annex");
    w.ok(w.admin(), "member.add", json!({"project": b, "person": pm, "template": buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap()}));
    let run = w.ok(pm, "company_report.run", json!({"columns": ["rfi_oldest", "rfi_age", "rfis_overdue"], "sort": "rfi_oldest", "descending": true}));
    assert_eq!(names(&run), ["Riverside Clinic", "Annex"], "{run}");
    assert!(run["rows"][1]["cells"][1].is_null() && run["rows"][1]["cells"][2].is_null(), "{run}");
    assert_eq!(run["rows"][1]["cells"][0].as_u64(), Some(0), "columns come back in the table's order");
    w.ok(pm, "member.update", json!({"project": a, "person": sub, "overrides": {"rfis": "none"}}));
    let blind = w.ok(sub, "company_report.run", json!({"columns": ["rfis_overdue", "rfi_oldest"]}));
    assert!(blind["rows"][0]["cells"][0].is_null() && blind["rows"][0]["cells"][1].is_null(), "{blind}");
    w.assert_replay();
}

#[test]
fn delivery_days_follow_the_rhythm() {
    use buildkiosk_core::{Date, Id};
    use buildkiosk_model::saved_report::{DeliveryRhythm, ReportDelivery};
    let d = |s: &str| Date::parse(s).unwrap();
    let make = |rhythm, weekday, day, last: Option<&str>| ReportDelivery {
        rhythm,
        weekday,
        day,
        recipients: vec![Id::new(2)],
        last_sent: last.map(|l| Date::parse(l).unwrap()),
    };
    // Weekly on Wednesday (2): the latest Wednesday on or before today.
    let weekly = make(DeliveryRhythm::Weekly, 2, 1, None);
    assert_eq!(weekly.scheduled_on(d("2026-10-14")), Some(d("2026-10-14")), "a Wednesday is its own day");
    assert_eq!(weekly.scheduled_on(d("2026-10-16")), Some(d("2026-10-14")));
    assert_eq!(weekly.scheduled_on(d("2026-10-13")), Some(d("2026-10-07")));
    // Monthly on the 15th: this month's from the 15th on, last month's before it (across a year end too).
    let monthly = make(DeliveryRhythm::Monthly, 0, 15, None);
    assert_eq!(monthly.scheduled_on(d("2026-10-15")), Some(d("2026-10-15")));
    assert_eq!(monthly.scheduled_on(d("2026-10-31")), Some(d("2026-10-15")));
    assert_eq!(monthly.scheduled_on(d("2027-01-03")), Some(d("2026-12-15")));
    assert_eq!(make(DeliveryRhythm::Daily, 0, 1, None).scheduled_on(d("2026-10-12")), Some(d("2026-10-12")));
    // Due means the latest scheduled day is after the last send; a send on the day settles it, and a late pass still sends.
    assert!(make(DeliveryRhythm::Weekly, 2, 1, Some("2026-10-07")).due(d("2026-10-14")));
    assert!(!make(DeliveryRhythm::Weekly, 2, 1, Some("2026-10-14")).due(d("2026-10-16")));
    assert!(make(DeliveryRhythm::Weekly, 2, 1, Some("2026-10-14")).due(d("2026-10-21")));
    assert!(make(DeliveryRhythm::Monthly, 0, 15, Some("2026-09-15")).due(d("2026-10-20")), "caught up late");
    assert!(!make(DeliveryRhythm::Monthly, 0, 15, Some("2026-10-15")).due(d("2026-10-20")));
    assert!(make(DeliveryRhythm::Daily, 0, 1, Some("2026-10-11")).due(d("2026-10-12")));
    // With no one to send it to, nothing is due.
    let mut nobody = make(DeliveryRhythm::Daily, 0, 1, None);
    nobody.recipients.clear();
    assert!(!nobody.due(d("2026-10-12")));
}

#[test]
fn emailing_a_saved_report_on_a_schedule() {
    use buildkiosk_engine::Actor;
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sam = w.member(p, "Sam", "Sub", "Subcontractor");
    let admin = w.admin();
    let private = w.ok(pm, "company_report.save", json!({"name": "My open items", "columns": ["open", "tasks"]}))["id"].as_u64().unwrap();
    let shared = w.ok(pm, "company_report.save", json!({"name": "Everyone", "columns": ["open"], "shared": true}))["id"].as_u64().unwrap();

    // Only the owner or a company admin schedules; recipients are active users with an email; a private report goes to its owner and company admins only.
    w.err(sam, "company_report.schedule", json!({"id": shared, "rhythm": "daily", "people": [sam.raw()]}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "hourly", "people": [sam.raw()]}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "daily", "people": []}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "daily"}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "weekly", "weekday": "someday", "people": [sam.raw()]}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "monthly", "day": 31, "people": [sam.raw()]}));
    w.err(pm, "company_report.schedule", json!({"id": shared, "rhythm": "daily", "people": [9999]}));
    w.err(pm, "company_report.schedule", json!({"id": private, "rhythm": "daily", "people": [sam.raw()]}));
    w.ok(pm, "company_report.schedule", json!({"id": private, "rhythm": "daily", "people": [pm.raw()]}));
    // A company admin may schedule anyone's report.
    w.ok(admin, "company_report.schedule", json!({"id": private, "rhythm": "off"}));
    w.ok(pm, "company_report.schedule", json!({"id": shared, "rhythm": "weekly", "weekday": "Monday", "people": [sam.raw(), pm.raw(), sam.raw()]}));
    let list = w.ok(pm, "company_report.list", json!({}));
    let report = list.as_array().unwrap().iter().find(|r| r["name"] == "Everyone").unwrap();
    assert_eq!(report["delivery"]["recipients"].as_array().unwrap().len(), 2, "a person named twice is one");

    // Nothing is due on the day it is set (the first email is the next Monday), and only the system asks.
    w.err(pm, "company_report.due", json!({}));
    assert_eq!(w.ok(Actor::System, "company_report.due", json!({})), json!([]));
    w.advance_days(7);
    let due = w.ok(Actor::System, "company_report.due", json!({}));
    assert_eq!(due.as_array().unwrap().len(), 1);
    assert_eq!((due[0]["name"].clone(), due[0]["recipients"].as_array().unwrap().len()), (json!("Everyone"), 2));
    assert!(due[0]["recipients"][0]["email"].as_str().unwrap().contains('@'));
    // Recording the send settles it until the next Monday; a person who left isn't emailed.
    w.err(pm, "company_report.mark_sent", json!({"id": shared}));
    w.err(Actor::System, "company_report.mark_sent", json!({"id": private}));
    w.ok(Actor::System, "company_report.mark_sent", json!({"id": shared}));
    assert_eq!(w.ok(Actor::System, "company_report.due", json!({})), json!([]));
    w.advance_days(7);
    w.ok(admin, "person.update", json!({"id": sam.raw(), "active": false}));
    let due = w.ok(Actor::System, "company_report.due", json!({}));
    assert_eq!(due[0]["recipients"].as_array().unwrap().len(), 1);
    // Editing the report keeps its schedule; turning it off clears it; a deleted report is not due.
    w.ok(pm, "company_report.save", json!({"id": shared, "name": "Everyone", "columns": ["open", "overdue"], "shared": true}));
    assert_eq!(w.ok(Actor::System, "company_report.due", json!({})).as_array().unwrap().len(), 1);
    w.ok(pm, "company_report.delete", json!({"id": shared}));
    assert_eq!(w.ok(Actor::System, "company_report.due", json!({})), json!([]));
    w.assert_replay();
}

#[test]
fn company_reports_cover_more_tools() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    w.ok(pm, "task.create", json!({"project": p, "title": "Order hardware"}));
    w.ok(pm, "task.create", json!({"project": p, "title": "File permit"}));
    w.ok(pm, "budget.import", json!({"project": p, "text": "03-300, 5000"}));
    let all = [
        "tasks",
        "observations",
        "inspections",
        "incidents",
        "meeting_actions",
        "correspondence",
        "transmittals",
        "forms",
        "schedule_open",
        "commitments",
        "change_events",
        "hours_waiting",
        "original",
    ];
    let run = w.ok(pm, "company_report.run", json!({"columns": all}));
    let labels: Vec<&str> = run["columns"].as_array().unwrap().iter().map(|c| c["label"].as_str().unwrap()).collect();
    assert_eq!(labels.len(), all.len(), "{labels:?}");
    let row = run["rows"].as_array().unwrap().iter().find(|r| r["name"] == "Riverside Clinic").unwrap();
    let cells = row["cells"].as_array().unwrap();
    let at = |key: &str| {
        let i = run["columns"].as_array().unwrap().iter().position(|c| c["key"] == key).unwrap();
        cells[i].clone()
    };
    assert_eq!(at("tasks"), json!(2));
    assert_eq!(at("original"), json!("5000.00"));
    // A subcontractor has no financial tools, so those columns are blank for them; counts of the tools they read still show.
    let theirs = w.ok(sub, "company_report.run", json!({"columns": ["tasks", "original", "commitments"]}));
    let row = theirs["rows"].as_array().unwrap().iter().find(|r| r["name"] == "Riverside Clinic").unwrap();
    assert_eq!(row["cells"][0], json!(2));
    assert_eq!((row["cells"][1].clone(), row["cells"][2].clone()), (json!(null), json!(null)));
    w.assert_replay();
}
