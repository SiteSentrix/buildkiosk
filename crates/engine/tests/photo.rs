//! Photos: pictures only, albums, ownership of edits.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

fn sha(c: char) -> String {
    c.to_string().repeat(64)
}

#[test]
fn add_edit_and_list() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let a = w.ok(sup, "photo.add", json!({"project": p, "sha256": sha('a'), "name": "IMG_1.jpg", "size": 1000, "mime": "image/jpeg", "caption": "Footing forms", "album": "Foundations", "location": "Grid C"}))["id"]
        .as_u64()
        .unwrap();
    w.ok(
        sup,
        "photo.add",
        json!({"project": p, "sha256": sha('b'), "name": "IMG_2.png", "size": 1000, "mime": "IMAGE/PNG", "album": "Roof", "taken": "2026-10-01"}),
    );
    assert_eq!(w.engine.state().photos.iter().find(|x| x.album == "Roof").unwrap().file.mime, "image/png");
    // Only pictures: no SVG (it can hold script), no HTML, no unknown types.
    for bad in ["image/svg+xml", "text/html", "application/pdf", ""] {
        w.err(sup, "photo.add", json!({"project": p, "sha256": sha('c'), "name": "x", "size": 1, "mime": bad}));
    }
    w.err(reader, "photo.add", json!({"project": p, "sha256": sha('d'), "name": "x", "size": 1, "mime": "image/gif"}));
    w.err(sup, "photo.add", json!({"project": p, "sha256": "zz", "name": "x", "size": 1, "mime": "image/gif"}));

    assert_eq!(w.ok(reader, "photo.list", json!({"project": p, "album": "foundations"})).as_array().unwrap().len(), 1);
    assert_eq!(w.ok(reader, "photo.list", json!({"project": p, "q": "grid c"})).as_array().unwrap().len(), 1);
    w.err(reader, "photo.update", json!({"project": p, "id": a, "caption": "x"}));
    let other = w.member(p, "Olga", "Other", "Project engineer");
    w.err(other, "photo.update", json!({"project": p, "id": a, "caption": "x"}));
    w.ok(sup, "photo.update", json!({"project": p, "id": a, "caption": "Footing forms, east"}));
    w.ok(pm, "photo.update", json!({"project": p, "id": a, "album": ""}));
    w.err(other, "photo.delete", json!({"project": p, "id": a}));
    w.ok(sup, "photo.delete", json!({"project": p, "id": a}));
    w.err(reader, "photo.list", json!({"project": p, "deleted": true}));
    w.ok(pm, "photo.restore", json!({"project": p, "id": a}));
    let o = w.project("Other");
    w.err(pm, "photo.get", json!({"project": o, "id": a}));
    w.assert_replay();
}
