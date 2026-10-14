//! Bidding: invite, record bids, compare, award.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::Actor;
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

#[test]
fn weighted_scoring_and_the_size_of_the_bond() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    let admin = w.admin();
    let a = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(admin, "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id =
        w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "invite": [a, b], "bond_percent": "10"}))["id"].as_u64().unwrap();
    w.ok(pm, "bid_package.open", json!({"project": p, "id": id}));

    // Criteria: weights add up to 100, one can be the price, no repeats.
    for bad in
        ["Price, 50, price\nSafety record, 30", "Price, 100, price\nPrice, 0", "A, 60\nB, 40, cheap", "A, 50, price\nB, 50, price", "A, 0", ", 100"]
    {
        w.err(pm, "bid_package.set_criteria", json!({"project": p, "id": id, "text": bad}));
    }
    w.err(reader, "bid_package.set_criteria", json!({"project": p, "id": id, "text": "A, 100"}));
    w.err(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": a, "scores": {"1": 5}}));
    w.ok(pm, "bid_package.set_criteria", json!({"project": p, "id": id, "text": "Price, 50, price\nSafety record, 30\nSchedule, 20"}));

    // Bids: a bond below 10% of the bid is refused when recorded; the right one is taken.
    w.err(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "amount": "100000", "bond_received": true, "bond_amount": "5000"}),
    );
    w.ok(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "amount": "100000", "bond_received": true, "bond_amount": "10000"}),
    );
    w.ok(pm, "bid_package.record_bid", json!({"project": p, "id": id, "vendor": b, "amount": "125000", "bond_received": true}));

    // The price criterion is worked out (low bid 10.00; the other 100000/125000 of that = 8.00); the rest is scored 0 to 10.
    let level = |w: &mut World| w.ok(pm, "bid_package.level", json!({"project": p, "id": id}));
    assert!(level(&mut w)["rows"][0]["weighted"].is_null(), "unscored criteria leave the total open");
    w.err(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": a, "scores": {"1": 5}}));
    w.err(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": a, "scores": {"2": 11}}));
    w.err(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": a, "scores": {"4": 5}}));
    w.err(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": 9999, "scores": {"2": 5}}));
    w.ok(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": a, "scores": {"2": 6, "3": 8}}));
    w.ok(pm, "bid_package.score", json!({"project": p, "id": id, "vendor": b, "scores": {"2": 10, "3": 9}}));
    let pkg = package(&w, id);
    let total = |v: u64| pkg.weighted(pkg.bids.iter().find(|x| x.vendor.raw() == v).unwrap());
    // Keystone: 50*10 + 30*6 + 20*8 = 500+180+160 = 840 / 100 -> 8.40; Northline: 50*8 + 30*10 + 20*9 = 400+300+180 -> 8.80.
    assert_eq!((total(a), total(b)), (Some(840), Some(880)));
    let rows = level(&mut w);
    let weighted: Vec<f64> = rows["rows"].as_array().unwrap().iter().map(|r| r["weighted"].as_f64().unwrap()).collect();
    assert_eq!(weighted, vec![8.4, 8.8], "cheapest first, each with its total");

    // A revised bid keeps the scores; changing the criteria clears them.
    w.ok(
        pm,
        "bid_package.record_bid",
        json!({"project": p, "id": id, "vendor": a, "amount": "110000", "bond_received": true, "bond_amount": "11000"}),
    );
    assert_eq!(package(&w, id).bids.iter().find(|x| x.vendor.raw() == a).unwrap().scores.len(), 2);
    // An award is refused while the recorded bond is short of the bid (Northline gave none: amount unknown is allowed).
    w.ok(pm, "bid_package.award", json!({"project": p, "id": id, "vendor": b}));
    w.err(pm, "bid_package.set_criteria", json!({"project": p, "id": id, "text": "Price, 100, price"}));
    w.assert_replay();
}

#[test]
fn bid_links_are_invited_resent_and_cancelled() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rory", "Reader", "Read only");
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(w.admin(), "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "invite": [a]}))["id"].as_u64().unwrap();
    let args = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };

    // A link needs a valid address, a real company and a future expiry; only people with Bidding access can send one.
    w.err(pm, "bid_package.invite", args(json!({"vendor": a, "email": "not an address"})));
    w.err(pm, "bid_package.invite", args(json!({"vendor": a})));
    w.err(pm, "bid_package.invite", args(json!({"vendor": 9999, "email": "x@example.com"})));
    w.err(pm, "bid_package.invite", args(json!({"vendor": a, "email": "x@example.com", "expires": "2000-01-01"})));
    w.err(reader, "bid_package.invite", args(json!({"vendor": a, "email": "x@example.com"})));

    let first = w.ok(pm, "bid_package.invite", args(json!({"vendor": a, "email": "Estimator@Keystone.example", "expires": "2099-01-01"})));
    assert_eq!(first["number"], 1);
    w.err(pm, "bid_package.invite", args(json!({"vendor": a, "email": "estimator@keystone.example"})));
    // A second contact at the company, and a company not yet on the invited list (which is added to it).
    assert_eq!(w.ok(pm, "bid_package.invite", args(json!({"vendor": a, "email": "owner@keystone.example"})))["number"], 2);
    assert_eq!(w.ok(pm, "bid_package.invite", args(json!({"vendor": b, "email": "bids@northline.example"})))["number"], 3);
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert_eq!(pkg.invited.len(), 2);
    assert_eq!(pkg.invitations[0].email, "estimator@keystone.example", "addresses are lowercased");
    assert!(pkg.invitations.iter().all(|i| i.sent.is_none() && i.generation == 0));

    // The list never carries a secret, and says what state each link is in.
    let list = w.ok(pm, "bid_package.invitations", args(json!({})));
    assert_eq!(list["invitations"].as_array().unwrap().len(), 3);
    assert_eq!(list["invitations"][0]["state"], "waiting to be sent");
    assert!(!list.to_string().contains("secret") && !list.to_string().contains("token"));
    w.err(reader, "bid_package.resend_invite", args(json!({"number": 1})));

    // The server marks a link sent only for the generation it emailed; sending again bumps the generation.
    w.ok(pm, "bid_package.resend_invite", args(json!({"number": 1, "email": "new@keystone.example"})));
    w.ok(Actor::System, "bid_package.invite_sent", args(json!({"number": 1, "generation": 0})));
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert!(pkg.invitations[0].sent.is_none(), "an email for the old link doesn't count");
    assert_eq!((pkg.invitations[0].generation, pkg.invitations[0].email.as_str()), (1, "new@keystone.example"));
    w.ok(Actor::System, "bid_package.invite_sent", args(json!({"number": 1, "generation": 1})));
    assert!(w.engine.state().bid_packages.iter().next().unwrap().invitations[0].sent.is_some());
    w.err(pm, "bid_package.invite_sent", args(json!({"number": 1, "generation": 1})));

    // Cancelling is final: it can't be repeated or resent, and a new link can go to the same address.
    w.ok(pm, "bid_package.revoke_invite", args(json!({"number": 2})));
    w.err(pm, "bid_package.revoke_invite", args(json!({"number": 2})));
    w.err(pm, "bid_package.resend_invite", args(json!({"number": 2})));
    w.err(pm, "bid_package.revoke_invite", args(json!({"number": 99})));
    assert_eq!(w.ok(pm, "bid_package.invite", args(json!({"vendor": a, "email": "owner@keystone.example"})))["number"], 4);
    let list = w.ok(pm, "bid_package.invitations", args(json!({})));
    assert_eq!(list["invitations"][1]["state"], "cancelled");

    // Links stop going out once bidding has closed.
    w.ok(pm, "bid_package.open", args(json!({})));
    w.ok(pm, "bid_package.close", args(json!({})));
    w.err(pm, "bid_package.invite", args(json!({"vendor": b, "email": "late@northline.example"})));
    w.err(pm, "bid_package.resend_invite", args(json!({"number": 1})));
    w.assert_replay();
}

#[test]
fn bidders_use_their_links_and_sealed_bids_stay_unseen_until_closing() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"bidding": "standard"}}));
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(w.admin(), "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(
        pm,
        "bid_package.create",
        json!({"project": p, "title": "Concrete", "scope": "Footings", "due": "2099-01-01",
                   "form_items": [{"description": "Footings", "unit": "cy", "quantity": "10"}], "alternates": [{"name": "Colored slab"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let args = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    let link = |n: u32, g: u32, extra: serde_json::Value| {
        args(json!({"invitation": n, "generation": g}))
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .chain(extra.as_object().unwrap().clone())
            .collect::<serde_json::Value>()
    };
    w.ok(pm, "bid_package.invite", args(json!({"vendor": a, "email": "bids@keystone.example"})));
    w.ok(pm, "bid_package.invite", args(json!({"vendor": b, "email": "bids@northline.example"})));
    w.ok(pm, "bid_package.addendum", args(json!({"title": "Footing depth"})));

    // Nobody but the server (the system) can run the portal commands, and nothing works before bidding opens.
    w.err(pm, "bid_package.portal_view", link(1, 0, json!({})));
    w.err(w.admin(), "bid_package.portal_submit", link(1, 0, json!({"amount": "5"})));
    w.ok(Actor::System, "bid_package.portal_view", link(1, 0, json!({})));
    assert!(!w.ok(Actor::System, "bid_package.portal_view", link(1, 0, json!({})))["taking_bids"].as_bool().unwrap());
    w.err(Actor::System, "bid_package.portal_submit", link(1, 0, json!({"lines": [{"item": 0, "unit_price": "100"}]})));
    w.ok(pm, "bid_package.open", args(json!({})));

    // A bidder sees their own package and nothing about the other company.
    let view = w.ok(Actor::System, "bid_package.portal_view", link(1, 0, json!({})));
    assert_eq!((view["company"].as_str().unwrap(), view["taking_bids"].as_bool().unwrap()), ("Keystone Concrete", true));
    assert!(!view.to_string().contains("Northline") && !view.to_string().contains("northline"));
    assert!(view["bid"].is_null());

    // Wrong or unknown links all give the same answer.
    let same = w.err(Actor::System, "bid_package.portal_view", link(1, 7, json!({}))).to_string();
    assert_eq!(same, w.err(Actor::System, "bid_package.portal_view", link(99, 0, json!({}))).to_string());
    assert_eq!(same, w.err(Actor::System, "bid_package.portal_submit", link(1, 7, json!({"amount": "5"}))).to_string());

    // Submitting: validated like a team bid, and the lines make the total.
    w.err(Actor::System, "bid_package.portal_submit", link(1, 0, json!({"lines": [{"item": 3, "unit_price": "100"}]})));
    w.err(Actor::System, "bid_package.portal_submit", link(1, 0, json!({"amount": "0"})));
    let out = w.ok(
        Actor::System,
        "bid_package.portal_submit",
        link(1, 0, json!({"lines": [{"item": 0, "unit_price": "1,200"}], "notes": "Excludes rebar", "alternates": [{"index": 0, "amount": "900"}]})),
    );
    assert_eq!(out["reference"], "BP1-1");
    w.ok(Actor::System, "bid_package.portal_submit", link(2, 0, json!({"lines": [{"item": 0, "unit_price": "1100"}]})));
    w.ok(Actor::System, "bid_package.portal_ack", link(1, 0, json!({"number": 1})));
    w.ok(Actor::System, "bid_package.portal_ack", link(1, 0, json!({"number": 1})));
    w.err(Actor::System, "bid_package.portal_ack", link(1, 0, json!({"number": 9})));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().acknowledgements.len(), 1);

    // The team sees that two bids arrived but not what they say, in any command.
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert!(pkg.bids.is_empty() && pkg.sealed_bids.len() == 2);
    for shown in [
        w.ok(pm, "bid_package.get", args(json!({}))),
        w.ok(pm, "bid_package.list", json!({"project": p})),
        w.ok(pm, "bid_package.level", args(json!({}))),
    ] {
        let text = shown.to_string();
        assert!(!text.contains("12000.00") && !text.contains("11000.00") && !text.contains("Excludes rebar"), "{text}");
    }
    assert_eq!(w.ok(pm, "bid_package.get", args(json!({})))["sealed_waiting"].as_array().unwrap().len(), 2);
    w.err(pm, "bid_package.award", args(json!({"vendor": a})));
    w.err(eng, "bid_package.unseal", args(json!({})));

    // A bidder can revise before the due day, and withdraw; a cancelled or replaced link stops working at once.
    w.ok(Actor::System, "bid_package.portal_submit", link(1, 0, json!({"lines": [{"item": 0, "unit_price": "1000"}]})));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().sealed_bids.len(), 2, "a revision replaces the earlier bid");
    w.ok(Actor::System, "bid_package.portal_withdraw", link(2, 0, json!({})));
    w.err(Actor::System, "bid_package.portal_withdraw", link(2, 0, json!({})));
    w.ok(pm, "bid_package.resend_invite", args(json!({"number": 2})));
    w.err(Actor::System, "bid_package.portal_submit", link(2, 0, json!({"amount": "1"})));
    w.ok(Actor::System, "bid_package.portal_submit", link(2, 1, json!({"lines": [{"item": 0, "unit_price": "1050"}]})));
    w.ok(pm, "bid_package.revoke_invite", args(json!({"number": 2})));
    w.err(Actor::System, "bid_package.portal_submit", link(2, 1, json!({"amount": "1"})));

    // Closing opens the sealed bids; after that the bidder can't change anything.
    let closed = w.ok(pm, "bid_package.close", args(json!({})));
    assert_eq!(closed["opened"], 2);
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert!(pkg.sealed_bids.is_empty());
    assert_eq!(pkg.bids.len(), 2);
    assert!(pkg.bids.iter().all(|x| x.via.is_some()));
    assert_eq!(pkg.standing().first().unwrap().amount, Money::parse("10000").unwrap());
    w.err(Actor::System, "bid_package.portal_submit", link(1, 0, json!({"amount": "1"})));
    w.err(Actor::System, "bid_package.portal_withdraw", link(1, 0, json!({})));
    w.assert_replay();
}

#[test]
fn unsealed_packages_and_late_bids() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"bidding": "standard"}}));
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "sealed": false, "due": "2026-10-20", "invite": [a]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "bid_package.invite", json!({"project": p, "id": id, "vendor": a, "email": "bids@keystone.example"}));
    w.ok(pm, "bid_package.open", json!({"project": p, "id": id}));
    let link = json!({"project": p, "id": id, "invitation": 1, "generation": 0, "amount": "5000"});
    // Not sealed: the team sees the bid at once.
    w.ok(Actor::System, "bid_package.portal_submit", link.clone());
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().bids.len(), 1);
    // Sealed bids can be opened early by an admin only.
    w.ok(pm, "bid_package.update", json!({"project": p, "id": id, "sealed": true}));
    w.ok(Actor::System, "bid_package.portal_submit", json!({"project": p, "id": id, "invitation": 1, "generation": 0, "amount": "4800"}));
    w.err(eng, "bid_package.unseal", json!({"project": p, "id": id}));
    w.err(pm, "bid_package.update", json!({"project": p, "id": id, "sealed": false}));
    let admin = w.admin();
    assert_eq!(w.ok(admin, "bid_package.unseal", json!({"project": p, "id": id}))["opened"], 1);
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().bids[0].amount, Money::parse("4800").unwrap());
    // Past the due day nothing goes through until the team moves the date.
    w.advance_days(60);
    w.err(Actor::System, "bid_package.portal_submit", link.clone());
    w.ok(pm, "bid_package.update", json!({"project": p, "id": id, "due": "2099-01-01"}));
    w.ok(Actor::System, "bid_package.portal_submit", link);
    // A link past its own last day stops working.
    w.ok(pm, "bid_package.invite", json!({"project": p, "id": id, "vendor": a, "email": "late@keystone.example", "expires": "2099-01-02"}));
    w.assert_replay();
}

#[test]
fn bidders_ask_questions_and_the_team_answers() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rory", "Reader", "Read only");
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let b = w.ok(w.admin(), "vendor.create", json!({"name": "Northline Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "due": "2099-01-01"}))["id"].as_u64().unwrap();
    let args = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    w.ok(pm, "bid_package.invite", args(json!({"vendor": a, "email": "bids@keystone.example"})));
    w.ok(pm, "bid_package.invite", args(json!({"vendor": b, "email": "bids@northline.example"})));
    let ask = |n: u32, text: &str| args(json!({"invitation": n, "generation": 0, "text": text}));

    // Questions open with bidding, not before; blank ones and wrong links are refused.
    w.err(Actor::System, "bid_package.portal_ask", ask(1, "What depth?"));
    w.ok(pm, "bid_package.open", args(json!({})));
    w.err(Actor::System, "bid_package.portal_ask", ask(1, "   "));
    w.err(Actor::System, "bid_package.portal_ask", ask(7, "What depth?"));
    w.err(pm, "bid_package.portal_ask", ask(1, "What depth?"));
    assert_eq!(w.ok(Actor::System, "bid_package.portal_ask", ask(1, "What depth are the footings?"))["number"], 1);
    assert_eq!(w.ok(Actor::System, "bid_package.portal_ask", ask(2, "Is rebar by others?"))["number"], 2);

    // The team sees who asked; the other company doesn't see the question at all until it is published.
    let got = w.ok(pm, "bid_package.get", args(json!({})));
    assert_eq!(got["questions"].as_array().unwrap().len(), 2);
    let view = |w: &mut World, n: u32| w.ok(Actor::System, "bid_package.portal_view", args(json!({"invitation": n, "generation": 0})));
    assert_eq!(view(&mut w, 2)["questions"].as_array().unwrap().len(), 1);
    assert!(!view(&mut w, 2).to_string().contains("footings"));

    // Answering needs Bidding access at standard level.
    w.err(reader, "bid_package.answer_question", args(json!({"number": 1, "answer": "Four feet"})));
    w.err(pm, "bid_package.answer_question", args(json!({"number": 1, "answer": ""})));
    w.err(pm, "bid_package.answer_question", args(json!({"number": 9, "answer": "x"})));
    w.ok(pm, "bid_package.answer_question", args(json!({"number": 1, "answer": "Four feet"})));
    let mine = view(&mut w, 1);
    assert_eq!((mine["questions"][0]["answer"].as_str().unwrap(), mine["questions"][0]["answered"].as_bool().unwrap()), ("Four feet", true));
    assert!(!view(&mut w, 2).to_string().contains("Four feet"), "an answer stays private until published");

    // Publishing shows the question and answer to the others, without the asker's name; the team may reword it first.
    w.ok(
        pm,
        "bid_package.answer_question",
        args(json!({"number": 1, "answer": "Four feet", "text": "What depth are the footings at the east wall?", "publish": true})),
    );
    let other = view(&mut w, 2);
    assert_eq!(other["published_answers"].as_array().unwrap().len(), 1);
    assert!(
        other.to_string().contains("east wall") && !other.to_string().contains("Keystone") && !other.to_string().contains("footings are"),
        "{other}"
    );
    assert!(view(&mut w, 1)["published_answers"].as_array().unwrap().is_empty(), "the asker doesn't get their own back as a published one");

    // Issued as an addendum, an answer is published and every company is asked to acknowledge it.
    let out = w.ok(pm, "bid_package.answer_question", args(json!({"number": 2, "answer": "Rebar is by others.", "addendum": true})));
    assert_eq!(out["addendum"], 1);
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert!(pkg.questions[1].published && pkg.addenda[0].body.contains("Rebar is by others."));
    w.ok(Actor::System, "bid_package.portal_ack", args(json!({"invitation": 2, "generation": 0, "number": 1})));

    // Each company can ask only so many; and nothing more once bidding closes.
    for i in 0..49 {
        w.ok(Actor::System, "bid_package.portal_ask", ask(1, &format!("More {i}?")));
    }
    w.err(Actor::System, "bid_package.portal_ask", ask(1, "One too many?"));
    w.ok(pm, "bid_package.close", args(json!({})));
    w.err(Actor::System, "bid_package.portal_ask", ask(2, "Late question?"));
    w.assert_replay();
}

#[test]
fn the_team_is_told_and_bidders_attach_files() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rory", "Reader", "Read only");
    let a = w.ok(w.admin(), "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "due": "2099-01-01"}))["id"].as_u64().unwrap();
    let args = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id, "invitation": 1, "generation": 0});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    let file = |n: u32| json!({"sha256": format!("{n:064x}"), "name": format!("proposal-{n}.pdf"), "size": 1000, "mime": "application/pdf"});
    w.ok(pm, "bid_package.invite", json!({"project": p, "id": id, "vendor": a, "email": "bids@keystone.example"}));
    w.ok(pm, "bid_package.open", json!({"project": p, "id": id}));

    // Files go with a bid, so there must be one first.
    w.err(Actor::System, "bid_package.portal_attach", args(file(1)));
    w.ok(Actor::System, "bid_package.portal_submit", args(json!({"amount": "5000"})));

    // The people who run bidding are told, without the amount; readers aren't.
    let told = |w: &mut World, who: Actor| w.ok(who, "notification.list", json!({})).to_string();
    let admin = w.admin();
    let admin_view = told(&mut w, admin);
    assert!(admin_view.contains("Keystone Concrete sent a bid (sealed until bidding closes)") && !admin_view.contains("5000"), "{admin_view}");
    assert!(told(&mut w, pm.into()).contains("sent a bid"));
    assert!(!told(&mut w, reader.into()).contains("sent a bid"));
    w.ok(Actor::System, "bid_package.portal_ask", args(json!({"text": "What depth?"})));
    assert!(told(&mut w, pm.into()).contains("asked a question"));

    // A bidder attaches files to their own (sealed) bid, up to a limit, and can remove them.
    w.ok(Actor::System, "bid_package.portal_attach", args(file(1)));
    w.ok(Actor::System, "bid_package.portal_attach", args(file(1)));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().sealed_bids[0].files.len(), 1, "the same file twice is one file");
    w.err(pm, "bid_package.portal_attach", args(file(2)));
    w.err(Actor::System, "bid_package.portal_attach", args(json!({"sha256": "short", "name": "x", "size": 1})));
    for n in 2..=10 {
        w.ok(Actor::System, "bid_package.portal_attach", args(file(n)));
    }
    w.err(Actor::System, "bid_package.portal_attach", args(file(11)));
    w.ok(Actor::System, "bid_package.portal_detach", args(json!({"sha256": format!("{:064x}", 10)})));
    w.err(Actor::System, "bid_package.portal_detach", args(json!({"sha256": format!("{:064x}", 10)})));
    let view = w.ok(Actor::System, "bid_package.portal_view", args(json!({})));
    assert_eq!(view["bid"]["files"].as_array().unwrap().len(), 9);
    // The team doesn't see the files until the bid is opened.
    assert!(!w.ok(pm, "bid_package.get", json!({"project": p, "id": id})).to_string().contains("proposal-1.pdf"));
    w.ok(pm, "bid_package.close", json!({"project": p, "id": id}));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().bids[0].files.len(), 9);
    w.err(Actor::System, "bid_package.portal_attach", args(file(12)));
    w.assert_replay();
}

#[test]
fn an_open_link_lets_companies_ask_to_be_invited() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let reader = w.member(p, "Rory", "Reader", "Read only");
    let id = w.ok(pm, "bid_package.create", json!({"project": p, "title": "Concrete", "due": "2099-01-01"}))["id"].as_u64().unwrap();
    let args = |extra: serde_json::Value| {
        let mut v = json!({"project": p, "id": id});
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        v
    };
    let reg = |generation: u32, company: &str, email: &str| {
        args(json!({"generation": generation, "company": company, "contact": "Sam Estimator", "email": email}))
    };

    // Off until the team switches it on; only people with Bidding access can.
    w.err(pm, "bid_package.open", args(json!({})));
    w.err(Actor::System, "bid_package.portal_register", reg(0, "Granite Works", "bids@granite.example"));
    w.err(Actor::System, "bid_package.portal_open_view", args(json!({"generation": 0})));
    w.err(reader, "bid_package.set_public", args(json!({})));
    w.ok(pm, "bid_package.set_public", args(json!({})));
    w.ok(pm, "bid_package.open", args(json!({})));
    let view = w.ok(Actor::System, "bid_package.portal_open_view", args(json!({"generation": 0})));
    assert_eq!((view["contractor"].as_str().unwrap(), view["accepting"].as_bool().unwrap()), ("Summit Ridge Builders", true));

    // Requests are validated, can't repeat an address, and tell the team.
    w.err(Actor::System, "bid_package.portal_register", reg(0, "", "bids@granite.example"));
    w.err(Actor::System, "bid_package.portal_register", reg(0, "Granite Works", "nope"));
    w.err(Actor::System, "bid_package.portal_register", reg(3, "Granite Works", "bids@granite.example"));
    w.err(pm, "bid_package.portal_register", reg(0, "Granite Works", "bids@granite.example"));
    assert_eq!(w.ok(Actor::System, "bid_package.portal_register", reg(0, "Granite Works", "Bids@Granite.example"))["number"], 1);
    w.err(Actor::System, "bid_package.portal_register", reg(0, "Granite Works", "bids@granite.example"));
    w.ok(Actor::System, "bid_package.portal_register", reg(0, "Slab Brothers", "info@slab.example"));
    w.ok(Actor::System, "bid_package.portal_register", reg(0, "Spam Inc", "spam@spam.example"));
    assert!(w.ok(pm, "notification.list", json!({})).to_string().contains("Granite Works asked to be invited to bid"));

    // Approving invites the company (a link goes to the address on the request); declining just records it.
    w.err(reader, "bid_package.approve_registration", args(json!({"number": 1})));
    w.err(pm, "bid_package.approve_registration", args(json!({"number": 1})));
    let out = w.ok(w.admin(), "bid_package.approve_registration", args(json!({"number": 1})));
    assert_eq!(out["invitation"], 1);
    let pkg = w.engine.state().bid_packages.iter().next().unwrap().clone();
    assert_eq!((pkg.invitations[0].email.as_str(), pkg.invited.len()), ("bids@granite.example", 1));
    assert!(w.engine.state().vendors.iter().any(|v| v.name == "Granite Works"));
    w.err(w.admin(), "bid_package.approve_registration", args(json!({"number": 1})));
    // A company already in the directory is chosen, not duplicated.
    let slab = w.ok(w.admin(), "vendor.create", json!({"name": "Slab Brothers", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    w.err(w.admin(), "bid_package.approve_registration", args(json!({"number": 2})));
    w.ok(pm, "bid_package.approve_registration", args(json!({"number": 2, "vendor": slab})));
    w.ok(pm, "bid_package.decline_registration", args(json!({"number": 3})));
    w.err(pm, "bid_package.decline_registration", args(json!({"number": 3})));
    assert_eq!(w.engine.state().bid_packages.iter().next().unwrap().invitations.len(), 2);

    // Replacing the link kills the old one; switching it off ends registration.
    let rotated = w.ok(pm, "bid_package.set_public", args(json!({"rotate": true})));
    assert_eq!(rotated["generation"], 1);
    w.err(Actor::System, "bid_package.portal_register", reg(0, "Late Co", "late@late.example"));
    w.ok(Actor::System, "bid_package.portal_register", reg(1, "Late Co", "late@late.example"));
    w.ok(pm, "bid_package.set_public", args(json!({"enabled": false})));
    w.err(Actor::System, "bid_package.portal_register", reg(1, "Later Co", "later@late.example"));
    w.assert_replay();
}
