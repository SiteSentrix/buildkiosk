//! The observations workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::observation::ObservationStatus;
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

    let id = w.ok(pm, "observation.create", json!({"project": p, "title": "Touch up paint at door 114", "location": "Level 1 corridor", "priority": "high", "assignees": [sub, sub], "due": "2026-10-30"}))["id"].as_u64().unwrap();
    let get = |w: &World| w.engine.state().observations.iter().next().unwrap().clone();
    assert_eq!(get(&w).assignees, vec![sub], "assigned once");
    assert_eq!(get(&w).ball_in_court, vec![sub]);
    assert_eq!(unread(&mut w, sub), 1);

    // Only assignees and the manager mark work done; readers can't even comment.
    w.err(other, "observation.complete", json!({"project": p, "id": id}));
    w.err(reader, "observation.comment", json!({"project": p, "id": id, "body": "hi"}));
    w.ok(other, "observation.comment", json!({"project": p, "id": id, "body": "Is the lift available?"}));
    w.ok(sub, "observation.complete", json!({"project": p, "id": id, "comment": "Painted two coats."}));
    assert_eq!((get(&w).status, get(&w).ball_in_court), (ObservationStatus::ReadyForReview, vec![pm]));
    assert_eq!(
        (unread(&mut w, pm), unread(&mut w, sub)),
        (1, 2),
        "the comment goes to the assignee, who has the ball; the manager hears the work is done"
    );
    w.err(sub, "observation.close", json!({"project": p, "id": id}));
    w.err(pm, "observation.send_back", json!({"project": p, "id": id}));
    w.ok(pm, "observation.send_back", json!({"project": p, "id": id, "comment": "Still see a patch near the frame."}));
    assert_eq!((get(&w).status, get(&w).comments.len()), (ObservationStatus::Open, 3));
    w.ok(sub, "observation.complete", json!({"project": p, "id": id}));
    w.ok(pm, "observation.close", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, ObservationStatus::Closed);
    w.err(sub, "observation.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(sub, "observation.comment", json!({"project": p, "id": id, "body": "late"}));
    w.err(sub, "observation.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "observation.reopen", json!({"project": p, "id": id, "comment": "Water stain returned."}));
    assert_eq!(get(&w).status, ObservationStatus::Open);
    w.assert_replay();
}

#[test]
fn files_listing_and_isolation() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let id = w.ok(pm, "observation.create", json!({"project": p, "title": "Fix handrail", "assignees": [sub]}))["id"].as_u64().unwrap();
    w.ok(pm, "observation.create", json!({"project": p, "title": "Replace ceiling tile", "due": "2020-01-01"}));
    let file = json!({"project": p, "id": id, "sha256": "f".repeat(64), "name": "photo.jpg", "size": 10});
    w.ok(sub, "observation.attach", file.clone());
    w.ok(pm, "observation.attach", file.clone());
    assert_eq!(w.engine.state().observations.iter().next().unwrap().attachments.len(), 1);
    w.err(sub, "observation.update", json!({"project": p, "id": id, "title": "Mine now"}));
    w.ok(sub, "observation.detach", json!({"project": p, "id": id, "sha256": "f".repeat(64)}));

    assert_eq!(w.ok(sub, "observation.list", json!({"project": p, "mine": true})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(pm, "observation.list", json!({"project": p, "overdue": true}))[0]["number"], 2);
    assert_eq!(w.ok(pm, "observation.list", json!({"project": p, "q": "handrail"})).as_array().unwrap().len(), 1);
    let other = w.project("Other");
    w.err(pm, "observation.get", json!({"project": other, "id": id}));
    w.ok(pm, "observation.delete", json!({"project": p, "id": id}));
    w.err(sub, "observation.restore", json!({"project": p, "id": id}));
    w.ok(w.admin(), "observation.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}

#[test]
fn observed_date_defaults_to_today_and_corrective_action_is_kept() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let id = w.ok(
        pm,
        "observation.create",
        json!({"project": p, "title": "Unguarded floor opening", "kind": "Safety", "corrective_action": "Install a cover and sign."}),
    )["id"]
        .as_u64()
        .unwrap();
    let o = w.engine.state().observations.iter().next().unwrap().clone();
    assert_eq!(o.observed, Some(w.now().date_at(0)));
    assert_eq!(o.corrective_action, "Install a cover and sign.");
    w.ok(pm, "observation.update", json!({"project": p, "id": id, "observed": "2026-10-09", "corrective_action": "Cover installed."}));
    let o = w.engine.state().observations.iter().next().unwrap().clone();
    assert_eq!((o.observed.unwrap().to_string(), o.corrective_action.as_str()), ("2026-10-09".to_string(), "Cover installed."));
    w.err(pm, "observation.update", json!({"project": p, "id": id, "observed": "not-a-date"}));
    w.assert_replay();
}
