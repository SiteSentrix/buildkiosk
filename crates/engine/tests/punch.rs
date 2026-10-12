//! The punch list workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::punch::PunchStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn work_done_review_send_back_close_and_reopen() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let other = w.member(p, "Olga", "Other", "Subcontractor");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();

    let id = w.ok(pm, "punch.create", json!({"project": p, "title": "Touch up paint at door 114", "location": "Level 1 corridor", "priority": "high", "assignees": [sub, sub], "due": "2026-10-30"}))["id"].as_u64().unwrap();
    let get = |w: &World| w.engine.state().punch_items.iter().next().unwrap().clone();
    assert_eq!(get(&w).assignees, vec![sub], "assigned once");
    assert_eq!(get(&w).ball_in_court, vec![sub]);
    assert_eq!(unread(&mut w, sub), 1);

    // Only assignees and the manager mark work done; readers can't even comment.
    w.err(other, "punch.complete", json!({"project": p, "id": id}));
    w.err(reader, "punch.comment", json!({"project": p, "id": id, "body": "hi"}));
    w.ok(other, "punch.comment", json!({"project": p, "id": id, "body": "Is the lift available?"}));
    w.ok(sub, "punch.complete", json!({"project": p, "id": id, "comment": "Painted two coats."}));
    assert_eq!((get(&w).status, get(&w).ball_in_court.clone()), (PunchStatus::ReadyForReview, vec![pm]));
    assert_eq!(
        (unread(&mut w, pm), unread(&mut w, sub)),
        (1, 2),
        "the comment goes to the assignee, who has the ball; the manager hears the work is done"
    );
    w.err(sub, "punch.close", json!({"project": p, "id": id}));
    w.err(pm, "punch.send_back", json!({"project": p, "id": id}));
    w.ok(pm, "punch.send_back", json!({"project": p, "id": id, "comment": "Still see a patch near the frame."}));
    assert_eq!((get(&w).status, get(&w).comments.len()), (PunchStatus::Open, 3));
    w.ok(sub, "punch.complete", json!({"project": p, "id": id}));
    w.ok(pm, "punch.close", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, PunchStatus::Closed);
    w.err(sub, "punch.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(sub, "punch.comment", json!({"project": p, "id": id, "body": "late"}));
    w.err(sub, "punch.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "punch.reopen", json!({"project": p, "id": id, "comment": "Water stain returned."}));
    assert_eq!(get(&w).status, PunchStatus::Open);
    w.assert_replay();
}

#[test]
fn files_listing_and_isolation() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let id = w.ok(pm, "punch.create", json!({"project": p, "title": "Fix handrail", "assignees": [sub]}))["id"].as_u64().unwrap();
    w.ok(pm, "punch.create", json!({"project": p, "title": "Replace ceiling tile", "due": "2020-01-01"}));
    let file = json!({"project": p, "id": id, "sha256": "f".repeat(64), "name": "photo.jpg", "size": 10});
    w.ok(sub, "punch.attach", file.clone());
    w.ok(pm, "punch.attach", file.clone());
    assert_eq!(w.engine.state().punch_items.iter().next().unwrap().attachments.len(), 1);
    w.err(sub, "punch.update", json!({"project": p, "id": id, "title": "Mine now"}));
    w.ok(sub, "punch.detach", json!({"project": p, "id": id, "sha256": "f".repeat(64)}));

    assert_eq!(w.ok(sub, "punch.list", json!({"project": p, "mine": true})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(pm, "punch.list", json!({"project": p, "overdue": true}))[0]["number"], 2);
    assert_eq!(w.ok(pm, "punch.list", json!({"project": p, "q": "handrail"})).as_array().unwrap().len(), 1);
    let other = w.project("Other");
    w.err(pm, "punch.get", json!({"project": other, "id": id}));
    w.ok(pm, "punch.delete", json!({"project": p, "id": id}));
    w.err(sub, "punch.restore", json!({"project": p, "id": id}));
    w.ok(w.admin(), "punch.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}
