//! Change events, commitment change orders and budget changes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::view::{budget_rows, budget_totals, commitment_revised_total};
use buildkiosk_model::finance::{ChangeEventStatus, ChangeStatus};
use buildkiosk_testkit::World;
use serde_json::json;

fn code(w: &World, code: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|c| c.code == code).unwrap().id.raw()
}

fn m(s: &str) -> Money {
    Money::parse(s).unwrap()
}

#[test]
fn change_orders_and_budget_changes_move_the_budget() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(
        pm,
        "member.update",
        json!({"project": p, "person": eng, "overrides": {"change_events": "standard", "change_orders": "standard", "commitments": "standard"}}),
    );
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));
    w.ok(pm, "budget.set_line", json!({"project": p, "cost_code": concrete, "original": "100000"}));
    w.ok(pm, "budget.set_line", json!({"project": p, "cost_code": steel, "original": "50000"}));
    let c = w.ok(pm, "commitment.create", json!({"project": p, "title": "Slab", "lines": [{"cost_code": concrete, "amount": "60000"}]}))["id"]
        .as_u64()
        .unwrap();

    // A change event tracks possible cost: pending while open, nothing once closed.
    let e = w.ok(
        eng,
        "change_event.create",
        json!({"project": p, "title": "Thicker slab at dock", "reason": "Field condition", "lines": [{"cost_code": concrete, "amount": "7500"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let rows = budget_rows(w.engine.state(), p).unwrap();
    assert_eq!(budget_totals(&rows).unwrap().pending, m("7500"));
    w.err(eng, "change_event.void", json!({"project": p, "id": e}));
    w.ok(eng, "change_event.close", json!({"project": p, "id": e}));
    assert_eq!(budget_totals(&budget_rows(w.engine.state(), p).unwrap()).unwrap().pending, Money::ZERO);
    w.err(eng, "change_event.update", json!({"project": p, "id": e, "title": "x"}));
    w.err(eng, "change_event.reopen", json!({"project": p, "id": e}));
    w.ok(pm, "change_event.reopen", json!({"project": p, "id": e}));
    assert_eq!(w.engine.state().change_events.iter().next().unwrap().status, ChangeEventStatus::Open);

    // A change order needs an approved commitment; drafts count for nothing and are private.
    let cco = w.ok(
        eng,
        "commitment_change.create",
        json!({"project": p, "commitment": c, "title": "Extra thickness", "change_event": e, "lines": [{"cost_code": concrete, "amount": "7500"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(pm, "commitment_change.approve", json!({"project": p, "id": cco}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(eng, "commitment_change.approve", json!({"project": p, "id": cco}));
    w.err(eng, "commitment_change.update", json!({"project": p, "id": cco, "commitment": 9999}));
    w.ok(pm, "commitment_change.approve", json!({"project": p, "id": cco}));
    assert_eq!(w.engine.state().commitment_changes.iter().next().unwrap().status, ChangeStatus::Approved);
    w.err(eng, "commitment_change.update", json!({"project": p, "id": cco, "title": "x"}));
    let row = |w: &World, id: u64| budget_rows(w.engine.state(), p).unwrap().into_iter().find(|r| r.cost_code.raw() == id).unwrap();
    assert_eq!(row(&w, concrete).committed, m("67500"));
    let commitment = w.engine.state().commitments.iter().next().unwrap().clone();
    assert_eq!(commitment_revised_total(w.engine.state(), &commitment), Some(m("67500")));

    // A budget change moves 10,000 from steel to concrete; the revised budget follows.
    let bc = w.ok(pm, "budget_change.create", json!({"project": p, "title": "Move to slab", "lines": [{"cost_code": steel, "amount": "-10000"}, {"cost_code": concrete, "amount": "10000"}]}))["id"].as_u64().unwrap();
    assert_eq!(row(&w, concrete).revised(), m("100000"), "a draft changes nothing");
    w.ok(pm, "budget_change.approve", json!({"project": p, "id": bc}));
    assert_eq!((row(&w, concrete).revised(), row(&w, steel).revised()), (m("110000"), m("40000")));
    assert_eq!(row(&w, concrete).remaining(), m("42500"));
    let t = budget_totals(&budget_rows(w.engine.state(), p).unwrap()).unwrap();
    assert_eq!((t.original, t.changes, t.revised()), (m("150000"), Money::ZERO, m("150000")), "a move nets to zero");

    // Voiding the commitment stops its change orders counting too; voiding a budget change reverses it.
    w.ok(pm, "commitment.void", json!({"project": p, "id": c}));
    assert_eq!(row(&w, concrete).committed, Money::ZERO);
    w.ok(pm, "budget_change.void", json!({"project": p, "id": bc}));
    assert_eq!(row(&w, concrete).revised(), m("100000"));
    w.err(pm, "budget_change.update", json!({"project": p, "id": bc, "title": "x"}));
    let got = w.ok(pm, "budget.get", json!({"project": p}));
    assert!(got["rows"][0].get("revised").is_some() && got["totals"].get("pending").is_some());
    w.assert_replay();
}

#[test]
fn rules_visibility_and_isolation() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"change_orders": "standard"}}));
    let steel = code(&w, "05-100");
    let draft = w.ok(eng, "budget_change.create", json!({"project": p, "title": "Draft", "lines": [{"cost_code": steel, "amount": "1"}]}))["id"]
        .as_u64()
        .unwrap();
    // Drafts are private to the author and admins; subcontractors see no financials at all.
    w.ok(pm, "budget_change.get", json!({"project": p, "id": draft}));
    w.err(sub, "budget_change.list", json!({"project": p}));
    w.err(eng, "budget_change.create", json!({"project": p, "title": "Bad code", "lines": [{"cost_code": 99999, "amount": "1"}]}));
    w.err(eng, "budget_change.create", json!({"project": p, "title": "Missing amount", "lines": [{"cost_code": steel}]}));
    let empty = w.ok(pm, "budget_change.create", json!({"project": p, "title": "Empty"}))["id"].as_u64().unwrap();
    w.err(pm, "budget_change.approve", json!({"project": p, "id": empty}));
    let other = w.project("Other");
    w.err(pm, "budget_change.get", json!({"project": other, "id": draft}));
    w.err(pm, "budget_change.create", json!({"project": other, "title": "Cross", "change_event": 1, "lines": []}));
    w.ok(eng, "budget_change.delete", json!({"project": p, "id": draft}));
    w.ok(w.admin(), "budget_change.restore", json!({"project": p, "id": draft}));
    w.assert_replay();
}
