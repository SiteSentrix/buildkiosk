//! Documents: folders, versions, private folders, ownership of edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn folders_documents_and_versions() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let f = w.ok(sup, "folder.create", json!({"project": p, "name": "Contracts"}))["id"].as_u64().unwrap();
    w.err(sup, "folder.create", json!({"project": p, "name": "contracts"}));
    w.err(reader, "folder.create", json!({"project": p, "name": "Mine"}));
    let sub = w.ok(sup, "folder.create", json!({"project": p, "name": "Signed", "parent": f}))["id"].as_u64().unwrap();

    let d = w.ok(
        sup,
        "document.add",
        json!({"project": p, "folder": sub, "sha256": SHA_A, "name": "agreement.pdf", "size": 1200, "mime": "application/pdf"}),
    )["id"]
        .as_u64()
        .unwrap();
    let doc = w.ok(reader, "document.get", json!({"project": p, "id": d}));
    assert_eq!(doc["title"], "agreement.pdf");
    assert_eq!(doc["versions"].as_array().unwrap().len(), 1);

    w.err(sup, "document.add_version", json!({"project": p, "id": d, "sha256": SHA_A, "name": "agreement.pdf", "size": 1200}));
    w.err(sup, "document.add_version", json!({"project": p, "id": d, "sha256": "xyz", "name": "a.pdf", "size": 1}));
    w.err(reader, "document.add_version", json!({"project": p, "id": d, "sha256": SHA_B, "name": "agreement.pdf", "size": 1300}));
    let v = w.ok(
        pm,
        "document.add_version",
        json!({"project": p, "id": d, "sha256": SHA_B, "name": "agreement-v2.pdf", "size": 1300, "note": "Signed copy"}),
    );
    assert_eq!(v["version"], 2);

    // Only the uploader or an admin edits; anyone with Standard uploads versions.
    w.err(reader, "document.update", json!({"project": p, "id": d, "title": "x"}));
    w.ok(sup, "document.update", json!({"project": p, "id": d, "title": "Owner agreement", "folder": ""}));
    assert!(w.engine.state().documents.iter().next().unwrap().folder.is_none());
    w.err(sup, "folder.delete", json!({"project": p, "id": f}));
    w.ok(pm, "folder.delete", json!({"project": p, "id": sub}));
    w.ok(pm, "folder.delete", json!({"project": p, "id": f}));
    w.err(sup, "document.add", json!({"project": p, "folder": f, "sha256": SHA_A, "name": "a", "size": 1}));

    let found = w.ok(reader, "document.list", json!({"project": p, "q": "OWNER"}));
    assert_eq!(found.as_array().unwrap().len(), 1);
    w.ok(sup, "document.delete", json!({"project": p, "id": d}));
    assert!(w.ok(reader, "document.list", json!({"project": p})).as_array().unwrap().is_empty());
    w.err(reader, "document.list", json!({"project": p, "deleted": true}));
    w.err(sup, "document.restore", json!({"project": p, "id": d}));
    w.ok(pm, "document.restore", json!({"project": p, "id": d}));

    // Other projects' ids are not found.
    let other = w.project("Other");
    w.err(pm, "document.get", json!({"project": other, "id": d}));
    w.assert_replay();
}

#[test]
fn private_folders_hide_their_contents() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");

    w.err(sup, "folder.create", json!({"project": p, "name": "Bids", "private": true}));
    let f = w.ok(pm, "folder.create", json!({"project": p, "name": "Bids", "private": true, "access": [sup.raw()]}))["id"].as_u64().unwrap();
    let inner = w.ok(pm, "folder.create", json!({"project": p, "name": "Final", "parent": f}))["id"].as_u64().unwrap();
    let d =
        w.ok(sup, "document.add", json!({"project": p, "folder": inner, "sha256": SHA_A, "name": "bid.xlsx", "size": 10}))["id"].as_u64().unwrap();

    // Listed people and admins see it; others get "not found" and don't see it listed.
    w.ok(sup, "document.get", json!({"project": p, "id": d}));
    w.err(sub, "document.get", json!({"project": p, "id": d}));
    w.err(sub, "document.add", json!({"project": p, "folder": f, "sha256": SHA_B, "name": "x", "size": 1}));
    assert!(w.ok(sub, "document.list", json!({"project": p})).as_array().unwrap().is_empty());
    assert_eq!(w.ok(sup, "folder.list", json!({"project": p})).as_array().unwrap().len(), 2);
    assert!(w.ok(sub, "folder.list", json!({"project": p})).as_array().unwrap().is_empty());

    // Access changes are admin-only.
    w.err(sup, "folder.update", json!({"project": p, "id": f, "private": false}));
    w.ok(pm, "folder.update", json!({"project": p, "id": f, "access": [sup.raw(), sub.raw()]}));
    w.ok(sub, "document.get", json!({"project": p, "id": d}));
    w.assert_replay();
}
