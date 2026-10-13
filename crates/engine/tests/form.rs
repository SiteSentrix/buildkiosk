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

#[test]
fn conditions_signatures_copies_schedules_and_report() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let q = w.project("Second job");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.member(q, "Quinn", "Other", "Project manager");

    // A conditional question and two signing roles.
    let t = w.ok(
        pm,
        "form_template.create",
        json!({"project": p, "name": "Hot work permit", "signers": "Fire watch, Superintendent, fire watch", "fields": [
            {"label": "Welding?", "kind": "yes_no", "required": "true"},
            {"label": "Fire extinguisher serial", "kind": "text", "required": "true", "show_if_question": 1, "show_if_answer": "Yes"},
            {"label": "Area", "kind": "choice", "options": "Roof, Basement"},
            {"label": "Roof anchor tag", "kind": "text", "required": "true", "show_if_question": 3, "show_if_answer": "roof"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    let tpl = w.engine.state().form_templates.iter().next().unwrap().clone();
    assert_eq!((tpl.signers.len(), tpl.fields[1].show_if.as_ref().unwrap().equals.as_str()), (2, "yes"));
    w.err(pm, "form_template.create", json!({"project": p, "name": "A", "fields": [{"label": "Q", "show_if_question": 1, "show_if_answer": "x"}]}));
    w.err(
        pm,
        "form_template.create",
        json!({"project": p, "name": "B", "fields": [{"label": "Q", "kind": "yes_no"}, {"label": "R", "show_if_question": 1}]}),
    );
    w.err(pm, "form_template.create", json!({"project": p, "name": "C", "fields": [{"label": "Q", "kind": "yes_no"}, {"label": "R", "show_if_question": 1, "show_if_answer": "maybe"}]}));

    let f = w.ok(sup, "form.create", json!({"project": p, "template": t, "answers": {"f1": "no", "f2": "FE-12", "f3": "Basement"}}))["id"]
        .as_u64()
        .unwrap();
    // Required questions that are not shown do not block the submit, and their answers are dropped.
    w.ok(sup, "form.submit", json!({"project": p, "id": f}));
    let rec = w.engine.state().form_records.iter().find(|r| r.id.raw() == f).unwrap().clone();
    assert!(!rec.answers.contains_key("f2") && rec.answers.get("f3").map(String::as_str) == Some("Basement"));
    let g = w.ok(sup, "form.create", json!({"project": p, "template": t, "answers": {"f1": "yes", "f3": "Roof"}}))["id"].as_u64().unwrap();
    let e = w.err(sup, "form.submit", json!({"project": p, "id": g}));
    assert!(e.to_string().contains("Fire extinguisher serial") && e.to_string().contains("Roof anchor tag"), "{e}");
    w.ok(sup, "form.update", json!({"project": p, "id": g, "answers": {"f2": "FE-9", "f4": "A-4"}}));
    w.ok(sup, "form.submit", json!({"project": p, "id": g}));

    // Signatures: typed own name, one role each, one role per person; changes by an admin void them.
    w.err(sup, "form.sign", json!({"project": p, "id": g, "role": "Inspector", "name": "Sue Super"}));
    w.err(sup, "form.sign", json!({"project": p, "id": g, "role": "Superintendent", "name": "Someone Else"}));
    w.ok(sup, "form.sign", json!({"project": p, "id": g, "role": "superintendent", "name": "sue super"}));
    w.err(sup, "form.sign", json!({"project": p, "id": g, "role": "Fire watch", "name": "Sue Super"}));
    w.err(eng, "form.sign", json!({"project": p, "id": g, "role": "Superintendent", "name": "Eli Engineer"}));
    w.ok(eng, "form.sign", json!({"project": p, "id": g, "role": "Fire watch", "name": "Eli Engineer"}));
    let rec = w.engine.state().form_records.iter().find(|r| r.id.raw() == g).unwrap().clone();
    assert_eq!((rec.signatures.len(), rec.unsigned().len()), (2, 0));
    let d = w.ok(sup, "form.create", json!({"project": p, "template": t}))["id"].as_u64().unwrap();
    w.err(sup, "form.sign", json!({"project": p, "id": d, "role": "Superintendent", "name": "Sue Super"}));
    w.ok(pm, "form.update", json!({"project": p, "id": g, "answers": {"f4": "A-5"}}));
    assert!(w.engine.state().form_records.iter().find(|r| r.id.raw() == g).unwrap().signatures.is_empty());

    // Copy a template to another project: only for admins of both.
    w.err(pm, "form_template.copy", json!({"project": p, "id": t, "to_project": q}));
    let quinn = w.engine.state().people.iter().find(|x| x.first_name == "Quinn").unwrap().id;
    w.ok(w.admin(), "member.add", json!({"project": p, "person": quinn, "template": buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap()}));
    w.ok(quinn, "form_template.copy", json!({"project": p, "id": t, "to_project": q}));
    w.err(quinn, "form_template.copy", json!({"project": p, "id": t, "to_project": q}));
    assert_eq!(w.engine.state().form_templates.iter().filter(|x| x.project == q).count(), 1);

    // A schedule makes one draft form per due day for its assignee, once per pass.
    let day = w.now();
    let today = day.date_at(0).to_string();
    w.err(sup, "form_schedule.save", json!({"project": p, "template": t, "every": "daily", "assignee": sup.raw()}));
    w.err(pm, "form_schedule.save", json!({"project": p, "template": t, "every": "never", "assignee": sup.raw()}));
    w.ok(
        pm,
        "form_schedule.save",
        json!({"project": p, "template": t, "every": "weekly", "assignee": eng.raw(), "title": "Weekly permit", "next": today}),
    );
    assert_eq!(w.run(buildkiosk_engine::Actor::System, "form.generate_due", json!({})).unwrap()["made"], 1);
    assert_eq!(w.run(buildkiosk_engine::Actor::System, "form.generate_due", json!({})).unwrap()["made"], 0);
    let made = w.engine.state().form_records.iter().find(|r| r.assignee == Some(eng)).unwrap().clone();
    assert!(made.title.starts_with("Weekly permit") && made.schedule.is_some());
    assert!(w.engine.state().notifications.iter().any(|n| n.person == eng && n.text.contains("ready to fill in")));
    w.ok(eng, "form.update", json!({"project": p, "id": made.id.raw(), "answers": {"f1": "no"}}));
    w.err(sup, "form.update", json!({"project": p, "id": made.id.raw(), "answers": {"f1": "yes"}}));
    w.err(sup, "form.generate_due", json!({}));
    w.advance_days(7);
    assert_eq!(w.run(buildkiosk_engine::Actor::System, "form.generate_due", json!({})).unwrap()["made"], 1);
    assert_eq!(w.ok(sup, "form_schedule.list", json!({"project": p})).as_array().unwrap().len(), 1);

    // The report: one row per form, one column per question.
    let r = w.ok(sup, "form.report", json!({"project": p, "template": t}));
    assert_eq!(r["columns"].as_array().unwrap().len(), 4);
    assert_eq!(r["rows"].as_array().unwrap().len(), 5);
    let r = w.ok(sup, "form.report", json!({"project": p, "template": t, "status": "submitted"}));
    assert_eq!(r["rows"].as_array().unwrap().len(), 2);
    assert_eq!(r["rows"][1]["answers"][1], "FE-9");
    w.err(sup, "form.report", json!({"project": q, "template": t}));
    w.assert_replay();
}
