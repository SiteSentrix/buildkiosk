//! Schedule: tasks, links, critical path, rolling up, baseline, import.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Date;
use buildkiosk_engine::sched;
use buildkiosk_testkit::World;
use serde_json::{Value, json};

fn d(s: &str) -> Date {
    Date::parse(s).unwrap()
}

fn row(list: &Value, number: u64) -> &Value {
    list.as_array().unwrap().iter().find(|r| r["number"] == number).unwrap()
}

#[test]
fn working_day_arithmetic() {
    assert_eq!(sched::shift(d("2026-10-12"), 2), Some(d("2026-10-14")));
    assert_eq!(sched::shift(d("2026-10-15"), 3), Some(d("2026-10-20")), "over a weekend");
    assert_eq!(sched::shift(d("2026-10-20"), -3), Some(d("2026-10-15")));
    assert_eq!(sched::days_inclusive(d("2026-10-15"), d("2026-10-20")), 4);
    assert_eq!(sched::days_inclusive(d("2026-10-12"), d("2026-10-12")), 1);
    assert_eq!(sched::days_inclusive(d("2026-10-17"), d("2026-10-18")), 0, "a weekend has no working days");
    assert_eq!(sched::days_inclusive(d("2026-10-20"), d("2026-10-15")), 0);
}

#[test]
fn links_critical_path_and_recalculate() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    w.ok(pm, "schedule.add", json!({"project": p, "name": "Excavate", "start": "2026-10-12", "days": 3}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Foundations", "start": "2026-10-15", "days": 4, "predecessors": "1"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Framing", "start": "2026-10-21", "days": 5, "predecessors": "2"}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Permit close-out", "start": "2026-10-12", "days": 2}));
    w.err(reader, "schedule.add", json!({"project": p, "name": "x", "start": "2026-10-12"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "finish": "2026-10-01"}));
    w.err(pm, "schedule.add", json!({"project": p, "name": "Bad", "start": "2026-10-12", "predecessors": "99"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": 1, "predecessors": "3"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": 2, "predecessors": "2"}));

    let list = w.ok(reader, "schedule.list", json!({"project": p, "today": null}));
    assert_eq!(row(&list, 2)["finish"], "2026-10-20");
    for n in [1, 2, 3] {
        assert_eq!((row(&list, n)["critical"].as_bool(), row(&list, n)["float"].as_i64()), (Some(true), Some(0)), "task {n}");
    }
    assert_eq!((row(&list, 4)["critical"].as_bool(), row(&list, 4)["float"].as_i64()), (Some(false), Some(10)));

    // Slip the first task: its successor now starts too early; recalculate pushes the chain.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 1, "days": 5}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!(row(&list, 2)["conflict"], true);
    let out = w.ok(pm, "schedule.recalculate", json!({"project": p}));
    assert_eq!(out["moved"], 2);
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert_eq!((row(&list, 2)["start"].as_str(), row(&list, 2)["finish"].as_str()), (Some("2026-10-19"), Some("2026-10-22")));
    assert_eq!((row(&list, 3)["start"].as_str(), row(&list, 3)["finish"].as_str()), (Some("2026-10-23"), Some("2026-10-29")));
    assert_eq!(row(&list, 2)["conflict"], false);

    // Lag and overlap.
    w.ok(pm, "schedule.update", json!({"project": p, "id": 3, "predecessors": "2+2"}));
    assert_eq!(w.ok(pm, "schedule.recalculate", json!({"project": p}))["moved"], 1);

    // Baseline and slip.
    w.err(eng, "schedule.baseline", json!({"project": p}));
    w.ok(pm, "schedule.baseline", json!({"project": p}));
    w.ok(pm, "schedule.update", json!({"project": p, "id": 4, "start": "2026-10-14"}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    assert!(row(&list, 4)["slip"].as_i64().unwrap() > 0);
    w.ok(pm, "schedule.progress", json!({"project": p, "id": 1, "percent": 100}));
    w.err(pm, "schedule.progress", json!({"project": p, "id": 1, "percent": 101}));

    w.err(eng, "schedule.delete", json!({"project": p, "id": 4}));
    w.ok(pm, "schedule.delete", json!({"project": p, "id": 4}));
    w.ok(pm, "schedule.restore", json!({"project": p, "id": 4}));
    let other = w.project("Other");
    w.err(pm, "schedule.get", json!({"project": other, "id": 1}));
    w.assert_replay();
}

#[test]
fn summary_tasks_roll_up_and_import() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let out = w.ok(pm, "schedule.add", json!({"project": p, "name": "Site work", "start": "2026-10-12", "days": 1}));
    let parent = out["id"].as_u64().unwrap();
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Clear", "start": "2026-10-12", "days": 2, "parent": parent, "percent": 100}));
    w.ok(pm, "schedule.add", json!({"project": p, "name": "Grade", "start": "2026-10-14", "days": 2, "parent": parent, "predecessors": "2"}));
    w.err(pm, "schedule.update", json!({"project": p, "id": parent, "parent": 3}));
    w.err(pm, "schedule.delete", json!({"project": p, "id": parent}));
    let list = w.ok(pm, "schedule.list", json!({"project": p}));
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        (rows[0]["summary"].as_bool(), rows[0]["start"].as_str(), rows[0]["finish"].as_str()),
        (Some(true), Some("2026-10-12"), Some("2026-10-15"))
    );
    assert_eq!(rows[0]["percent"], 50, "weighted by length");
    assert_eq!(rows[1]["depth"], 1);

    let imp = w.ok(
        pm,
        "schedule.import",
        json!({"project": p, "text": "id, name, start, finish, percent, predecessors, parent\nA, Mobilize, 2026-11-02, 2026-11-03, 0, ,\nB, Pour slab, 2026-11-04, 2026-11-06, 0, A;A, \nC, Cure, 2026-11-09, 2026-11-10, 25%, B+1, B"}),
    );
    assert_eq!(imp["added"], 3);
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, 2026-11-02, , , B\nB, Two, 2026-11-03, , , A"}));
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, not-a-date"}));
    w.err(pm, "schedule.import", json!({"project": p, "text": "A, One, 2026-11-02, , , Z"}));
    assert_eq!(w.ok(pm, "schedule.list", json!({"project": p})).as_array().unwrap().len(), 6);
    w.assert_replay();
}
