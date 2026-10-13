//! Forms: templates, records, validation, submit.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::form::FormStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn template_fill_and_submit() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let t = w.ok(
        pm,
        "form_template.create",
        json!({"project": p, "name": "Pre-pour check", "fields": [
            {"label": "Rebar inspected", "kind": "yes_no", "required": "true"},
            {"label": "Slump (in)", "kind": "number"},
            {"label": "Mix", "kind": "choice", "options": "3000 psi, 4000 psi, 4000 psi"},
            {"label": "Pour date", "kind": "date"},
            {"label": "Notes", "kind": "long_text"},
            {"label": "", "kind": "text"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    let tpl = w.engine.state().form_templates.iter().next().unwrap().clone();
    assert_eq!((tpl.fields.len(), tpl.fields[2].options.len(), tpl.fields[0].key.as_str()), (5, 2, "f1"));
    w.err(sup, "form_template.create", json!({"project": p, "name": "x", "fields": [{"label": "A"}]}));
    w.err(pm, "form_template.create", json!({"project": p, "name": "pre-pour CHECK", "fields": [{"label": "A"}]}));
    w.err(pm, "form_template.create", json!({"project": p, "name": "Bad", "fields": [{"label": "Pick", "kind": "choice", "options": "only one"}]}));
    w.err(pm, "form_template.create", json!({"project": p, "name": "Empty", "fields": []}));

    let f = w.ok(
        sup,
        "form.create",
        json!({"project": p, "template": t, "title": "Slab A", "answers": {"f1": "Yes", "f2": "4.5", "f3": "4000 PSI"}}),
    )["id"]
        .as_u64()
        .unwrap();
    let rec = w.engine.state().form_records.iter().next().unwrap().clone();
    assert_eq!((rec.answers["f1"].as_str(), rec.answers["f3"].as_str(), rec.number), ("yes", "4000 psi", 1));
    w.err(sup, "form.update", json!({"project": p, "id": f, "answers": {"f2": "lots"}}));
    w.err(sup, "form.update", json!({"project": p, "id": f, "answers": {"f4": "2026-13-45"}}));
    w.err(sup, "form.update", json!({"project": p, "id": f, "answers": {"f9": "x"}}));
    w.err(sup, "form.update", json!({"project": p, "id": f, "answers": {"f3": "9000 psi"}}));
    w.err(reader, "form.update", json!({"project": p, "id": f, "title": "x"}));
    w.ok(sup, "form.update", json!({"project": p, "id": f, "answers": {"f4": "2026-10-20", "f2": ""}}));
    assert!(!w.engine.state().form_records.iter().next().unwrap().answers.contains_key("f2"), "a blank answer clears it");

    // Required questions block submitting; editing the template leaves the record's questions alone.
    w.ok(sup, "form.update", json!({"project": p, "id": f, "answers": {"f1": ""}}));
    w.err(sup, "form.submit", json!({"project": p, "id": f}));
    w.ok(pm, "form_template.update", json!({"project": p, "id": t, "name": "Pre-pour checklist", "fields": [{"label": "Only one"}]}));
    assert_eq!(w.engine.state().form_records.iter().next().unwrap().fields.len(), 5);
    w.ok(sup, "form.update", json!({"project": p, "id": f, "answers": {"f1": "no"}}));
    w.ok(sup, "form.submit", json!({"project": p, "id": f}));
    assert_eq!(w.engine.state().form_records.iter().next().unwrap().status, FormStatus::Submitted);
    w.err(sup, "form.update", json!({"project": p, "id": f, "title": "changed"}));
    w.err(sup, "form.reopen", json!({"project": p, "id": f}));
    w.ok(pm, "form.reopen", json!({"project": p, "id": f}));
    w.ok(sup, "form.attach", json!({"project": p, "id": f, "sha256": "a".repeat(64), "name": "photo.jpg", "size": 3}));
    assert_eq!(w.ok(reader, "form.list", json!({"project": p, "q": "4000"})).as_array().unwrap().len(), 1);
    w.err(reader, "form.list", json!({"project": p, "deleted": true}));
    w.ok(sup, "form.delete", json!({"project": p, "id": f}));
    w.ok(pm, "form.restore", json!({"project": p, "id": f}));
    let other = w.project("Other");
    w.err(pm, "form.get", json!({"project": other, "id": f}));
    w.assert_replay();
}
