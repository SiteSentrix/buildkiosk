//! Drawings: the sheet log, revisions, import.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn log_revisions_and_import() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let d = w.ok(
        sup,
        "drawing.add",
        json!({"project": p, "number": "A-101", "title": "First floor plan", "discipline": "Architectural", "area": "Level 1",
               "revision": "0", "date": "2026-08-01", "set": "Permit set", "sha256": SHA_A, "name": "A-101.pdf", "size": 900, "mime": "application/pdf"}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(sup, "drawing.add", json!({"project": p, "number": "a-101"}));
    w.err(reader, "drawing.add", json!({"project": p, "number": "A-102"}));
    w.err(sup, "drawing.add", json!({"project": p, "number": "A-103", "discipline": "nonsense"}));
    w.err(sup, "drawing.add", json!({"project": p, "number": "A-103", "revision": "0", "sha256": "short", "name": "x", "size": 1}));

    w.err(sup, "drawing.add_revision", json!({"project": p, "id": d, "revision": "0"}));
    w.err(reader, "drawing.add_revision", json!({"project": p, "id": d, "revision": "A"}));
    w.ok(sup, "drawing.add_revision", json!({"project": p, "id": d, "revision": "A", "set": "Bulletin 1", "note": "Door 104 moved", "sha256": SHA_B, "name": "A-101-A.pdf", "size": 910}));
    let got = w.ok(reader, "drawing.get", json!({"project": p, "id": d}));
    assert_eq!(got["revisions"].as_array().unwrap().len(), 2);
    assert_eq!(got["revisions"][1]["label"], "A");

    // Import a log: a header line, duplicates and comments are skipped; bad rows are refused whole.
    let out = w.ok(
        sup,
        "drawing.import",
        json!({"project": p, "text": "number, title, discipline, area\n# note\nA-101, Duplicate\nS-201, Foundation plan, structural, Level 0\nM-301\tHVAC plan\tmechanical\tRoof"}),
    );
    assert_eq!((out["added"].as_u64(), out["skipped"].as_u64()), (Some(2), Some(1)));
    w.err(sup, "drawing.import", json!({"project": p, "text": "E-1, Panel schedule, wiring"}));
    assert_eq!(w.ok(reader, "drawing.list", json!({"project": p})).as_array().unwrap().len(), 3);
    let l = w.ok(reader, "drawing.list", json!({"project": p, "discipline": "mechanical"}));
    assert_eq!(l[0]["number"], "M-301");
    assert_eq!(w.ok(reader, "drawing.list", json!({"project": p, "q": "foundation"})).as_array().unwrap().len(), 1);

    w.ok(sup, "drawing.update", json!({"project": p, "id": d, "title": "Level 1 plan", "area": "Level 1 east"}));
    w.err(sup, "drawing.update", json!({"project": p, "id": d, "number": "S-201"}));
    w.err(sup, "drawing.delete", json!({"project": p, "id": d}));
    w.ok(pm, "drawing.delete", json!({"project": p, "id": d}));
    w.err(reader, "drawing.list", json!({"project": p, "deleted": true}));
    w.ok(pm, "drawing.restore", json!({"project": p, "id": d}));

    let other = w.project("Other");
    w.err(pm, "drawing.get", json!({"project": other, "id": d}));
    w.assert_replay();
}
