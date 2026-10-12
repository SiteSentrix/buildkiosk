//! The RFI workflow and its permissions.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Date;
use buildkiosk_engine::EngineError;
use buildkiosk_model::rfi::RfiStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn full_lifecycle_with_ball_in_court() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let architect = w.member(p, "Arch", "Itect", "Owner / design team");
    let engineer = w.member(p, "Eli", "Engineer", "Owner / design team");

    let out = w.ok(pm, "rfi.create", json!({"project": p, "subject": "Footing depth at grid C4", "question": "Soil report says 4 ft; drawings say 3 ft. Which governs?", "assignees": [architect, engineer]}));
    assert_eq!(out["number"], 1);
    let id = out["id"].as_u64().unwrap();
    let rfi = w.engine.state().rfis.iter().next().unwrap().clone();
    assert_eq!(rfi.status, RfiStatus::Draft);
    assert_eq!(rfi.manager, Some(pm));
    assert!(rfi.ball_in_court.is_empty());

    // Opening sets the due date seven working days out (Monday 12 Oct + a minute → Wednesday 21 Oct).
    w.ok(pm, "rfi.open", json!({"project": p, "id": id}));
    let rfi = w.engine.state().rfis.iter().next().unwrap().clone();
    assert_eq!(rfi.status, RfiStatus::Open);
    assert_eq!(rfi.due, Date::parse("2026-10-21"));
    assert_eq!(rfi.ball_in_court, vec![architect, engineer]);

    w.ok(architect, "rfi.respond", json!({"project": p, "id": id, "body": "4 ft governs; revised detail to follow."}));
    assert_eq!(w.engine.state().rfis.iter().next().unwrap().ball_in_court, vec![engineer]);
    w.ok(engineer, "rfi.respond", json!({"project": p, "id": id, "body": "Agree with the architect."}));
    // Everyone answered: the manager has the ball.
    assert_eq!(w.engine.state().rfis.iter().next().unwrap().ball_in_court, vec![pm]);

    // Only the manager (or an admin) picks the official answer and closes.
    let e = w.err(architect, "rfi.mark_official", json!({"project": p, "id": id, "response": 1}));
    assert!(matches!(e, EngineError::Forbidden(_)), "{e}");
    w.ok(pm, "rfi.mark_official", json!({"project": p, "id": id, "response": 1}));
    w.ok(pm, "rfi.close", json!({"project": p, "id": id}));
    let rfi = w.engine.state().rfis.iter().next().unwrap().clone();
    assert_eq!(rfi.status, RfiStatus::Closed);
    assert_eq!(rfi.official_response().unwrap().id, 1);
    assert!(rfi.ball_in_court.is_empty());

    // Closed RFIs take no responses until reopened.
    let e = w.err(architect, "rfi.respond", json!({"project": p, "id": id, "body": "One more thing"}));
    assert!(matches!(e, EngineError::Conflict(_)), "{e}");
    w.ok(pm, "rfi.reopen", json!({"project": p, "id": id}));
    w.ok(architect, "rfi.respond", json!({"project": p, "id": id, "body": "One more thing"}));
    w.assert_replay();
}

#[test]
fn numbers_are_per_project_and_survive_deletion() {
    let mut w = World::new();
    let a = w.project("A");
    let b = w.project("B");
    let admin = w.admin();
    for _ in 0..3 {
        w.ok(admin, "rfi.create", json!({"project": a, "subject": "x"}));
    }
    let first_b = w.ok(admin, "rfi.create", json!({"project": b, "subject": "y"}));
    assert_eq!(first_b["number"], 1);
    w.ok(admin, "rfi.delete", json!({"project": a, "id": 3}));
    let next = w.ok(admin, "rfi.create", json!({"project": a, "subject": "z"}));
    assert_eq!(next["number"], 4, "deleted numbers aren't reused");
    w.ok(admin, "rfi.restore", json!({"project": a, "id": 3}));
    w.assert_replay();
}

#[test]
fn permissions_by_template() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let viewer = w.member(p, "Rita", "Reader", "Read only");
    let outsider = w.user("Otto", "Outsider");

    // Subcontractors (Standard) can raise RFIs; read-only members and outsiders can't.
    let out = w.ok(sub, "rfi.create", json!({"project": p, "subject": "Conduit route", "question": "Can we run it above the ceiling?"}));
    let id = out["id"].as_u64().unwrap();
    assert!(matches!(w.err(viewer, "rfi.create", json!({"project": p, "subject": "x"})), EngineError::Forbidden(_)));
    assert!(matches!(w.err(outsider, "rfi.list", json!({"project": p})), EngineError::Forbidden(_)));

    // A draft is visible to its author only (and admins).
    assert_eq!(w.ok(viewer, "rfi.list", json!({"project": p})).as_array().unwrap().len(), 0);
    assert_eq!(w.ok(sub, "rfi.list", json!({"project": p})).as_array().unwrap().len(), 1);
    assert!(matches!(w.err(viewer, "rfi.get", json!({"project": p, "id": id})), EngineError::NotFound(_)));

    // The author can't open without an assignee; the manager defaults to the author.
    let e = w.err(sub, "rfi.open", json!({"project": p, "id": id}));
    assert!(matches!(e, EngineError::Conflict(_)), "{e}");
    let admin = w.admin;
    w.ok(sub, "rfi.update", json!({"project": p, "id": id, "assignees": [admin]}));
    w.ok(sub, "rfi.open", json!({"project": p, "id": id}));
    assert_eq!(w.ok(viewer, "rfi.list", json!({"project": p})).as_array().unwrap().len(), 1);

    // Read-only members can't respond; non-assignees with Standard can't either.
    assert!(matches!(w.err(viewer, "rfi.respond", json!({"project": p, "id": id, "body": "hi"})), EngineError::Forbidden(_)));
    let sub2 = w.member(p, "Tia", "Trade", "Subcontractor");
    assert!(matches!(w.err(sub2, "rfi.respond", json!({"project": p, "id": id, "body": "hi"})), EngineError::Forbidden(_)));
    // Only admins delete open RFIs.
    assert!(matches!(w.err(sub, "rfi.delete", json!({"project": p, "id": id})), EngineError::Forbidden(_)));
    w.assert_replay();
}

#[test]
fn private_rfis_are_hidden_from_uninvolved_members() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Arch", "Itect", "Owner / design team");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let out = w.ok(
        pm,
        "rfi.create",
        json!({"project": p, "subject": "Owner budget question", "question": "?", "assignees": [arch], "private": true, "open": true}),
    );
    let id = out["id"].as_u64().unwrap();
    assert_eq!(w.ok(arch, "rfi.list", json!({"project": p})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(sub, "rfi.list", json!({"project": p})).as_array().unwrap().len(), 0);
    assert!(matches!(w.err(sub, "rfi.get", json!({"project": p, "id": id})), EngineError::NotFound(_)));
}

#[test]
fn ids_from_another_project_are_not_found() {
    let mut w = World::new();
    let a = w.project("A");
    let b = w.project("B");
    let admin = w.admin();
    let in_b = w.ok(admin, "rfi.create", json!({"project": b, "subject": "B's RFI"}));
    // A member of A only can't reach B's RFI through A.
    let sub = w.member(a, "Sam", "Sub", "Subcontractor");
    let e = w.err(sub, "rfi.update", json!({"project": a, "id": in_b["id"], "subject": "hijacked"}));
    assert!(matches!(e, EngineError::NotFound(_)), "{e}");
    let e = w.err(admin, "rfi.get", json!({"project": a, "id": in_b["id"]}));
    assert!(matches!(e, EngineError::NotFound(_)), "{e}");
}

#[test]
fn validation_errors_name_the_field() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let admin = w.admin();
    let e = w.err(admin, "rfi.create", json!({"project": p}));
    assert!(e.to_string().contains("`subject`"), "{e}");
    let e = w.err(admin, "rfi.create", json!({"project": p, "subject": "x", "due": "next week"}));
    assert!(e.to_string().contains("`due`"), "{e}");
    let e = w.err(admin, "rfi.create", json!({"project": p, "subject": "x", "assignees": [999]}));
    assert!(e.to_string().contains("999"), "{e}");
    let e = w.err(admin, "rfi.create", json!({"project": 42, "subject": "x"}));
    assert!(matches!(e, EngineError::NotFound(_)), "{e}");
    assert!(w.journal.entries.iter().all(|e| e.cmd != "rfi.create"), "failed commands aren't journaled");
}

#[test]
fn list_filters() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let admin = w.admin;
    w.ok(
        admin,
        "rfi.create",
        json!({"project": p, "subject": "Door hardware", "question": "q", "assignees": [admin], "open": true, "due": "2026-10-13"}),
    );
    w.ok(admin, "rfi.create", json!({"project": p, "subject": "Window sills"}));
    w.advance_days(3);
    let all = |w: &mut World, q: serde_json::Value| w.ok(admin, "rfi.list", q).as_array().unwrap().len();
    assert_eq!(all(&mut w, json!({"project": p})), 2);
    assert_eq!(all(&mut w, json!({"project": p, "status": "open"})), 1);
    assert_eq!(all(&mut w, json!({"project": p, "q": "window"})), 1);
    assert_eq!(all(&mut w, json!({"project": p, "mine": true})), 1);
    assert_eq!(all(&mut w, json!({"project": p, "overdue": true})), 1);
}

#[test]
fn attachments_follow_edit_rights() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let out = w.ok(pm, "rfi.create", json!({"project": p, "subject": "Door 114", "question": "Which hardware?", "assignees": [sub]}));
    let id = out["id"].as_u64().unwrap();
    let sha = "a".repeat(64);
    let file = json!({"project": p, "id": id, "sha256": sha, "name": "photo.jpg", "size": 1234, "mime": "image/jpeg"});
    // A draft is the manager's; the assignee can't see it, let alone attach.
    w.err(sub, "rfi.attach", file.clone());
    w.ok(pm, "rfi.attach", file.clone());
    w.ok(pm, "rfi.attach", file.clone());
    assert_eq!(w.engine.state().rfis.iter().next().unwrap().attachments.len(), 1, "same file twice is one attachment");
    w.err(pm, "rfi.attach", json!({"project": p, "id": id, "sha256": "xyz", "name": "a", "size": 1}));
    w.ok(pm, "rfi.open", json!({"project": p, "id": id}));
    let other = json!({"project": p, "id": id, "sha256": "b".repeat(64), "name": "sketch.pdf", "size": 10});
    w.ok(sub, "rfi.attach", other);
    w.ok(sub, "rfi.detach", json!({"project": p, "id": id, "sha256": "b".repeat(64)}));
    w.err(sub, "rfi.detach", json!({"project": p, "id": id, "sha256": "b".repeat(64)}));
    w.ok(pm, "rfi.close", json!({"project": p, "id": id}));
    w.err(sub, "rfi.attach", file);
    w.assert_replay();
}
