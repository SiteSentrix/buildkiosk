//! Meetings: agenda, attendance, action items, minutes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn plan_hold_issue_and_follow_up() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let m = w.ok(
        eng,
        "meeting.create",
        json!({"project": p, "title": "Weekly coordination", "date": "2026-10-20", "category": "Coordination", "time": "9:00 AM", "location": "Site trailer",
               "invitees": [pm.raw(), arch.raw(), arch.raw()],
               "agenda": [{"title": "Look-ahead", "notes": ""}, {"title": "", "notes": ""}, {"title": "RFIs", "notes": "Door hardware"}],
               "actions": [{"description": "Confirm steel delivery", "assignee": arch.raw(), "due": "2026-10-27"}, {"description": ""}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let meeting = w.engine.state().meetings.iter().next().unwrap().clone();
    assert_eq!((meeting.attendees.len(), meeting.agenda.len(), meeting.actions.len()), (2, 2, 1), "duplicates and blank rows are dropped");
    w.err(eng, "meeting.create", json!({"project": p, "date": "2026-10-20"}));
    w.err(eng, "meeting.create", json!({"project": p, "title": "x"}));
    w.err(reader, "meeting.create", json!({"project": p, "title": "x", "date": "2026-10-20"}));

    // Edit while scheduled: author or admin; record who came; attendance survives a changed invite list.
    w.err(arch, "meeting.update", json!({"project": p, "id": m, "title": "x"}));
    w.ok(eng, "meeting.update", json!({"project": p, "id": m, "present": [arch.raw()], "minutes": "All agreed."}));
    w.ok(eng, "meeting.update", json!({"project": p, "id": m, "invitees": [arch.raw(), pm.raw(), eng.raw()]}));
    let a = w.engine.state().meetings.iter().next().unwrap().attendees.clone();
    assert!(a.iter().find(|x| x.person == arch).unwrap().present);
    assert_eq!(a.len(), 3);

    // The assignee ticks their own action off; others can't.
    w.err(reader, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": true}));
    w.err(pm, "meeting.set_action", json!({"project": p, "id": m, "index": 5, "done": true}));
    w.ok(arch, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": true}));
    w.ok(eng, "meeting.set_action", json!({"project": p, "id": m, "index": 0, "done": false}));

    // Issuing notifies attendees, and the people with open actions get the longer message.
    w.err(arch, "meeting.issue", json!({"project": p, "id": m}));
    w.ok(eng, "meeting.issue", json!({"project": p, "id": m}));
    w.err(eng, "meeting.issue", json!({"project": p, "id": m}));
    w.err(eng, "meeting.update", json!({"project": p, "id": m, "title": "Changed"}));
    let note = w.engine.state().notifications.iter().find(|n| n.person == arch).unwrap().text.clone();
    assert!(note.contains("Meeting #1") && note.contains("you have an action"), "{note}");
    assert!(w.engine.state().notifications.iter().any(|n| n.person == pm));
    w.err(eng, "meeting.reopen", json!({"project": p, "id": m}));
    w.ok(pm, "meeting.reopen", json!({"project": p, "id": m}));

    assert_eq!(w.ok(reader, "meeting.list", json!({"project": p, "q": "steel"})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(reader, "meeting.list", json!({"project": p, "status": "issued"})).as_array().unwrap().len(), 0);
    w.ok(eng, "meeting.attach", json!({"project": p, "id": m, "sha256": "f".repeat(64), "name": "agenda.pdf", "size": 3}));
    w.ok(eng, "meeting.detach", json!({"project": p, "id": m, "sha256": "f".repeat(64)}));
    w.ok(eng, "meeting.delete", json!({"project": p, "id": m}));
    w.ok(pm, "meeting.restore", json!({"project": p, "id": m}));
    let other = w.project("Other");
    w.err(pm, "meeting.get", json!({"project": other, "id": m}));
    w.assert_replay();
}
