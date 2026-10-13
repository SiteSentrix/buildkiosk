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

#[test]
fn forecasting_methods() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"budget": "standard"}}));
    let concrete = code(&w, "03-300");
    let steel = code(&w, "05-100");
    w.ok(pm, "budget.import", json!({"project": p, "text": "03-300, 100000\n05-100, 50000"}));
    w.ok(pm, "direct_cost.create", json!({"project": p, "cost_code": concrete, "amount": "30000", "description": "Slab pour", "date": "2026-10-01"}));
    let projected = |w: &World, c: u64| budget_rows(w.engine.state(), p).unwrap().iter().find(|r| r.cost_code.raw() == c).unwrap().projected();

    // The default method assumes the budget is spent.
    assert_eq!(projected(&w, concrete), money("100000"));
    assert_eq!(projected(&w, steel), money("50000"));

    // Committed: only what is spent counts; a code with nothing spent projects to nothing.
    w.err(eng, "budget.set_method", json!({"project": p, "method": "committed"}));
    w.err(pm, "budget.set_method", json!({"project": p, "method": "sideways"}));
    w.err(pm, "budget.set_method", json!({"project": p}));
    w.ok(pm, "budget.set_method", json!({"project": p, "method": "committed"}));
    assert_eq!(projected(&w, concrete), money("30000"));
    assert_eq!(projected(&w, steel), Money::ZERO);
    // An open change event is possible cost: the committed method counts it, the others leave it out.
    let ev = w.ok(
        pm,
        "change_event.create",
        json!({"project": p, "title": "Thicker slab", "reason": "Field condition", "lines": [{"cost_code": concrete, "amount": "7500"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(projected(&w, concrete), money("37500"));
    w.ok(pm, "change_event.close", json!({"project": p, "id": ev}));
    assert_eq!(projected(&w, concrete), money("30000"));
    // A forecast still wins over the method.
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": "10000"}));
    assert_eq!(projected(&w, concrete), money("40000"));
    w.ok(eng, "budget.set_forecast", json!({"project": p, "cost_code": concrete, "forecast_to_complete": ""}));

    // Percent complete: 30,000 spent at 40% done projects to 75,000; no percent falls back to committed.
    w.ok(pm, "budget.set_method", json!({"project": p, "method": "progress"}));
    assert_eq!(projected(&w, concrete), money("30000"));
    w.err(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "101"}));
    w.err(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "-5"}));
    w.ok(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "40"}));
    assert_eq!(projected(&w, concrete), money("75000"));
    // Done is done: 100% projects to the cost so far; 0% is the same as none.
    w.ok(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "100"}));
    assert_eq!(projected(&w, concrete), money("30000"));
    w.ok(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "0"}));
    assert_eq!(projected(&w, concrete), money("30000"));
    w.ok(eng, "budget.set_progress", json!({"project": p, "cost_code": concrete, "percent_complete": "40"}));
    // The query reports the figures and a snapshot keeps the percent.
    let got = w.ok(eng, "budget.get", json!({"project": p}));
    let row = got["rows"].as_array().unwrap().iter().find(|r| r["cost_code"] == concrete).unwrap();
    assert_eq!((row["projected"].as_str().unwrap(), row["percent_complete"].as_str().unwrap()), ("75000.00", "40"));
    // Back to the default.
    w.ok(pm, "budget.set_method", json!({"project": p, "method": "budget"}));
    assert_eq!(projected(&w, concrete), money("100000"));
    w.assert_replay();
}

#[test]
fn budget_views() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let other = w.project("Harbor Office");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let template = buildkiosk_engine::cmd::company::template_named(w.engine.state(), "Project manager").unwrap();
    w.ok(w.admin(), "member.add", json!({"project": other, "person": pm, "template": template}));
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "member.update", json!({"project": p, "person": reader, "overrides": {"budget": "read"}}));

    w.err(reader, "budget.view_save", json!({"project": p, "name": "Cash", "columns": "original"}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "", "columns": "original"}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "Cash"}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "Cash", "columns": ""}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "Cash", "columns": "original, sideways"}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "Cash", "columns": "original", "group": "diagonal"}));
    w.err(pm, "budget.view_save", json!({"project": p, "name": "Cash", "columns": [1, 2]}));

    // Columns come back in table order whatever order and spelling they were given in, without duplicates.
    let id = w.ok(
        pm,
        "budget.view_save",
        json!({"project": p, "name": "Cash", "columns": "Projected cost, original, committed, original", "group": "division"}),
    )["id"]
        .as_u64()
        .unwrap();
    let list = w.ok(reader, "budget.views", json!({"project": p}));
    assert_eq!(list[0]["columns"], json!(["original", "committed", "projected"]));
    assert_eq!(list[0]["group"], "division");
    // A name belongs to one view; another project has its own.
    w.err(pm, "budget.view_save", json!({"project": p, "name": "cash", "columns": "original"}));
    w.ok(pm, "budget.view_save", json!({"project": other, "name": "Cash", "columns": ["original"]}));
    // Changing a view keeps its id; another project's id is not found.
    w.ok(pm, "budget.view_save", json!({"project": p, "id": id, "name": "Cash flow", "columns": ["revised", "variance"]}));
    let list = w.ok(reader, "budget.views", json!({"project": p}));
    assert_eq!(
        (list.as_array().unwrap().len(), list[0]["name"].as_str().unwrap(), list[0]["columns"].clone(), list[0]["group"].as_str().unwrap()),
        (1, "Cash flow", json!(["revised", "variance"]), "none")
    );
    w.err(pm, "budget.view_save", json!({"project": other, "id": id, "name": "x", "columns": "original"}));
    w.err(pm, "budget.view_delete", json!({"project": other, "id": id}));
    w.err(reader, "budget.view_delete", json!({"project": p, "id": id}));
    w.ok(pm, "budget.view_delete", json!({"project": p, "id": id}));
    assert_eq!(w.ok(reader, "budget.views", json!({"project": p})).as_array().unwrap().len(), 0);
    w.err(pm, "budget.view_save", json!({"project": p, "id": id, "name": "Back", "columns": "original"}));
    w.assert_replay();
}

#[test]
fn divisions_of_cost_codes() {
    use buildkiosk_engine::view::code_division;
    assert_eq!(code_division("03-300"), "03");
    assert_eq!(code_division("03.300"), "03");
    assert_eq!(code_division("03 30 00"), "03");
    assert_eq!(code_division("A/12"), "A");
    assert_eq!(code_division("0330"), "0330");
    assert_eq!(code_division("-5"), "-5");
    assert_eq!(code_division(""), "");
    assert_eq!(code_division("  07-100 "), "07");
}
