//! Incidents: the shared workflow plus severity, recordable and the investigation fields.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::incident::Severity;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn report_investigate_and_hide_from_trades() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");

    let id = w.ok(
        sup,
        "incident.create",
        json!({"project": p, "title": "Worker slipped on ramp", "kind": "Fall", "severity": "first_aid", "recordable": "true",
               "occurred": "2026-10-12", "people_involved": "J. Smith (laborer); witness R. Jones", "assignees": [pm.raw()]}),
    )["id"]
        .as_u64()
        .unwrap();
    let i = w.engine.state().incidents.iter().next().unwrap().clone();
    assert_eq!((i.severity, i.recordable, i.kind.as_str()), (Severity::FirstAid, true, "Fall"));
    w.err(sup, "incident.create", json!({"project": p, "title": "x", "severity": "bad"}));

    // Subcontractors have no access to incidents at all.
    w.err(sub, "incident.list", json!({"project": p}));
    w.err(sub, "incident.get", json!({"project": p, "id": id}));
    w.err(sub, "incident.create", json!({"project": p, "title": "x"}));

    w.ok(pm, "incident.update", json!({"project": p, "id": id, "root_cause": "Wet ramp, no grip strips", "corrective_action": "Strips installed"}));
    w.ok(pm, "incident.complete", json!({"project": p, "id": id, "comment": "Investigation done."}));
    w.ok(sup, "incident.close", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().incidents.iter().next().unwrap().root_cause, "Wet ramp, no grip strips");
    assert_eq!(w.ok(pm, "incident.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.assert_replay();
}

fn unread(w: &mut World, who: buildkiosk_core::Id<buildkiosk_model::directory::Person>) -> usize {
    w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len()
}

#[test]
fn a_serious_report_tells_the_tool_admins_once() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let (before_pm, before_sup) = (unread(&mut w, pm), unread(&mut w, sup));
    // Minor reports tell nobody.
    let minor = w.ok(sup, "incident.create", json!({"project": p, "title": "Near miss at hoist", "severity": "near_miss"}))["id"].as_u64().unwrap();
    assert_eq!(unread(&mut w, pm), before_pm);
    // A report that is serious from the start tells the admins, not the person who wrote it.
    w.ok(sup, "incident.create", json!({"project": p, "title": "Crushed hand", "severity": "medical_treatment"}));
    assert_eq!((unread(&mut w, pm), unread(&mut w, sup)), (before_pm + 1, before_sup));
    // Raising a report to serious later tells them once; more edits don't repeat it.
    w.ok(sup, "incident.update", json!({"project": p, "id": minor, "severity": "lost_time"}));
    assert_eq!(unread(&mut w, pm), before_pm + 2);
    w.ok(sup, "incident.update", json!({"project": p, "id": minor, "root_cause": "Rigging failed"}));
    w.ok(sup, "incident.update", json!({"project": p, "id": minor, "severity": "fatality"}));
    assert_eq!(unread(&mut w, pm), before_pm + 2);
    // Recordable counts as serious too, whatever the severity says.
    let rec = w.ok(sup, "incident.create", json!({"project": p, "title": "Strain", "severity": "first_aid"}))["id"].as_u64().unwrap();
    assert_eq!(unread(&mut w, pm), before_pm + 2);
    w.ok(sup, "incident.update", json!({"project": p, "id": rec, "recordable": true}));
    assert_eq!(unread(&mut w, pm), before_pm + 3);
    w.assert_replay();
}

#[test]
fn private_incidents_are_for_the_people_they_concern() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let hurt = w.member(p, "Hank", "Hurt", "Superintendent");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"incidents": "standard"}}));
    let open = w.ok(sup, "incident.create", json!({"project": p, "title": "Open one"}))["id"].as_u64().unwrap();
    let secret = w.ok(
        sup,
        "incident.create",
        json!({"project": p, "title": "Back injury", "private": true, "involved": [hurt.raw()], "people_involved": "H. Hurt"}),
    )["id"]
        .as_u64()
        .unwrap();
    let titles = |w: &mut World, who| -> Vec<String> {
        w.ok(who, "incident.list", json!({"project": p})).as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap().to_string()).collect()
    };
    // The author, the person it happened to and the tool admin see both; a colleague with access sees only the open one.
    assert_eq!(titles(&mut w, sup).len(), 2);
    assert_eq!(titles(&mut w, hurt).len(), 2);
    assert_eq!(titles(&mut w, pm).len(), 2);
    assert_eq!(titles(&mut w, eng), ["Open one"]);
    w.err(eng, "incident.get", json!({"project": p, "id": secret}));
    w.err(eng, "incident.comment", json!({"project": p, "id": secret, "body": "hi"}));
    w.ok(eng, "incident.get", json!({"project": p, "id": open}));
    // It stays out of the injury log, the "waiting on me" list and notifications of people who can't see it.
    w.ok(sup, "incident.update", json!({"project": p, "id": secret, "recordable": true}));
    assert_eq!(w.ok(eng, "incident.log", json!({"project": p, "year": 2026}))["recordable"], 0);
    assert_eq!(w.ok(pm, "incident.log", json!({"project": p, "year": 2026}))["recordable"], 1);
    // Making it public again shows it; people must exist, and the author can add someone to it.
    w.err(sup, "incident.update", json!({"project": p, "id": secret, "involved": [9999]}));
    w.ok(sup, "incident.update", json!({"project": p, "id": secret, "involved": [hurt.raw(), eng.raw()]}));
    assert_eq!(titles(&mut w, eng).len(), 2);
    w.ok(sup, "incident.update", json!({"project": p, "id": secret, "private": false, "involved": []}));
    assert_eq!(titles(&mut w, eng).len(), 2);
    w.assert_replay();
}

#[test]
fn the_injury_log_adds_up_the_year_and_the_rates() {
    use buildkiosk_engine::cmd::incident::rate;
    assert_eq!(rate(2, 200_000), Some("20.00".to_string()), "2 cases in 20,000 hours is 20 per 200,000");
    assert_eq!(rate(1, 1_234_560), Some("1.62".to_string()));
    assert_eq!(rate(0, 100), Some("0.00".to_string()));
    assert_eq!(rate(5, 0), None, "no hours, no rate");
    assert!(rate(u64::MAX, 1).is_some(), "huge inputs don't overflow");

    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"incidents": "read"}}));
    let add = |w: &mut World, title: &str, severity: &str, recordable: bool, on: &str| {
        w.ok(sup, "incident.create", json!({"project": p, "title": title, "severity": severity, "recordable": recordable, "occurred": on}));
    };
    add(&mut w, "Near miss", "near_miss", false, "2026-03-01");
    add(&mut w, "Laceration", "medical_treatment", true, "2026-04-02");
    add(&mut w, "Fall from ladder", "lost_time", true, "2026-06-15");
    add(&mut w, "Struck by", "first_aid", false, "2026-07-01");
    add(&mut w, "Last year's", "lost_time", true, "2025-12-30");

    // Without hours there are counts but no rates.
    let log = w.ok(eng, "incident.log", json!({"project": p, "year": 2026}));
    assert_eq!(
        (log["incidents"].as_u64(), log["recordable"].as_u64(), log["lost_time"].as_u64(), log["fatalities"].as_u64()),
        (Some(4), Some(2), Some(1), Some(0))
    );
    assert!(log["recordable_rate"].is_null() && log["lost_time_rate"].is_null());
    assert_eq!(log["recordable_cases"].as_array().unwrap().len(), 2);
    assert_eq!(log["by_severity"].as_array().unwrap().iter().find(|s| s["severity"] == "first_aid").unwrap()["count"], 1);

    // Other hours are set by an admin; approved timesheets count too (unapproved ones don't).
    w.err(eng, "incident.set_hours", json!({"project": p, "year": 2026, "hours": 1000}));
    w.err(pm, "incident.set_hours", json!({"project": p, "year": 2026, "hours": -1}));
    w.err(pm, "incident.set_hours", json!({"project": p, "year": 1500, "hours": 1}));
    w.ok(pm, "incident.set_hours", json!({"project": p, "year": 2026, "hours": 19_990}));
    w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "8"}));
    w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-13", "hours": "2"}));
    w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}));
    let first = w.engine.state().time_entries.iter().next().unwrap().id.raw();
    w.ok(pm, "timesheet.approve", json!({"project": p, "ids": [first]}));
    let log = w.ok(eng, "incident.log", json!({"project": p, "year": 2026}));
    assert_eq!(
        (log["hours"]["timesheets"].as_f64(), log["hours"]["other"].as_f64(), log["hours"]["total"].as_f64()),
        (Some(8.0), Some(19_990.0), Some(19_998.0))
    );
    // 2 recordable cases over 19,998 hours: 2 * 200,000 / 19,998 = 20.00; the one lost-time case: 10.00.
    assert_eq!((log["recordable_rate"].as_str(), log["lost_time_rate"].as_str()), (Some("20.00"), Some("10.00")));
    // Another year has its own figures, and setting hours to 0 removes them.
    assert_eq!(w.ok(eng, "incident.log", json!({"project": p, "year": 2025}))["recordable"], 1);
    w.ok(pm, "incident.set_hours", json!({"project": p, "year": 2026, "hours": 0}));
    assert_eq!(w.ok(eng, "incident.log", json!({"project": p, "year": 2026}))["hours"]["other"].as_f64(), Some(0.0));
    w.err(eng, "incident.log", json!({"project": p, "year": "soon"}));
    w.assert_replay();
}
