//! Timesheets: logging, the 24-hour day, submit, approve, send back, visibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::timesheet::TimeStatus;
use buildkiosk_testkit::World;
use serde_json::json;

fn status(w: &World, id: u64) -> TimeStatus {
    w.engine.state().time_entries.iter().find(|e| e.id.raw() == id).unwrap().status
}

#[test]
fn log_submit_approve_and_return() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let concrete = w.engine.state().cost_codes.iter().find(|c| c.code == "03-300").unwrap().id.raw();

    let a = w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "7.5", "cost_code": concrete, "description": "Footings"}))
        ["id"]
        .as_u64()
        .unwrap();
    let b = w.ok(sup, "timesheet.add", json!({"project": p, "date": "2026-10-13", "hours": 8}))["id"].as_u64().unwrap();
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "17"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "0"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "abc"}));
    w.err(sup, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "1", "person": eng.raw()}));
    w.ok(pm, "timesheet.add", json!({"project": p, "date": "2026-10-12", "hours": "4", "person": eng.raw()}));

    // Everyone sees their own; only admins see all.
    assert_eq!(w.ok(sup, "timesheet.list", json!({"project": p})).as_array().unwrap().len(), 2);
    assert_eq!(w.ok(pm, "timesheet.list", json!({"project": p})).as_array().unwrap().len(), 3);
    w.err(sup, "timesheet.list", json!({"project": p, "person": eng.raw()}));
    w.err(eng, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.ok(sup, "timesheet.update", json!({"project": p, "id": a, "hours": "8"}));
    w.err(sup, "timesheet.update", json!({"project": p, "id": a, "date": "2026-10-13", "hours": "17"}));

    w.err(sup, "timesheet.submit", json!({"project": p, "from": "2026-11-01"}));
    assert_eq!(w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}))["submitted"], 2);
    assert_eq!((status(&w, a), status(&w, b)), (TimeStatus::Submitted, TimeStatus::Submitted));
    w.err(sup, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.err(sup, "timesheet.approve", json!({"project": p, "ids": [a]}));

    w.ok(pm, "timesheet.return", json!({"project": p, "ids": [b], "note": "Wrong cost code"}));
    assert_eq!(status(&w, b), TimeStatus::Draft);
    assert_eq!(w.engine.state().time_entries.iter().find(|e| e.id.raw() == b).unwrap().returned_note, "Wrong cost code");
    w.ok(sup, "timesheet.update", json!({"project": p, "id": b, "cost_code": concrete}));
    w.ok(sup, "timesheet.submit", json!({"project": p, "from": "2026-10-12"}));
    assert_eq!(w.ok(pm, "timesheet.approve", json!({"project": p, "person": sup.raw(), "from": "2026-10-12", "to": "2026-10-18"}))["approved"], 2);
    assert_eq!(status(&w, a), TimeStatus::Approved);
    w.err(pm, "timesheet.approve", json!({"project": p, "ids": [a]}));
    w.err(pm, "timesheet.update", json!({"project": p, "id": a, "hours": "1"}));
    w.err(sup, "timesheet.delete", json!({"project": p, "id": a}));

    let other = w.project("Other");
    w.err(pm, "timesheet.approve", json!({"project": other, "ids": [a]}));
    w.assert_replay();
}
