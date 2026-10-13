//! Bidding: invite, record bids, compare, award.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::cmd::bidding::spread;
use buildkiosk_model::bidding::BidStatus;
use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn invite_bid_compare_and_award() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"bidding": "standard"}}));
    let concrete = w.engine.state().cost_codes.iter().find(|c| c.code == "03-300").unwrap().id.raw();
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(w.admin(), "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let c = w.ok(w.admin(), "vendor.create", json!({"name": "Summit Flatwork", "kind": "subcontractor"}))["id"].as_u64().unwrap();

    let id = w.ok(
        eng,
        "bid_package.create",
        json!({"project": p, "title": "Concrete", "scope": "Footings and slab", "due": "2026-11-01", "cost_code": concrete, "invite": [a, b]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(eng, "bid_package.create", json!({"project": p}));
    w.err(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "100"}));
    w.ok(eng, "bid_package.open", json!({"project": p, "id": id}));
    w.err(eng, "bid_package.open", json!({"project": p, "id": id}));

    w.ok(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "120,000", "notes": "Excludes rebar"}));
    w.ok(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": b, "amount": "98000.50"}));
    w.ok(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": c, "amount": "130000"}));
    w.ok(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "115000"}));
    w.err(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "0"}));
    w.err(eng, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "-5"}));
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert_eq!((pkg.bids.len(), pkg.invited.len()), (3, 3), "a new bid replaces the old; an uninvited bidder is added");
    let (low, avg, high) = spread(&pkg).unwrap();
    assert_eq!((low, high, avg), (Money::parse("98000.50").unwrap(), Money::parse("130000").unwrap(), Money::parse("114333.50").unwrap()));

    w.ok(eng, "bid_package.withdraw_bid", json!({"project": p, "id": id, "vendor": c}));
    w.err(eng, "bid_package.withdraw_bid", json!({"project": p, "id": id, "vendor": c}));
    assert_eq!(w.ok(eng, "bid_package.get", json!({"project": p, "id": id}))["standing"].as_array().unwrap().len(), 2);
    w.ok(eng, "bid_package.close", json!({"project": p, "id": id}));
    w.ok(eng, "bid_package.reopen", json!({"project": p, "id": id}));
    w.ok(eng, "bid_package.close", json!({"project": p, "id": id}));

    // Only admins award, and only to a company with a standing bid.
    w.err(eng, "bid_package.award", json!({"project": p, "id": id, "vendor": b}));
    w.err(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": c}));
    let out = w.ok(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": b, "commitment": true}));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().status, BidStatus::Awarded);
    let commitment = w.engine.state().commitments.iter().next().unwrap().clone();
    assert_eq!((commitment.vendor.map(|v| v.raw()), commitment.total().unwrap()), (Some(b), Money::parse("98000.50").unwrap()));
    assert_eq!(out["commitment"], commitment.id.raw());
    w.err(eng, "bid_package.update", json!({"project": p, "id": id, "title": "x"}));
    w.err(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": a}));

    let other = w.project("Other");
    w.err(pm, "bid_package.get", json!({"project": other, "id": id}));
    w.assert_replay();
}
