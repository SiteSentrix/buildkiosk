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

const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SHA2: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

fn package(w: &World, id: u64) -> buildkiosk_model::bidding::BidPackage {
    w.engine.state().bid_packages.get(buildkiosk_core::Id::new(id)).unwrap().clone()
}

#[test]
fn a_bid_form_prices_lines_alternates_and_scope() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let a = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(admin, "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w
        .ok(
            pm,
            "bid_package.create",
            json!({"project": p, "title": "Concrete", "invite": [a, b],
                   "scope_items": [{"text": "Footings"}, {"text": "Slab on grade"}, {"text": "Curing and sealing"}, {"text": ""}],
                   "form_items": [{"description": "Footings", "unit": "cy", "quantity": "40"}, {"description": "Slab", "unit": "sf", "quantity": "1,000"}],
                   "alternates": [{"name": "Polished finish"}, {"name": "Deduct vapour barrier"}],
                   "bond_percent": "5"}),
        )["id"]
        .as_u64()
        .unwrap();
    let pkg = package(&w, id);
    assert_eq!((pkg.scope_items.len(), pkg.form_items.len(), pkg.alternates.len(), pkg.form_items[1].quantity_milli), (3, 2, 2, 1_000_000));
    assert_eq!(pkg.bond_required.map(|x| x.plain()), Some("5".to_string()));
    for bad in [
        json!({"bond_percent": "0"}),
        json!({"bond_percent": "101"}),
        json!({"form_items": [{"description": "", "quantity": "5"}]}),
        json!({"form_items": [{"description": "x", "quantity": "0"}]}),
    ] {
        let mut params = json!({"project": p, "id": id});
        for (k, v) in bad.as_object().unwrap() {
            params[k] = v.clone();
        }
        w.err(pm, "bid_package.update", params);
    }
    w.ok(pm, "bid_package.open", json!({"project": p, "id": id}));

    // Unit prices for every line make the amount; alternates and scope answers must refer to what exists.
    w.err(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "lines": [{"item": 0, "unit_price": "300"}]}));
    w.err(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "lines": [{"item": 0, "unit_price": "300"}, {"item": 5, "unit_price": "9"}]}),
    );
    w.err(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "lines": [{"item": 0, "unit_price": "300"}, {"item": 0, "unit_price": "9"}]}),
    );
    w.err(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "5", "alternates": [{"index": 9, "amount": "1"}]}));
    w.err(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "5", "scope": [{"item": 9, "answer": "included"}]}));
    w.err(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "5", "scope": [{"item": 0, "answer": "maybe"}]}));
    w.err(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "amount": "5", "scope": [{"item": 0, "answer": "excluded", "adjustment": "-3"}]}),
    );
    let r = w.ok(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "lines": [{"item": 0, "unit_price": "300"}, {"item": 1, "unit_price": "9.50"}],
               "alternates": [{"index": 0, "amount": "4000"}, {"index": 1, "amount": "-1500"}],
               "scope": [{"item": 0, "answer": "included"}, {"item": 1, "answer": "included"}, {"item": 2, "answer": "excluded", "adjustment": "2500"}],
               "bond_received": true, "bond_amount": "1075"}),
    );
    assert_eq!(r["amount"], "21500.00", "40 cy x 300 + 1,000 sf x 9.50");
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": b, "amount": "23000", "scope": [{"item": 2, "answer": "included"}]}));
    let lvl = w.ok(pm, "bid_package.level", json!({"project": p, "id": id}));
    let rows = lvl["rows"].as_array().unwrap();
    // Keystone is cheaper as bid (21,500) but 24,000 once the missing sealing is added: Northline's 23,000 is lower.
    assert_eq!((rows[0]["company"].as_str(), rows[0]["leveled"].as_str()), (Some("Northline Concrete"), Some("23000.00")));
    assert_eq!(
        (rows[1]["company"].as_str(), rows[1]["adjustments"].as_str(), rows[1]["leveled"].as_str()),
        (Some("Keystone Concrete"), Some("2500.00"), Some("24000.00"))
    );
    assert_eq!((rows[1]["bond_ok"].as_bool(), rows[0]["bond_ok"].as_bool()), (Some(true), Some(false)), "5% of 21,500 is 1,075; Northline sent none");
    // Once bids are in, the structure they refer to is fixed.
    w.err(pm, "bid_package.update", json!({"project": p, "id": id, "scope_items": [{"text": "Different"}]}));
    // A replacement bid keeps the files the company sent.
    w.ok(pm, "bid_package.attach", json!({"project": p, "id": id, "vendor": a, "sha256": SHA, "name": "proposal.pdf", "size": 10}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "21000", "bond_received": true}));
    assert_eq!(package(&w, id).bids.iter().find(|x| x.vendor.raw() == a).unwrap().files.len(), 1);
    // The award waits for the bond.
    w.err(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": b}));
    w.ok(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": a}));
    w.assert_replay();
}

#[test]
fn addenda_and_files() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let admin = w.admin();
    let a = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let outsider = w.ok(admin, "vendor.create", json!({"name": "Not Invited", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "invite": [a]}))["id"].as_u64().unwrap();
    w.err(reader, "bid_package.addendum", json!({"project": p, "id": id, "title": "x"}));
    w.err(pm, "bid_package.addendum", json!({"project": p, "id": id}));
    w.ok(pm, "bid_package.attach", json!({"project": p, "id": id, "sha256": SHA, "name": "scope.pdf", "size": 10, "mime": "application/pdf"}));
    w.ok(pm, "bid_package.attach", json!({"project": p, "id": id, "sha256": SHA, "name": "scope.pdf", "size": 10}));
    w.err(pm, "bid_package.attach", json!({"project": p, "id": id, "sha256": "short", "name": "x", "size": 1}));
    assert_eq!(package(&w, id).attachments.len(), 1);
    w.ok(pm, "bid_package.open", json!({"project": p, "id": id}));
    let n1 = w.ok(pm, "bid_package.addendum", json!({"project": p, "id": id, "title": "Slab thickness changed", "body": "Now 6 inches."}))["number"]
        .as_u64()
        .unwrap();
    let n2 = w.ok(pm, "bid_package.addendum", json!({"project": p, "id": id, "title": "Due date moved"}))["number"].as_u64().unwrap();
    assert_eq!((n1, n2), (1, 2));
    assert_eq!(package(&w, id).unacknowledged(buildkiosk_core::Id::new(a)), vec![1, 2]);
    w.err(pm, "bid_package.acknowledge", json!({"project": p, "id": id, "vendor": a, "number": 9}));
    w.err(pm, "bid_package.acknowledge", json!({"project": p, "id": id, "vendor": outsider, "number": 1}));
    w.ok(pm, "bid_package.acknowledge", json!({"project": p, "id": id, "vendor": a, "number": 1}));
    w.err(pm, "bid_package.acknowledge", json!({"project": p, "id": id, "vendor": a, "number": 1}));
    assert_eq!(package(&w, id).unacknowledged(buildkiosk_core::Id::new(a)), vec![2]);
    // Files of a bid.
    w.err(pm, "bid_package.attach", json!({"project": p, "id": id, "vendor": a, "sha256": SHA2, "name": "proposal.pdf", "size": 5}));
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": a, "amount": "100"}));
    w.ok(pm, "bid_package.attach", json!({"project": p, "id": id, "vendor": a, "sha256": SHA2, "name": "proposal.pdf", "size": 5}));
    w.ok(pm, "bid_package.detach", json!({"project": p, "id": id, "vendor": a, "sha256": SHA2}));
    w.err(pm, "bid_package.detach", json!({"project": p, "id": id, "vendor": a, "sha256": SHA2}));
    w.ok(pm, "bid_package.detach", json!({"project": p, "id": id, "sha256": SHA}));
    // Awarded packages are closed to addenda and package files.
    w.ok(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": a}));
    w.err(pm, "bid_package.addendum", json!({"project": p, "id": id, "title": "Late"}));
    w.err(pm, "bid_package.attach", json!({"project": p, "id": id, "sha256": SHA, "name": "late.pdf", "size": 1}));
    w.assert_replay();
}
