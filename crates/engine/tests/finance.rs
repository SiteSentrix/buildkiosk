//! Budget, commitments and direct costs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::view::{budget_rows, budget_totals};
use buildkiosk_model::finance::CommitmentStatus;
use buildkiosk_testkit::World;
use serde_json::json;

/// The id of a starter cost code.
fn code(w: &World, code: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|c| c.code == code).unwrap().id.raw()
}

fn money(units: i64) -> Money {
    Money::from_units(units).unwrap()
}

#[test]
fn budget_rolls_up_commitments_and_direct_costs() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    let concrete = code(&w, "03-300");
    let steel = code(&w, "05-100");
    let other = w.project("Other");
    // Project engineers read financials; give this one Standard access to commitments and direct costs.
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"commitments": "standard", "direct_costs": "standard"}}));

    // The budget: set a line, change it, import the rest; a code needs to exist for the project.
    w.ok(pm, "budget.set_line", json!({"project": p, "cost_code": concrete, "original": "100,000.00"}));
    w.ok(pm, "budget.set_line", json!({"project": p, "cost_code": concrete, "original": "120000"}));
    assert_eq!(w.engine.state().budget_lines.iter().filter(|b| b.deleted.is_none()).count(), 1, "one line per code");
    w.err(pm, "budget.set_line", json!({"project": other, "cost_code": concrete, "original": "5"}));
    w.err(pm, "budget.set_line", json!({"project": p, "cost_code": concrete, "original": "-5"}));
    w.ok(pm, "budget.import", json!({"project": p, "text": "code,amount,notes\n05-100, 80000, Erected by others\n"}));
    w.err(pm, "budget.import", json!({"project": p, "text": "99-999, 10"}));
    w.err(sub, "budget.get", json!({"project": p}));
    w.err(eng, "budget.set_line", json!({"project": p, "cost_code": steel, "original": "1"}));
    assert_eq!(w.ok(eng, "budget.get", json!({"project": p}))["rows"].as_array().unwrap().len(), 2);

    // A draft commitment costs nothing until approved; only admins approve.
    let c = w.ok(
        eng,
        "commitment.create",
        json!({"project": p, "title": "Footings and slab", "kind": "subcontract", "lines": [
            {"cost_code": concrete, "description": "Footings", "amount": "30,000.50"},
            {"cost_code": concrete, "amount": "20000"},
            {"cost_code": null, "amount": "", "description": ""}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    let commitment = w.engine.state().commitments.iter().next().unwrap().clone();
    assert_eq!((commitment.lines.len(), commitment.total().unwrap()), (2, Money::parse("50000.50").unwrap()));
    let rows = budget_rows(w.engine.state(), p).unwrap();
    assert!(rows.iter().all(|r| r.committed.is_zero()));
    w.err(eng, "commitment.approve", json!({"project": p, "id": c}));
    w.err(sub, "commitment.get", json!({"project": p, "id": c}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    assert_eq!(w.engine.state().commitments.iter().next().unwrap().status, CommitmentStatus::Approved);
    w.err(eng, "commitment.update", json!({"project": p, "id": c, "title": "x"}));

    // Direct costs land on codes, credits count against them.
    w.ok(
        eng,
        "direct_cost.create",
        json!({"project": p, "cost_code": concrete, "amount": "1250.25", "description": "Pump truck", "kind": "equipment", "date": "2026-10-10"}),
    );
    w.ok(eng, "direct_cost.create", json!({"project": p, "cost_code": steel, "amount": "4000", "description": "Anchor bolts"}));
    w.ok(eng, "direct_cost.create", json!({"project": p, "cost_code": steel, "amount": "-500", "description": "Returned bolts"}));
    w.err(eng, "direct_cost.create", json!({"project": p, "cost_code": steel, "amount": "0", "description": "Nothing"}));
    w.err(eng, "direct_cost.create", json!({"project": p, "cost_code": 9999, "amount": "5", "description": "Bad code"}));

    let rows = budget_rows(w.engine.state(), p).unwrap();
    let t = budget_totals(&rows).unwrap();
    let (original, committed, direct) = (t.original, t.committed, t.direct);
    assert_eq!((original, committed, direct), (money(200_000), Money::parse("50000.50").unwrap(), Money::parse("4750.25").unwrap()));
    let concrete_row = &rows[0];
    assert_eq!(
        (concrete_row.original, concrete_row.cost(), concrete_row.remaining()),
        (money(120_000), Money::parse("51250.75").unwrap(), Money::parse("68749.25").unwrap())
    );
    let steel_row = &rows[1];
    assert_eq!(steel_row.remaining(), Money::parse("76500.00").unwrap());
    let got = w.ok(pm, "budget.get", json!({"project": p}));
    assert_eq!(got["rows"][0]["code"], "03-300");
    assert_eq!(got["totals"]["committed"], json!(Money::parse("50000.50").unwrap()));

    // Voiding stops the commitment counting; deleting a cost removes it.
    w.ok(pm, "commitment.void", json!({"project": p, "id": c}));
    assert!(budget_rows(w.engine.state(), p).unwrap().iter().all(|r| r.committed.is_zero()));
    let d = w.ok(pm, "direct_cost.list", json!({"project": p}))[0]["id"].as_u64().unwrap();
    w.err(sub, "direct_cost.delete", json!({"project": p, "id": d}));
    w.ok(pm, "direct_cost.delete", json!({"project": p, "id": d}));
    w.assert_replay();
}

#[test]
fn money_overflow_is_an_error_not_a_panic() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let code = code(&w, "01-100");
    let big = "9999999999999.99";
    for _ in 0..3 {
        let _ = w.run(pm, "direct_cost.create", json!({"project": p, "cost_code": code, "amount": big, "description": "Huge"}));
    }
    // However many were accepted, reading the budget never panics.
    let _ = w.run(pm, "budget.get", json!({"project": p}));
    let _ = w.run(
        pm,
        "commitment.create",
        json!({"project": p, "title": "Huge", "lines": [{"cost_code": code, "amount": big}, {"cost_code": code, "amount": big}]}),
    );
    w.assert_replay();
}

#[test]
fn commitment_files() {
    const SHA: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"commitments": "standard"}}));
    w.ok(pm, "member.update", json!({"project": p, "person": reader, "overrides": {"commitments": "read"}}));
    let concrete = code(&w, "03-300");
    let c = w.ok(pm, "commitment.create", json!({"project": p, "title": "Concrete", "lines": [{"cost_code": concrete, "amount": "1000"}]}))["id"]
        .as_u64()
        .unwrap();

    w.ok(pm, "commitment.attach", json!({"project": p, "id": c, "sha256": SHA, "name": "contract.pdf", "size": 10, "mime": "application/pdf"}));
    w.ok(pm, "commitment.attach", json!({"project": p, "id": c, "sha256": SHA, "name": "contract.pdf", "size": 10}));
    assert_eq!(w.engine.state().commitments.iter().next().unwrap().attachments.len(), 1, "the same file is added once");
    w.err(reader, "commitment.attach", json!({"project": p, "id": c, "sha256": SHA, "name": "x", "size": 1}));
    w.err(pm, "commitment.attach", json!({"project": p, "id": c, "sha256": "nope", "name": "x", "size": 1}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(eng, "commitment.detach", json!({"project": p, "id": c, "sha256": SHA}));
    w.err(pm, "commitment.detach", json!({"project": p, "id": c, "sha256": "d".repeat(64)}));
    w.ok(pm, "commitment.detach", json!({"project": p, "id": c, "sha256": SHA}));
    w.ok(pm, "commitment.void", json!({"project": p, "id": c}));
    w.err(pm, "commitment.attach", json!({"project": p, "id": c, "sha256": SHA, "name": "x", "size": 1}));
    w.assert_replay();
}
