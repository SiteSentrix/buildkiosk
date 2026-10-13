//! Estimating: priced lines, markups, issue, put into the budget.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::cmd::estimate::parse_quantity;
use buildkiosk_testkit::World;
use serde_json::json;

fn code(w: &World, c: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|x| x.code == c).unwrap().id.raw()
}

#[test]
fn quantities() {
    assert_eq!(parse_quantity("2.5"), Some(2500));
    assert_eq!(parse_quantity("1,250"), Some(1_250_000));
    assert_eq!(parse_quantity("0.001"), Some(1));
    assert_eq!(parse_quantity(".5"), Some(500));
    assert_eq!(parse_quantity("1.2345"), None);
    assert_eq!(parse_quantity("abc"), None);
    assert_eq!(parse_quantity(""), None);
}

#[test]
fn price_issue_and_apply() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"estimating": "standard"}}));
    let (concrete, steel, general) = (code(&w, "03-300"), code(&w, "05-100"), code(&w, "01-100"));

    let id = w.ok(
        eng,
        "estimate.create",
        json!({"project": p, "name": "Schematic estimate",
               "lines": [
                   {"cost_code": concrete, "description": "Slab on grade", "quantity": "1,250.5", "unit": "sf", "unit_cost": "8.25"},
                   {"cost_code": concrete, "description": "Footings", "quantity": "40", "unit": "cy", "unit_cost": "310"},
                   {"cost_code": steel, "description": "Structural steel", "quantity": "", "unit": "ls", "unit_cost": "84,000"},
                   {"cost_code": null, "description": "", "quantity": "", "unit_cost": ""}
               ],
               "markups": [{"name": "Overhead", "percent": "5"}, {"name": "Profit", "percent": "8.5"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let got = w.ok(eng, "estimate.get", json!({"project": p, "id": id}));
    // 1250.5 × 8.25 = 10,316.625 → 10,316.63; 40 × 310 = 12,400; steel 84,000.
    assert_eq!(got["totals"]["subtotal"], "106716.63");
    assert_eq!(got["totals"]["markups"][0], "5335.83");
    assert_eq!(got["totals"]["markups"][1], "9070.91");
    assert_eq!(got["totals"]["total"], "121123.37");

    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": concrete, "quantity": "x", "unit_cost": "1"}]}));
    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": concrete, "unit_cost": ""}]}));
    w.err(eng, "estimate.create", json!({"project": p, "name": "Bad", "lines": [{"cost_code": 9999, "unit_cost": "1"}]}));
    w.err(
        eng,
        "estimate.create",
        json!({"project": p, "name": "Big", "lines": [{"cost_code": concrete, "quantity": "999999999", "unit_cost": "9,000,000,000,000"}]}),
    );
    w.err(eng, "estimate.apply_to_budget", json!({"project": p, "id": id}));
    w.err(eng, "estimate.finalize", json!({"project": p, "id": w.engine.state().estimates.iter().next().unwrap().id.raw() + 9}));
    w.ok(eng, "estimate.finalize", json!({"project": p, "id": id}));
    w.err(eng, "estimate.update", json!({"project": p, "id": id, "name": "x"}));
    w.err(eng, "estimate.apply_to_budget", json!({"project": p, "id": id}));

    // Putting it in the budget: per code, markups on a chosen code, refused when locked.
    w.ok(pm, "estimate.apply_to_budget", json!({"project": p, "id": id, "markup_cost_code": general}));
    let rows = w.ok(pm, "budget.get", json!({"project": p}));
    let amount = |c: u64| rows["rows"].as_array().unwrap().iter().find(|r| r["cost_code"] == c).map(|r| r["original"].clone()).unwrap();
    assert_eq!((amount(concrete), amount(steel), amount(general)), (json!("22716.63"), json!("84000.00"), json!("14406.74")));
    w.ok(pm, "budget.lock", json!({"project": p}));
    w.err(pm, "estimate.apply_to_budget", json!({"project": p, "id": id}));
    w.ok(pm, "budget.unlock", json!({"project": p}));
    w.ok(eng, "estimate.reopen", json!({"project": p, "id": id}));
    w.ok(eng, "estimate.update", json!({"project": p, "id": id, "markups": []}));
    assert_eq!(w.engine.state().estimates.iter().next().unwrap().totals().unwrap().total, Money::parse("106716.63").unwrap());
    assert_eq!(w.ok(eng, "estimate.list", json!({"project": p})).as_array().unwrap().len(), 1);
    w.ok(eng, "estimate.delete", json!({"project": p, "id": id}));
    w.ok(pm, "estimate.restore", json!({"project": p, "id": id}));
    w.assert_replay();
}
