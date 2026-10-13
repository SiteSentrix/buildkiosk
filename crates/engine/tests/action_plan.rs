//! Action plans: outline, items, activation, completion.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_model::action_plan::PlanStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn outline_assign_tick_and_complete() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let id = w.ok(
        sup,
        "action_plan.create",
        json!({"project": p, "title": "Closeout", "outline": "Loose item\n# Documents\n- Collect warranties\n* Record drawings\n\n# Site\nFinal clean\n"}),
    )["id"]
        .as_u64()
        .unwrap();
    let plan = w.engine.state().action_plans.iter().next().unwrap().clone();
    assert_eq!(plan.sections.len(), 3);
    assert_eq!((plan.sections[0].title.as_str(), plan.sections[1].items.len(), plan.progress()), ("Items", 2, (0, 4)));
    w.err(sup, "action_plan.create", json!({"project": p, "title": "x", "outline": "#\nitem"}));
    w.err(reader, "action_plan.create", json!({"project": p, "title": "x"}));
    w.err(sup, "action_plan.activate", json!({"project": p, "id": w.engine.state().action_plans.iter().next().unwrap().id.raw() + 5}));

    // Only the manager (the author here) assigns; owners tick their own items.
    w.err(eng, "action_plan.set_item", json!({"project": p, "id": id, "section": 1, "item": 0, "assignee": eng.raw()}));
    w.ok(
        sup,
        "action_plan.set_item",
        json!({"project": p, "id": id, "section": 1, "item": 0, "assignee": eng.raw(), "due": "2026-11-15", "note": "Binder"}),
    );
    w.ok(sup, "action_plan.activate", json!({"project": p, "id": id}));
    w.err(sup, "action_plan.activate", json!({"project": p, "id": id}));
    assert!(w.engine.state().notifications.iter().any(|n| n.person == eng && n.text.contains("Action plan #1")));
    w.err(reader, "action_plan.set_item", json!({"project": p, "id": id, "section": 1, "item": 0, "done": true}));
    w.err(eng, "action_plan.set_item", json!({"project": p, "id": id, "section": 1, "item": 1, "done": true}));
    w.ok(eng, "action_plan.set_item", json!({"project": p, "id": id, "section": 1, "item": 0, "done": true}));
    w.err(sup, "action_plan.set_item", json!({"project": p, "id": id, "section": 9, "item": 0, "done": true}));
    w.err(sup, "action_plan.complete", json!({"project": p, "id": id}));

    // A new outline keeps what was recorded on the items that stay.
    w.ok(sup, "action_plan.update", json!({"project": p, "id": id, "outline": "# Documents\n- Collect warranties\n- Record drawings\n- Training"}));
    let plan = w.engine.state().action_plans.iter().next().unwrap().clone();
    assert_eq!(plan.sections.len(), 1);
    assert!(plan.sections[0].items[0].done && plan.sections[0].items[0].assignee == Some(eng) && plan.sections[0].items[0].note == "Binder");
    for i in 1..3 {
        w.ok(sup, "action_plan.set_item", json!({"project": p, "id": id, "section": 0, "item": i, "done": true}));
    }
    w.err(eng, "action_plan.complete", json!({"project": p, "id": id}));
    w.ok(sup, "action_plan.complete", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().status, PlanStatus::Complete);
    w.err(sup, "action_plan.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(eng, "action_plan.reopen", json!({"project": p, "id": id}));
    w.ok(pm, "action_plan.reopen", json!({"project": p, "id": id}));
    assert_eq!(w.ok(reader, "action_plan.list", json!({"project": p, "q": "training"})).as_array().unwrap().len(), 1);
    w.err(eng, "action_plan.delete", json!({"project": p, "id": id}));
    w.ok(sup, "action_plan.delete", json!({"project": p, "id": id}));
    w.ok(pm, "action_plan.restore", json!({"project": p, "id": id}));
    let other = w.project("Other");
    w.err(pm, "action_plan.get", json!({"project": other, "id": id}));
    w.assert_replay();
}

#[test]
fn approvers_comments_templates_and_sign_off() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let owner = w.member(p, "Olga", "Owner", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let id = w.ok(sup, "action_plan.create", json!({"project": p, "title": "Closeout", "outline": "# Docs\n- Warranties\n- Record drawings"}))["id"]
        .as_u64()
        .unwrap();
    // Approvers: only the manager names them, only they answer, and the plan waits for all of them.
    w.err(eng, "action_plan.set_approvers", json!({"project": p, "id": id, "approvers": [owner.raw()]}));
    w.ok(sup, "action_plan.set_approvers", json!({"project": p, "id": id, "approvers": [owner.raw(), eng.raw()]}));
    assert!(w.engine.state().notifications.iter().any(|n| n.person == owner && n.text.contains("approve")));
    w.err(sup, "action_plan.approve", json!({"project": p, "id": id}));
    w.err(sup, "action_plan.activate", json!({"project": p, "id": id}));
    w.ok(owner, "action_plan.approve", json!({"project": p, "id": id}));
    w.ok(eng, "action_plan.approve", json!({"project": p, "id": id, "approved": false, "note": "Add training"}));
    w.err(sup, "action_plan.activate", json!({"project": p, "id": id}));
    // Changing the wording clears the answers; a tweak to an owner or date does not.
    w.ok(sup, "action_plan.update", json!({"project": p, "id": id, "outline": "# Docs\n- Warranties\n- Record drawings\n- Training"}));
    assert!(w.engine.state().action_plans.iter().next().unwrap().approvals.is_empty());
    w.ok(owner, "action_plan.approve", json!({"project": p, "id": id}));
    w.ok(eng, "action_plan.approve", json!({"project": p, "id": id}));
    w.ok(sup, "action_plan.set_item", json!({"project": p, "id": id, "section": 0, "item": 0, "assignee": eng.raw()}));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().approvals.len(), 2);
    // Taking an approver off drops their answer.
    w.ok(sup, "action_plan.set_approvers", json!({"project": p, "id": id, "approvers": [owner.raw()]}));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().approvals.len(), 1);
    w.ok(sup, "action_plan.activate", json!({"project": p, "id": id}));

    // Comments: the owner, the manager and Standard users; readers cannot.
    w.ok(eng, "action_plan.comment", json!({"project": p, "id": id, "section": 0, "item": 0, "text": "Binder is at the trailer"}));
    w.err(reader, "action_plan.comment", json!({"project": p, "id": id, "section": 0, "item": 0, "text": "hi"}));
    w.err(eng, "action_plan.comment", json!({"project": p, "id": id, "section": 0, "item": 0, "text": ""}));
    w.err(eng, "action_plan.comment", json!({"project": p, "id": id, "section": 5, "item": 0, "text": "x"}));
    assert!(w.engine.state().notifications.iter().any(|n| n.person == sup && n.text.contains("New comment")));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().sections[0].items[0].comments.len(), 1);

    // Sign-off: completion records who and when; reopening clears it.
    for i in 0..3 {
        w.ok(sup, "action_plan.set_item", json!({"project": p, "id": id, "section": 0, "item": i, "done": true}));
    }
    w.ok(sup, "action_plan.complete", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().completed.unwrap().by, Some(sup));

    // Templates are company-wide, without owners, dates or progress.
    w.err(reader, "action_plan.template_save", json!({"project": p, "id": id, "name": "Closeout"}));
    let t = w.ok(sup, "action_plan.template_save", json!({"project": p, "id": id, "name": "Closeout"}))["id"].as_u64().unwrap();
    assert_eq!(w.ok(sup, "action_plan.template_save", json!({"project": p, "id": id, "name": "closeout"}))["id"].as_u64().unwrap(), t);
    assert_eq!(w.ok(reader, "action_plan.template_list", json!({})).as_array().unwrap().len(), 1);
    let tpl = w.engine.state().plan_templates.iter().next().unwrap().clone();
    assert!(tpl.sections[0].items.iter().all(|i| !i.done && i.assignee.is_none() && i.comments.is_empty()));
    let other = w.project("Second job");
    let sup2 = w.member(other, "Sam", "Super", "Superintendent");
    let n = w.ok(sup2, "action_plan.create", json!({"project": other, "title": "Closeout 2", "template": t}))["id"].as_u64().unwrap();
    let copy = w.engine.state().action_plans.iter().find(|a| a.project == other).unwrap().clone();
    assert_eq!((copy.sections[0].items.len(), copy.id.raw()), (3, n));
    w.err(sup2, "action_plan.create", json!({"project": other, "title": "x", "template": 9_999}));
    w.err(eng, "action_plan.template_delete", json!({"id": t}));
    w.ok(sup, "action_plan.template_delete", json!({"id": t}));
    assert_eq!(w.ok(sup, "action_plan.template_list", json!({})).as_array().unwrap().len(), 0);
    w.assert_replay();
}

#[test]
fn files_on_the_plan_itself() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let id = w.ok(sup, "action_plan.create", json!({"project": p, "title": "Closeout", "outline": "# A\nDo it"}))["id"].as_u64().unwrap();
    let sha = "a".repeat(64);
    let file = json!({"project": p, "id": id, "sha256": sha, "name": "programme.pdf", "size": 10});
    w.err(reader, "action_plan.attach", file.clone());
    w.ok(sup, "action_plan.attach", file.clone());
    // The same file twice is one file; a bad hash is refused.
    w.ok(sup, "action_plan.attach", file.clone());
    w.err(sup, "action_plan.attach", json!({"project": p, "id": id, "sha256": "xyz", "name": "x", "size": 1}));
    assert_eq!(w.engine.state().action_plans.iter().next().unwrap().attachments.len(), 1);
    w.err(sup, "action_plan.detach", json!({"project": p, "id": id, "sha256": "b".repeat(64)}));
    // A finished plan keeps its files until it is reopened.
    w.ok(sup, "action_plan.activate", json!({"project": p, "id": id}));
    w.ok(sup, "action_plan.set_item", json!({"project": p, "id": id, "section": 0, "item": 0, "done": true}));
    w.ok(sup, "action_plan.complete", json!({"project": p, "id": id}));
    w.err(sup, "action_plan.attach", json!({"project": p, "id": id, "sha256": "c".repeat(64), "name": "y", "size": 1}));
    w.err(sup, "action_plan.detach", json!({"project": p, "id": id, "sha256": sha}));
    w.ok(sup, "action_plan.reopen", json!({"project": p, "id": id}));
    w.ok(sup, "action_plan.detach", json!({"project": p, "id": id, "sha256": sha}));
    assert!(w.engine.state().action_plans.iter().next().unwrap().attachments.is_empty());
    w.assert_replay();
}
