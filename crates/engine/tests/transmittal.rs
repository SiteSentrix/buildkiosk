//! Transmittals: drafts, sending, visibility, files.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

const SHA: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

#[test]
fn draft_send_and_visibility() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let t = w.ok(
        eng,
        "transmittal.create",
        json!({"project": p, "subject": "Permit set", "purpose": "for_review", "to": [arch.raw()], "cc": [reader.raw()], "due": "2026-11-01",
               "items": [{"description": "Full-size drawings", "copies": 3}, {"description": "", "copies": ""}], "private": true}),
    )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(w.engine.state().transmittals.iter().next().unwrap().items.len(), 1, "blank rows are dropped");
    w.err(eng, "transmittal.create", json!({"project": p}));

    // A draft is private to its author and admins.
    w.err(arch, "transmittal.get", json!({"project": p, "id": t}));
    w.ok(pm, "transmittal.get", json!({"project": p, "id": t}));
    w.err(pm, "transmittal.update", json!({"project": p, "id": t, "subject": ""}));
    w.err(eng, "transmittal.send", json!({"project": p, "id": w.engine.state().transmittals.iter().next().unwrap().id.raw() + 99}));
    w.ok(eng, "transmittal.attach", json!({"project": p, "id": t, "sha256": SHA, "name": "set.pdf", "size": 5}));
    w.ok(eng, "transmittal.update", json!({"project": p, "id": t, "to": []}));
    w.err(eng, "transmittal.send", json!({"project": p, "id": t}));
    w.ok(eng, "transmittal.update", json!({"project": p, "id": t, "to": [arch.raw()]}));
    w.ok(eng, "transmittal.send", json!({"project": p, "id": t}));
    w.err(eng, "transmittal.send", json!({"project": p, "id": t}));
    w.err(eng, "transmittal.update", json!({"project": p, "id": t, "subject": "Changed"}));
    w.err(eng, "transmittal.attach", json!({"project": p, "id": t, "sha256": "b".repeat(64), "name": "x", "size": 1}));

    // Recipients are notified and, for a private transmittal, are the only others who see it.
    let notes: Vec<_> = w.engine.state().notifications.iter().filter(|n| n.person == arch).collect();
    assert_eq!(notes.len(), 1);
    assert!(notes[0].text.contains("Transmittal #1"));
    w.ok(arch, "transmittal.get", json!({"project": p, "id": t}));
    w.ok(reader, "transmittal.get", json!({"project": p, "id": t}));
    let outsider = w.member(p, "Olive", "Outsider", "Read only");
    w.err(outsider, "transmittal.get", json!({"project": p, "id": t}));
    assert!(w.ok(outsider, "transmittal.list", json!({"project": p})).as_array().unwrap().is_empty());
    assert_eq!(w.ok(arch, "transmittal.list", json!({"project": p, "q": "full-size"})).as_array().unwrap().len(), 1);

    w.err(eng, "transmittal.delete", json!({"project": p, "id": t}));
    w.ok(pm, "transmittal.delete", json!({"project": p, "id": t}));
    w.ok(pm, "transmittal.restore", json!({"project": p, "id": t}));
    let other = w.project("Other");
    w.err(pm, "transmittal.get", json!({"project": other, "id": t}));
    w.assert_replay();
}
