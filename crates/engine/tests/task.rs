//! The tasks workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::task::TaskStatus;
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

    let id = w.ok(pm, "task.create", json!({"project": p, "title": "Order door hardware", "location": "Level 1 corridor", "priority": "high", "assignees": [sub, sub], "due": "2026-10-30"}))["id"].as_u64().unwrap();
    let get = |w: &World| w.engine.state().tasks.iter().next().unwrap().clone();
    assert_eq!(get(&w).assignees, vec![sub], "assigned once");
    assert_eq!(get(&w).ball_in_court, vec![sub]);
    assert_eq!(unread(&mut w, sub), 1);

    // Only assignees and the manager mark work done; readers can't even comment.
    w.err(other, "task.complete", json!({"project": p, "id": id}));
    w.err(reader, "task.comment", json!({"project": p, "id": id, "body": "hi"}));
    w.ok(other, "task.comment", json!({"project": p, "id": id, "body": "Is the lift available?"}));
    w.ok(sub, "task.complete", json!({"project": p, "id": id, "comment": "Painted two coats."}));
    assert_eq!((get(&w).status, get(&w).ball_in_court.clone()), (TaskStatus::ReadyForReview, vec![pm]));
    assert_eq!(
        (unread(&mut w, pm), unread(&mut w, sub)),
        (1, 2),
        "the comment goes to the assignee, who has the ball; the manager hears the work is done"
    );
    w.err(sub, "task.close", json!({"project": p, "id": id}));
    w.err(pm, "task.send_back", json!({"project": p, "id": id}));
    w.ok(pm, "task.send_back", json!({"project": p, "id": id, "comment": "Still see a patch near the frame."}));
    assert_eq!((get(&w).status, get(&w).comments.len()), (TaskStatus::Open, 3));
    w.ok(sub, "task.complete", json!({"project": p, "id": id}));
    w.ok(pm, "task.close", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, TaskStatus::Closed);
    w.err(sub, "task.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(sub, "task.comment", json!({"project": p, "id": id, "body": "late"}));
    w.err(sub, "task.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "task.reopen", json!({"project": p, "id": id, "comment": "Water stain returned."}));
    assert_eq!(get(&w).status, TaskStatus::Open);
    w.assert_replay();
}

#[test]
fn task_files_and_isolation() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let id = w.ok(pm, "task.create", json!({"project": p, "title": "Collect warranties"}))["id"].as_u64().unwrap();
    w.ok(pm, "task.attach", json!({"project": p, "id": id, "sha256": "f".repeat(64), "name": "list.xlsx", "size": 10}));
    let other = w.project("Other");
    w.err(pm, "task.get", json!({"project": other, "id": id}));
    assert_eq!(w.ok(pm, "task.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.assert_replay();
}
