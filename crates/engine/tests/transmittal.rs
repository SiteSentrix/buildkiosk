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

const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn items_link_to_documents_and_drawings_and_note_what_went_out() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let other = w.project("Harbor Office");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let doc = w.ok(
        pm,
        "document.add",
        json!({"project": p, "title": "Geotech report", "sha256": SHA, "name": "geo.pdf", "size": 10, "mime": "application/pdf"}),
    )["id"]
        .as_u64()
        .unwrap();
    let foreign_doc =
        w.ok(pm, "document.add", json!({"project": other, "title": "Elsewhere", "sha256": SHA, "name": "e.pdf", "size": 10}))["id"].as_u64().unwrap();
    let dwg = w.ok(
        pm,
        "drawing.add",
        json!({"project": p, "number": "A-101", "title": "Floor plan", "revision": "0", "sha256": SHA, "name": "a.pdf", "size": 10}),
    )["id"]
        .as_u64()
        .unwrap();
    let t = w
        .ok(pm, "transmittal.create", json!({"project": p, "subject": "Permit set", "to": [arch.raw()], "items": [{"document": doc, "copies": 1}, {"drawing": dwg, "copies": 3}, {"description": "Samples", "copies": 2}]}))["id"]
        .as_u64()
        .unwrap();
    let items = |w: &World| w.engine.state().transmittals.get(buildkiosk_core::Id::new(t)).unwrap().items.clone();
    let got = items(&w);
    // A blank description is filled from the thing linked, and the version going out is noted.
    assert_eq!((got[0].description.as_str(), got[0].revision.as_str()), ("Geotech report", "v1"));
    assert_eq!((got[1].description.as_str(), got[1].revision.as_str()), ("A-101 Floor plan", "rev 0"));
    assert_eq!((got[2].document, got[2].drawing, got[2].revision.as_str()), (None, None, ""));
    // Links must exist in this project.
    w.err(pm, "transmittal.update", json!({"project": p, "id": t, "items": [{"document": foreign_doc}]}));
    w.err(pm, "transmittal.update", json!({"project": p, "id": t, "items": [{"document": 999}]}));
    w.err(pm, "transmittal.update", json!({"project": p, "id": t, "items": [{"drawing": 999}]}));
    // New versions arrive before it is sent: what is sent is what is current at that moment.
    w.ok(pm, "document.add_version", json!({"project": p, "id": doc, "sha256": SHA_B, "name": "geo2.pdf", "size": 11}));
    w.ok(pm, "drawing.add_revision", json!({"project": p, "id": dwg, "revision": "A", "sha256": SHA_B, "name": "a2.pdf", "size": 11}));
    w.ok(pm, "transmittal.send", json!({"project": p, "id": t}));
    assert_eq!((items(&w)[0].revision.clone(), items(&w)[1].revision.clone()), ("v2".to_string(), "rev A".to_string()));
    // Later versions don't rewrite history, even when an admin corrects the text.
    w.ok(pm, "document.add_version", json!({"project": p, "id": doc, "sha256": SHA, "name": "geo3.pdf", "size": 12}));
    w.ok(pm, "transmittal.update", json!({"project": p, "id": t, "items": [{"description": "Geotech report (final)", "document": doc, "copies": 1}, {"drawing": dwg, "copies": 3}, {"description": "Samples", "copies": 2}]}));
    assert_eq!((items(&w)[0].description.clone(), items(&w)[0].revision.clone()), ("Geotech report (final)".to_string(), "v2".to_string()));
    w.assert_replay();
}

#[test]
fn recipients_acknowledge_receipt() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let copy = w.member(p, "Cy", "Copied", "Read only");
    let stranger = w.member(p, "Sol", "Stranger", "Read only");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();
    let t = w.ok(
        eng,
        "transmittal.create",
        json!({"project": p, "subject": "Bulletin 4", "to": [arch.raw()], "cc": [copy.raw()], "items": [{"description": "Bulletin"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let receipts = |w: &World| w.engine.state().transmittals.get(buildkiosk_core::Id::new(t)).unwrap().receipts.clone();
    // Nothing to acknowledge until it is sent.
    w.err(arch, "transmittal.acknowledge", json!({"project": p, "id": t}));
    w.ok(eng, "transmittal.send", json!({"project": p, "id": t}));
    let before = unread(&mut w, eng);
    // Only the people it was sent to, each once; the sender is told.
    w.err(stranger, "transmittal.acknowledge", json!({"project": p, "id": t}));
    w.ok(arch, "transmittal.acknowledge", json!({"project": p, "id": t, "note": "Printed for the trailer"}));
    assert_eq!((receipts(&w).len(), receipts(&w)[0].person, receipts(&w)[0].note.clone()), (1, arch, "Printed for the trailer".to_string()));
    assert_eq!(unread(&mut w, eng), before + 1);
    w.err(arch, "transmittal.acknowledge", json!({"project": p, "id": t}));
    // The sender or an admin records receipt for someone who told them by phone; others can't.
    w.err(arch, "transmittal.acknowledge", json!({"project": p, "id": t, "person": copy.raw()}));
    w.err(eng, "transmittal.acknowledge", json!({"project": p, "id": t, "person": stranger.raw()}));
    w.ok(eng, "transmittal.acknowledge", json!({"project": p, "id": t, "person": copy.raw(), "note": "By phone"}));
    assert_eq!(receipts(&w).len(), 2);
    assert_eq!(receipts(&w)[1].stamp.by, Some(eng), "who recorded it");
    // An admin can too.
    let t2 = w.ok(eng, "transmittal.create", json!({"project": p, "subject": "Two", "to": [arch.raw()], "items": [{"description": "x"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(eng, "transmittal.send", json!({"project": p, "id": t2}));
    w.ok(pm, "transmittal.acknowledge", json!({"project": p, "id": t2, "person": arch.raw()}));
    w.err(pm, "transmittal.acknowledge", json!({"project": p, "id": t2, "person": 9999}));
    w.assert_replay();
}
