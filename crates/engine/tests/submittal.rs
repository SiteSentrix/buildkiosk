//! The submittal workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::submittal::{Decision, SubmittalStatus};
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn review_rounds_and_ball_in_court() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Arch", "Itect", "Owner / design team");
    let eng = w.member(p, "Eli", "Engineer", "Owner / design team");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");

    let out = w.ok(
        pm,
        "submittal.create",
        json!({"project": p, "title": "Rooftop unit shop drawings", "spec_section": "23 74 00", "reviewers": [arch, eng]}),
    );
    let id = out["id"].as_u64().unwrap();
    assert_eq!(out["number"], 1);
    let get = |w: &World| w.engine.state().submittals.iter().next().unwrap().clone();
    assert_eq!(get(&w).status, SubmittalStatus::Draft);
    w.err(sub, "submittal.get", json!({"project": p, "id": id}));
    w.err(pm, "submittal.review", json!({"project": p, "id": id, "decision": "approved"}));

    w.ok(pm, "submittal.open", json!({"project": p, "id": id}));
    let s = get(&w);
    assert_eq!(s.status, SubmittalStatus::InReview);
    assert_eq!(s.ball_in_court, vec![arch, eng]);
    assert!(s.due.is_some());

    // Only reviewers review, once each.
    w.err(sub, "submittal.review", json!({"project": p, "id": id, "decision": "approved"}));
    w.err(arch, "submittal.review", json!({"project": p, "id": id}));
    w.ok(arch, "submittal.review", json!({"project": p, "id": id, "decision": "revise_resubmit", "comment": "Curb detail missing"}));
    w.err(arch, "submittal.review", json!({"project": p, "id": id, "decision": "approved"}));
    assert_eq!(get(&w).ball_in_court, vec![eng]);
    w.ok(eng, "submittal.review", json!({"project": p, "id": id, "decision": "approved_as_noted"}));
    assert_eq!(get(&w).ball_in_court, vec![pm], "everyone answered: back to the manager");

    // A resubmittal starts revision 1 with a clean slate of reviews.
    w.err(arch, "submittal.resubmit", json!({"project": p, "id": id}));
    w.ok(pm, "submittal.resubmit", json!({"project": p, "id": id}));
    let s = get(&w);
    assert_eq!((s.revision, s.reviews.len(), s.ball_in_court.len()), (1, 2, 2));
    w.ok(arch, "submittal.review", json!({"project": p, "id": id, "decision": "approved"}));
    w.ok(eng, "submittal.review", json!({"project": p, "id": id, "decision": "approved"}));
    w.err(eng, "submittal.close", json!({"project": p, "id": id, "decision": "approved"}));
    w.err(pm, "submittal.close", json!({"project": p, "id": id}));
    w.ok(pm, "submittal.close", json!({"project": p, "id": id, "decision": "approved"}));
    let s = get(&w);
    assert_eq!((s.status, s.final_decision), (SubmittalStatus::Closed, Some(Decision::Approved)));
    assert!(s.ball_in_court.is_empty());
    w.err(arch, "submittal.update", json!({"project": p, "id": id, "title": "x"}));
    w.assert_replay();
}

#[test]
fn drafts_need_reviewers_and_files_follow_edit_rights() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Arch", "Itect", "Owner / design team");
    let id = w.ok(pm, "submittal.create", json!({"project": p, "title": "Curtain wall"}))["id"].as_u64().unwrap();
    w.err(pm, "submittal.open", json!({"project": p, "id": id}));
    w.ok(pm, "submittal.update", json!({"project": p, "id": id, "reviewers": [arch]}));
    let file = json!({"project": p, "id": id, "sha256": "c".repeat(64), "name": "cw.pdf", "size": 10});
    w.ok(pm, "submittal.attach", file.clone());
    w.ok(pm, "submittal.attach", file.clone());
    assert_eq!(w.engine.state().submittals.iter().next().unwrap().attachments.len(), 1);
    w.ok(pm, "submittal.open", json!({"project": p, "id": id}));
    w.ok(arch, "submittal.attach", json!({"project": p, "id": id, "sha256": "d".repeat(64), "name": "note.pdf", "size": 5}));
    w.ok(arch, "submittal.detach", json!({"project": p, "id": id, "sha256": "d".repeat(64)}));
    // Another project's id answers "not found".
    let other = w.project("Other");
    w.err(pm, "submittal.get", json!({"project": other, "id": id}));
    w.ok(pm, "submittal.delete", json!({"project": p, "id": id}));
    w.ok(w.admin(), "submittal.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}
