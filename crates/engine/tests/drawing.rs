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

#[test]
fn publishing_tells_people_and_items_name_their_sheet() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Arch", "Itect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();
    let group = w.ok(w.admin(), "group.create", json!({"name": "Design team", "members": [arch, eng]}))["id"].as_u64().unwrap();

    let id = w.ok(pm, "drawing.add", json!({"project": p, "number": "A-101", "title": "Floor plan"}))["id"].as_u64().unwrap();
    w.err(pm, "drawing.publish", json!({"project": p, "id": id, "people": [eng]}));
    w.ok(pm, "drawing.add_revision", json!({"project": p, "id": id, "revision": "B", "set": "Bulletin 2"}));
    w.err(pm, "drawing.publish", json!({"project": p, "id": id}));
    w.err(reader, "drawing.publish", json!({"project": p, "id": id, "people": [eng]}));
    let out = w.ok(pm, "drawing.publish", json!({"project": p, "id": id, "people": [eng], "groups": [group], "message": "Door swing changed"}));
    assert_eq!(out["told"], 2, "named people and the group, nobody twice");
    assert_eq!((unread(&mut w, eng), unread(&mut w, arch), unread(&mut w, pm)), (1, 1, 0));
    w.err(pm, "drawing.publish", json!({"project": p, "id": id, "groups": [9999]}));

    let punch = w.ok(pm, "punch.create", json!({"project": p, "title": "Door swing", "drawing_number": "A-101"}))["id"].as_u64().unwrap();
    let item = w.engine.state().punch_items.iter().find(|i| i.id.raw() == punch).unwrap().clone();
    assert_eq!(item.drawing_number, "A-101");
    w.ok(pm, "punch.update", json!({"project": p, "id": punch, "drawing_number": ""}));
    assert_eq!(w.engine.state().punch_items.iter().next().unwrap().drawing_number, "");
    w.assert_replay();
}

#[test]
fn markups_on_a_revision() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let id = w.ok(pm, "drawing.add", json!({"project": p, "number": "A-101", "revision": "A"}))["id"].as_u64().unwrap();
    w.ok(pm, "drawing.add_revision", json!({"project": p, "id": id, "revision": "B"}));
    let rev = |w: &World, label: &str| w.engine.state().drawings.iter().next().unwrap().revisions.iter().find(|r| r.label == label).unwrap().clone();

    let pin = w.ok(eng, "drawing.markup_add", json!({"project": p, "id": id, "kind": "pin", "x": 2500, "y": 4000, "text": "Door swing?"}))["markup"]
        .as_u64()
        .unwrap();
    w.ok(
        eng,
        "drawing.markup_add",
        json!({"project": p, "id": id, "revision": "a", "page": 2, "kind": "rect", "x": 100, "y": 100, "x2": 900, "y2": 700}),
    );
    assert_eq!((rev(&w, "B").markups.len(), rev(&w, "A").markups.len()), (1, 1), "the newest revision by default");
    assert_eq!(rev(&w, "A").markups[0].page, 2);

    for bad in [
        json!({"kind": "pin", "x": 10001, "y": 0}),
        json!({"kind": "pin", "y": 5}),
        json!({"kind": "arrow", "x": 1, "y": 1}),
        json!({"kind": "text", "x": 1, "y": 1, "text": "  "}),
        json!({"kind": "squiggle", "x": 1, "y": 1}),
        json!({"kind": "pin", "x": 1, "y": 1, "revision": "Z"}),
        json!({"kind": "pin", "x": 1, "y": 1, "page": 0}),
    ] {
        let mut params = bad.clone();
        params["project"] = json!(p);
        params["id"] = json!(id);
        w.err(eng, "drawing.markup_add", params);
    }
    w.err(reader, "drawing.markup_add", json!({"project": p, "id": id, "kind": "pin", "x": 1, "y": 1}));

    w.ok(pm, "drawing.markup_resolve", json!({"project": p, "id": id, "markup": pin}));
    assert!(rev(&w, "B").markups[0].resolved);
    w.ok(pm, "drawing.markup_resolve", json!({"project": p, "id": id, "markup": pin, "resolved": false}));
    assert!(!rev(&w, "B").markups[0].resolved);
    w.err(pm, "drawing.markup_resolve", json!({"project": p, "id": id, "markup": 99}));

    // Only the author or an admin removes one.
    w.err(reader, "drawing.markup_delete", json!({"project": p, "id": id, "markup": pin}));
    let other = w.ok(pm, "drawing.markup_add", json!({"project": p, "id": id, "kind": "pin", "x": 5, "y": 5}))["markup"].as_u64().unwrap();
    w.err(eng, "drawing.markup_delete", json!({"project": p, "id": id, "markup": other}));
    w.ok(eng, "drawing.markup_delete", json!({"project": p, "id": id, "markup": pin}));
    w.ok(pm, "drawing.markup_delete", json!({"project": p, "id": id, "markup": other}));
    assert!(rev(&w, "B").markups.is_empty());
    w.assert_replay();
}

#[test]
fn measuring_against_a_pages_scale() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let id = w.ok(pm, "drawing.add", json!({"project": p, "id": 0, "number": "A-101", "revision": "A"}))["id"].as_u64().unwrap();
    let rev = |w: &World| w.engine.state().drawings.iter().next().unwrap().revisions[0].clone();
    let base = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        v
    };

    // No scale, no measuring.
    let dim = |x2: u32| base(json!({"kind": "dim", "x": 0, "y": 0, "x2": x2, "y2": 0}));
    w.err(pm, "drawing.markup_add", dim(5000));

    // A scale: the stretch across the middle of a square page is 10 feet.
    let scale = base(json!({"x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "10", "unit": "ft"}));
    w.err(reader, "drawing.scale_set", scale.clone());
    w.ok(pm, "drawing.scale_set", scale);
    for bad in [
        json!({"length": "0"}),
        json!({"length": "1.2345"}),
        json!({"length": "ten"}),
        json!({"unit": "yd"}),
        json!({"aspect": 10}),
        json!({"x2": 2510}),
        json!({"x": 20000}),
        json!({"page": 0}),
        json!({"revision": "Z"}),
    ] {
        let mut params = base(json!({"x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "10", "unit": "ft"}));
        for (k, v) in bad.as_object().unwrap() {
            params[k] = v.clone();
        }
        w.err(pm, "drawing.scale_set", params);
    }

    // Lengths and areas come out of the scale.
    let len = w.ok(pm, "drawing.markup_add", dim(2500))["markup"].as_u64().unwrap();
    let area = w.ok(pm, "drawing.markup_add", base(json!({"kind": "area", "x": 0, "y": 0, "x2": 5000, "y2": 2500})))["markup"].as_u64().unwrap();
    let m = |w: &World, n: u64| rev(w).markups.iter().find(|m| u64::from(m.id) == n).unwrap().clone();
    assert_eq!(m(&w, len).measure_text().as_deref(), Some("5.00 ft"));
    assert_eq!(m(&w, area).measure_text().as_deref(), Some("50.0 sq ft"));
    // Another page has no scale yet.
    w.err(pm, "drawing.markup_add", base(json!({"kind": "dim", "page": 2, "x": 0, "y": 0, "x2": 100, "y2": 0})));
    // A length needs its far end.
    w.err(pm, "drawing.markup_add", base(json!({"kind": "dim", "x": 0, "y": 0})));

    // A new scale (the same stretch is now 20 metres) works the measurements out again.
    w.ok(pm, "drawing.scale_set", base(json!({"x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "20", "unit": "m"})));
    assert_eq!(rev(&w).scales.len(), 1, "one scale per page");
    assert_eq!(m(&w, len).measure_text().as_deref(), Some("10.00 m"));
    assert_eq!(m(&w, area).measure_text().as_deref(), Some("200.00 sq m"));

    // Without a scale the lines stay but the values go.
    w.err(reader, "drawing.scale_clear", base(json!({})));
    w.ok(pm, "drawing.scale_clear", base(json!({})));
    assert_eq!((m(&w, len).measure, m(&w, len).measure_text()), (0, None));
    w.err(pm, "drawing.scale_clear", base(json!({})));
    w.assert_replay();
}

#[test]
fn paths_and_outlines_are_measured_and_taken_off() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let sha = "a".repeat(64);
    let id = w.ok(
        pm,
        "drawing.add",
        json!({"project": p, "number": "A-101", "title": "Plan", "revision": "A", "sha256": sha, "name": "a.pdf", "size": 10}),
    )["id"]
        .as_u64()
        .unwrap();
    let add = |w: &mut World, who, extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        for (k, val) in extra.as_object().unwrap() {
            v[k] = val.clone();
        }
        w.run(who, "drawing.markup_add", v)
    };
    let rev = |w: &World| w.engine.state().drawings.iter().next().unwrap().revisions[0].clone();
    let text_of = |w: &World, n: u64| rev(w).markups.iter().find(|m| u64::from(m.id) == n).unwrap().measure_text();

    // No scale yet: refused. Then a scale where 5,000 units across the page is 10 ft.
    assert!(add(&mut w, pm, json!({"kind": "polyline", "points": "0,0 5000,0 5000,2500"})).is_err());
    w.ok(
        pm,
        "drawing.scale_set",
        json!({"project": p, "id": id, "x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "10", "unit": "ft"}),
    );

    let path = add(&mut w, pm, json!({"kind": "polyline", "points": "0,0 5000,0 5000,2500", "text": "Footing"})).unwrap()["markup"].as_u64().unwrap();
    assert_eq!(text_of(&w, path).as_deref(), Some("15.00 ft"));
    // An L-shaped outline: 10 x 10 ft less a 5 x 5 ft corner.
    let l = "0,0 5000,0 5000,2500 2500,2500 2500,5000 0,5000";
    let area = add(&mut w, pm, json!({"kind": "polygon", "points": l, "text": "Slab"})).unwrap()["markup"].as_u64().unwrap();
    assert_eq!(text_of(&w, area).as_deref(), Some("75.0 sq ft"));
    // The first two corners are kept as x, y, x2, y2, and the corners with the markup.
    let m = rev(&w).markups.iter().find(|m| u64::from(m.id) == area).unwrap().clone();
    assert_eq!((m.x, m.y, m.x2, m.y2, m.points.len()), (0, 0, 5000, 0, 6));

    // Refused: too few points, junk, out of range, a side crossing another, a repeated corner, and a reader.
    for bad in [
        json!({"kind": "polygon", "points": "0,0 100,100"}),
        json!({"kind": "polyline", "points": "0,0"}),
        json!({"kind": "polyline", "points": "0,0 x,5"}),
        json!({"kind": "polyline", "points": "0,0 20000,5"}),
        json!({"kind": "polygon", "points": "0,0 5000,5000 5000,0 0,5000"}),
        json!({"kind": "polyline", "points": "0,0 0,0 100,100"}),
        json!({"kind": "polyline"}),
    ] {
        assert!(add(&mut w, pm, bad.clone()).is_err(), "{bad}");
    }
    let many: Vec<String> = (0..201).map(|i| format!("{},{}", i * 10, if i % 2 == 0 { 0 } else { 10 })).collect();
    assert!(add(&mut w, pm, json!({"kind": "polyline", "points": many.join(" ")})).is_err(), "more than 200 points");
    assert!(add(&mut w, reader, json!({"kind": "polyline", "points": "0,0 100,100"})).is_err());

    // Changing the scale works them out again (the stretch is now 20 m): 30 m and 300 sq m.
    w.ok(
        pm,
        "drawing.scale_set",
        json!({"project": p, "id": id, "x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "20", "unit": "m"}),
    );
    assert_eq!(text_of(&w, path).as_deref(), Some("30.00 m"));
    assert_eq!(text_of(&w, area).as_deref(), Some("300.00 sq m"));

    // A take-off adds a path to the straight lengths of the same label, and an outline to the areas.
    w.ok(
        pm,
        "drawing.scale_set",
        json!({"project": p, "id": id, "x": 2500, "y": 5000, "x2": 7500, "y2": 5000, "aspect": 1000, "length": "10", "unit": "ft"}),
    );
    add(&mut w, pm, json!({"kind": "dim", "x": 0, "y": 1000, "x2": 5000, "y2": 1000, "text": "Footing"})).unwrap();
    let sum = w.ok(pm, "takeoff.summary", json!({"project": p, "drawing": id}));
    let find = |kind: &str, label: &str| sum.as_array().unwrap().iter().find(|g| g["kind"] == kind && g["label"] == label).unwrap().clone();
    assert_eq!((find("dim", "Footing")["quantity"].as_str(), find("dim", "Footing")["count"].as_u64()), (Some("25.000"), Some(2)));
    assert_eq!((find("area", "Slab")["quantity"].as_str(), find("area", "Slab")["unit"].as_str()), (Some("75.000"), Some("sf")));
    w.assert_replay();
}
