//! Budget forecasting, locking and snapshots.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::view::{budget_rows, budget_totals};
use buildkiosk_testkit::World;
use serde_json::json;

fn code(w: &World, code: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|c| c.code == code).unwrap().id.raw()
}

fn money(s: &str) -> Money {
    Money::parse(s).unwrap()
}

#[test]
fn forecast_lock_and_snapshots() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"budget": "standard"}}));
    w.ok(pm, "member.update", json!({"project": p, "person": reader, "overrides": {"budget": "read"}}));
    let concrete = code(&w, "03-300");
    let steel = code(&w, "05-100");
    w.ok(pm, "budget.import", json!({"project": p, "text": "03-300, 100000\n05-100, 50000"}));
    w.ok(pm, "direct_cost.create", json!({"project": p, "cost_code": concrete, "amount": "30000", "description": "Slab pour", "date": "2026-10-01"}));

    // Without a forecast the budget is assumed spent: projected = max(revised, cost).
    let rows = budget_rows(w.engine.state(), p).unwrap();
    let r = rows.iter().find(|r| r.cost_code.raw() == concrete).unwrap();
    assert_eq!((r.projected(), r.variance()), (money("100000"), Money::ZERO));

    // A forecast makes it cost to date plus the forecast; an overrun shows as a negative variance.
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": "85000"}));
    w.err(reader, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": "1"}));
    w.err(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": "-1"}));
    let rows = budget_rows(w.engine.state(), p).unwrap();
    let r = rows.iter().find(|r| r.cost_code.raw() == concrete).unwrap();
    assert_eq!((r.projected(), r.variance()), (money("115000"), money("-15000")));
    let t = budget_totals(&rows).unwrap();
    assert_eq!((t.projected, t.variance()), (money("165000"), money("-15000")));
    let got = w.ok(reader, "budget.get", json!({"project": p}));
    assert_eq!(got["totals"]["variance"], "-15000.00");
    // Clearing the forecast goes back to the default.
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": ""}));
    assert_eq!(budget_totals(&budget_rows(w.engine.state(), p).unwrap()).unwrap().variance(), Money::ZERO);
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": "85000"}));

    // Snapshot, then change the budget and compare.
    w.err(reader, "budget.snapshot", json!({"project": p, "name": "x"}));
    let snap = w.ok(eng, "budget.snapshot", json!({"project": p, "name": "Baseline", "note": "Start of construction"}))["id"].as_u64().unwrap();
    w.ok(pm, "budget.lock", json!({"project": p}));
    w.err(eng, "budget.unlock", json!({"project": p}));
    w.err(pm, "budget.lock", json!({"project": p}));
    w.err(pm, "budget.set_line", json!({"project": p, "cost_code": steel, "original": "60000"}));
    w.err(pm, "budget.delete_line", json!({"project": p, "id": 1}));
    w.err(pm, "budget.import", json!({"project": p, "text": "05-100, 1"}));
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": steel, "forecast_to_complete": "10000"}));
    assert!(w.ok(reader, "budget.get", json!({"project": p}))["locked"].as_bool().unwrap());
    w.ok(pm, "budget.unlock", json!({"project": p}));
    w.ok(pm, "budget.set_line", json!({"project": p, "cost_code": steel, "original": "60000"}));

    let list = w.ok(reader, "budget.snapshots", json!({"project": p}));
    assert_eq!(list.as_array().unwrap().len(), 1);
    let s = w.ok(reader, "budget.snapshots", json!({"project": p, "id": snap}));
    let steel_row = s["rows"].as_array().unwrap().iter().find(|r| r["cost_code"] == steel).unwrap().clone();
    assert_eq!(steel_row["original"], "50000.00", "the snapshot keeps the old figure");
    let other = w.project("Other");
    w.err(pm, "budget.snapshots", json!({"project": other, "id": snap}));
    w.err(eng, "budget.delete_snapshot", json!({"project": p, "id": snap}));
    w.ok(pm, "budget.delete_snapshot", json!({"project": p, "id": snap}));
    assert!(w.ok(reader, "budget.snapshots", json!({"project": p})).as_array().unwrap().is_empty());
    w.assert_replay();
}
