//! Daily logs: one per day, author-owned drafts, admin-only changes after submitting.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Date;
use buildkiosk_model::daily_log::LogStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn draft_edit_submit_and_reopen() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let out = w.ok(
        sup,
        "daily_log.create",
        json!({
            "project": p, "date": "2026-10-12", "sky": "rain", "high": 54, "low": "41",
            "work_performed": "Poured footings at grid C.",
            "manpower": [
                {"company": "Northline Electric", "trade": "Electrical", "workers": 4, "hours": "7.5"},
                {"company": "", "workers": "", "hours": ""},
                {"company": "Keystone Concrete", "workers": 6, "hours": 8}
            ],
            "equipment": [{"name": "Excavator", "hours": "5.5"}, {"name": ""}],
            "deliveries": [{"description": "Rebar bundles", "company": "Steel Supply"}],
            "visitors": [{"name": "City inspector", "purpose": "Footing inspection"}]
        }),
    );
    let id = out["id"].as_u64().unwrap();
    let log = w.engine.state().daily_logs.iter().next().unwrap().clone();
    assert_eq!((log.manpower.len(), log.equipment.len(), log.weather.low), (2, 1, Some(41)), "blank rows are dropped");
    assert_eq!(log.headcount(), (10, 4 * 75 + 6 * 80));
    assert_eq!(log.date, Date::parse("2026-10-12").unwrap());

    // Drafts are private to the author and admins; one log per day.
    w.err(reader, "daily_log.get", json!({"project": p, "id": id}));
    w.err(sub, "daily_log.update", json!({"project": p, "id": id, "notes": "x"}));
    w.err(sup, "daily_log.create", json!({"project": p, "date": "2026-10-12"}));
    w.err(sup, "daily_log.update", json!({"project": p, "id": id, "manpower": [{"company": "A", "workers": 2, "hours": "30"}]}));
    w.ok(sup, "daily_log.update", json!({"project": p, "id": id, "notes": "Left early for rain.", "sky": "overcast"}));

    w.ok(sup, "daily_log.submit", json!({"project": p, "id": id}));
    w.err(sup, "daily_log.submit", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().daily_logs.iter().next().unwrap().status, LogStatus::Submitted);
    w.ok(reader, "daily_log.get", json!({"project": p, "id": id}));
    w.err(sup, "daily_log.update", json!({"project": p, "id": id, "notes": "changed"}));
    w.err(sup, "daily_log.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "daily_log.update", json!({"project": p, "id": id, "notes": "Admin fix."}));
    w.ok(pm, "daily_log.reopen", json!({"project": p, "id": id}));
    w.ok(sup, "daily_log.update", json!({"project": p, "id": id, "notes": "Back to a draft."}));

    // Lists respect dates and visibility; ids of other projects are not found.
    let other = w.project("Other");
    w.err(pm, "daily_log.get", json!({"project": other, "id": id}));
    let l = w.ok(pm, "daily_log.list", json!({"project": p, "from": "2026-10-13"}));
    assert!(l.as_array().unwrap().is_empty());
    w.ok(sup, "daily_log.delete", json!({"project": p, "id": id}));
    w.ok(w.admin(), "daily_log.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}

#[test]
fn copy_previous_and_submit_notifies_admins() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Eli", "Engineer", "Project engineer");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();

    let first = w.ok(sup, "daily_log.create", json!({"project": p, "date": "2026-10-12", "manpower": [{"company": "Keystone", "workers": 6, "hours": 8}], "equipment": [{"name": "Excavator", "hours": 5}], "deliveries": [{"description": "Rebar"}]}))["id"].as_u64().unwrap();
    w.ok(sup, "daily_log.submit", json!({"project": p, "id": first}));
    assert_eq!(unread(&mut w, pm), 1, "the project manager is told it is in");
    assert_eq!(unread(&mut w, sup), 0, "not the person who submitted");

    let next = w.ok(sup, "daily_log.create", json!({"project": p, "date": "2026-10-13", "copy_previous": true, "work_performed": "More"}))["id"]
        .as_u64()
        .unwrap();
    let log = w.engine.state().daily_logs.iter().find(|l| l.id.raw() == next).unwrap().clone();
    assert_eq!((log.manpower.len(), log.equipment.len(), log.deliveries.len()), (1, 1, 0), "crews and equipment come along, deliveries don't");
    let earlier = w.ok(sup, "daily_log.create", json!({"project": p, "date": "2026-10-01", "copy_previous": true}))["id"].as_u64().unwrap();
    assert!(w.engine.state().daily_logs.iter().find(|l| l.id.raw() == earlier).unwrap().manpower.is_empty(), "nothing earlier to copy");
    w.assert_replay();
}
