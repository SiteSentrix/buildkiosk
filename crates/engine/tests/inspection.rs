//! Inspections: templates, answering a checklist, completing, and raising punch items.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::inspection::{Answer, InspectionStatus};
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn templates_and_the_starter_set() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let out = w.ok(pm, "inspection_template.install_starters", json!({"project": p}));
    assert_eq!(out["added"], 4);
    assert_eq!(w.ok(pm, "inspection_template.install_starters", json!({"project": p}))["added"], 0, "installing twice adds nothing");
    let list = w.ok(reader, "inspection_template.list", json!({"project": p}));
    assert_eq!(list.as_array().unwrap().len(), 4);

    w.ok(
        pm,
        "inspection_template.create",
        json!({"project": p, "name": "Roof check", "items": "# Membrane\nSeams are sealed\nFlashing is lapped\n# Drains\nDrains are clear"}),
    );
    let roof = w.engine.state().inspection_templates.iter().find(|t| t.name == "Roof check").unwrap().clone();
    assert_eq!((roof.items.len(), roof.items[2].section.as_str()), (3, "Drains"));
    w.err(pm, "inspection_template.create", json!({"project": p, "name": "roof CHECK", "items": "x"}));
    w.err(pm, "inspection_template.create", json!({"project": p, "name": "Empty", "items": "# Only a heading"}));
    w.err(reader, "inspection_template.create", json!({"project": p, "name": "Nope", "items": "x"}));
    w.assert_replay();
}

#[test]
fn answer_complete_and_raise_punch_items() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let insp = w.member(p, "Ines", "Inspector", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let unread = |w: &mut World, who| w.ok(who, "notification.list", json!({"unread": true})).as_array().unwrap().len();

    w.ok(pm, "inspection_template.install_starters", json!({"project": p}));
    let t = w.engine.state().inspection_templates.iter().find(|t| t.name == "Framing walk-through").unwrap().id;
    let id = w.ok(pm, "inspection.create", json!({"project": p, "template": t, "location": "Level 2", "inspector": insp, "scheduled": "2026-10-15"}))
        ["id"]
        .as_u64()
        .unwrap();
    let get = |w: &World| w.engine.state().inspections.iter().next().unwrap().clone();
    assert_eq!((get(&w).title.as_str(), get(&w).items.len()), ("Framing walk-through", 7));
    assert_eq!(unread(&mut w, insp), 1);

    // Changing the template afterwards leaves the inspection alone.
    w.ok(pm, "inspection_template.delete", json!({"project": p, "id": t}));
    assert_eq!(get(&w).items.len(), 7);

    w.err(sub, "inspection.answer", json!({"project": p, "id": id, "answers": [{"line": 1, "answer": "pass"}]}));
    w.err(insp, "inspection.answer", json!({"project": p, "id": id, "answers": [{"line": 99, "answer": "pass"}]}));
    w.err(insp, "inspection.answer", json!({"project": p, "id": id, "answers": [{"line": 1, "answer": "maybe"}]}));
    let mut answers: Vec<_> = (1..=7).map(|n| json!({"line": n, "answer": "pass"})).collect();
    answers[2] = json!({"line": 3, "answer": "fail", "note": "No backing at the sink wall"});
    answers[5] = json!({"line": 6, "answer": "na"});
    w.ok(insp, "inspection.answer", json!({"project": p, "id": id, "answers": &answers[..6]}));
    assert_eq!((get(&w).answered(), get(&w).failed()), (6, 1));
    w.err(insp, "inspection.complete", json!({"project": p, "id": id}));
    w.ok(insp, "inspection.answer", json!({"project": p, "id": id, "answers": [answers[6].clone()]}));
    w.ok(insp, "inspection.complete", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, InspectionStatus::Completed);
    assert_eq!(unread(&mut w, pm), 1, "the manager is told it is complete");
    w.err(insp, "inspection.answer", json!({"project": p, "id": id, "answers": [{"line": 1, "answer": "fail"}]}));

    // Failed lines become punch items once, with the note and the location.
    w.err(insp, "inspection.raise_punch", json!({"project": p, "id": id}));
    let out = w.ok(pm, "inspection.raise_punch", json!({"project": p, "id": id, "assignees": [sub]}));
    assert_eq!(out["punch_items"].as_array().unwrap().len(), 1);
    let item = w.engine.state().punch_items.iter().next().unwrap().clone();
    assert!(item.title.contains("Inspection #1") && item.description.contains("No backing") && item.location == "Level 2");
    assert_eq!(item.assignees, vec![sub]);
    assert_eq!(get(&w).items[2].punch, Some(item.id));
    assert_eq!(get(&w).items[2].answer, Some(Answer::Fail));
    w.err(pm, "inspection.raise_punch", json!({"project": p, "id": id}));

    w.err(insp, "inspection.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "inspection.reopen", json!({"project": p, "id": id}));
    assert_eq!(get(&w).status, InspectionStatus::Open);
    // Changing an answer detaches the punch item raised for the old one.
    w.ok(insp, "inspection.answer", json!({"project": p, "id": id, "answers": [{"line": 3, "answer": "pass"}]}));
    assert_eq!(get(&w).items[2].punch, None);
    w.assert_replay();
}

#[test]
fn ad_hoc_inspections_files_and_visibility() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    w.err(pm, "inspection.create", json!({"project": p}));
    w.err(pm, "inspection.create", json!({"project": p, "items": "One line"}));
    let id =
        w.ok(pm, "inspection.create", json!({"project": p, "title": "Quick check", "items": "Door closes\nLatch holds"}))["id"].as_u64().unwrap();
    assert_eq!(w.ok(sub, "inspection.list", json!({"project": p})).as_array().unwrap().len(), 1);
    let file = json!({"project": p, "id": id, "sha256": "a".repeat(64), "name": "photo.jpg", "size": 9});
    w.err(sub, "inspection.attach", file.clone());
    w.ok(pm, "inspection.attach", file.clone());
    w.ok(pm, "inspection.attach", file);
    assert_eq!(w.engine.state().inspections.iter().next().unwrap().attachments.len(), 1);
    w.ok(pm, "inspection.detach", json!({"project": p, "id": id, "sha256": "a".repeat(64)}));
    let other = w.project("Other");
    w.err(pm, "inspection.get", json!({"project": other, "id": id}));
    w.ok(pm, "inspection.delete", json!({"project": p, "id": id}));
    w.ok(w.admin(), "inspection.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}
