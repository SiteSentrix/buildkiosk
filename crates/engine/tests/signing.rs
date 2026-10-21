//! Signing a contract through a link: the other party reads the contract and signs it with no account.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Id;
use buildkiosk_engine::Actor;
use buildkiosk_model::finance::SignatureStatus;
use buildkiosk_testkit::World;
use serde_json::{Value, json};

fn code(w: &World, code: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|c| c.code == code).unwrap().id.raw()
}

fn link(kind: &str, id: u64, generation: u64, extra: Value) -> Value {
    let mut m = extra.as_object().cloned().unwrap_or_default();
    m.insert("kind".into(), json!(kind));
    m.insert("id".into(), json!(id));
    m.insert("generation".into(), json!(generation));
    Value::Object(m)
}

/// The generation of the commitment's current link.
fn generation(w: &World, c: u64) -> u64 {
    u64::from(w.engine.state().commitments.get(Id::new(c)).unwrap().signature.link.as_ref().unwrap().generation)
}

#[test]
fn the_other_party_signs_a_commitment_through_its_link() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let reader = w.member(p, "Rae", "Reader", "Read only");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"commitments": "standard"}}));
    let admin = w.admin();
    let vendor = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let concrete = code(&w, "03-300");
    let c = w
        .ok(eng, "commitment.create", json!({"project": p, "title": "Footings", "vendor": vendor, "retainage": "10", "lines": [{"cost_code": concrete, "description": "Footings and piers", "amount": "30000"}]}))
        ["id"]
        .as_u64()
        .unwrap();

    // A link comes with an address; a bad address or a past day is refused; nothing is signable before it is sent.
    w.err(eng, "commitment.send_for_signature", json!({"project": p, "id": c, "email": "not an address"}));
    w.err(eng, "commitment.send_for_signature", json!({"project": p, "id": c, "email": "sam@keystone.example", "expires": "2020-01-01"}));
    w.err(Actor::System, "contract.portal_view", link("commitment", c, 0, json!({})));
    w.ok(
        eng,
        "commitment.send_for_signature",
        json!({"project": p, "id": c, "to": "Sam Sub", "email": "sam@keystone.example", "expires": "2030-01-01"}),
    );
    let g = generation(&w, c);
    assert!(w.engine.state().commitments.get(Id::new(c)).unwrap().signature.link.as_ref().unwrap().sent.is_none());

    // The server records the email as sent (system only), and a stale generation does nothing.
    w.err(eng, "contract.link_sent", link("commitment", c, g, json!({})));
    w.ok(Actor::System, "contract.link_sent", link("commitment", c, g + 1, json!({})));
    assert!(w.engine.state().commitments.get(Id::new(c)).unwrap().signature.link.as_ref().unwrap().sent.is_none());
    w.ok(Actor::System, "contract.link_sent", link("commitment", c, g, json!({})));
    assert!(w.engine.state().commitments.get(Id::new(c)).unwrap().signature.link.as_ref().unwrap().sent.is_some());

    // What the signer reads: the terms and the company, not other contracts; a person can't use the portal commands.
    w.err(pm, "contract.portal_view", link("commitment", c, g, json!({})));
    let view = w.ok(Actor::System, "contract.portal_view", link("commitment", c, g, json!({})));
    assert_eq!(
        (view["party"].clone(), view["title"].clone(), view["status"].clone()),
        (json!("Keystone Concrete"), json!("Footings"), json!("sent"))
    );
    assert_eq!((view["total"].clone(), view["retainage"].clone()), (json!("30000.00"), json!("10")));
    assert_eq!(view["lines"][0]["description"], "Footings and piers");
    assert!(view["terms"].as_i64().unwrap() > 0);
    // Anything else is the same answer: a wrong generation, another kind, another contract.
    for bad in [
        link("commitment", c, g + 1, json!({})),
        link("prime_contract", c, g, json!({})),
        link("commitment", c + 99, g, json!({})),
        link("nonsense", c, g, json!({})),
    ] {
        assert_eq!(w.err(Actor::System, "contract.portal_view", bad).to_string(), "this link is not valid");
    }

    // Signing: the box must be ticked, a full name typed, and the terms must be the ones the page showed.
    let terms = view["terms"].as_i64().unwrap();
    let sign = |name: &str, agree: bool, terms: i64| link("commitment", c, g, json!({"name": name, "agree": agree, "terms": terms}));
    w.err(Actor::System, "contract.portal_sign", sign("Sam Sub", false, terms));
    w.err(Actor::System, "contract.portal_sign", sign("S", true, terms));
    w.err(Actor::System, "contract.portal_sign", sign("Sam Sub", true, terms - 1));
    // Changing the contract after the page opened refuses the signature and clears the link along with the signature.
    let before = w.engine.state().notifications.iter().count();
    w.ok(eng, "commitment.update", json!({"project": p, "id": c, "description": "Includes rebar"}));
    w.err(Actor::System, "contract.portal_sign", sign("Sam Sub", true, terms));
    let view = w.ok(Actor::System, "contract.portal_view", link("commitment", c, g, json!({})));
    let terms = view["terms"].as_i64().unwrap();
    w.ok(Actor::System, "contract.portal_sign", sign("Sam Sub", true, terms));
    let s = w.engine.state().commitments.get(Id::new(c)).unwrap().signature.clone();
    assert_eq!((s.status, s.signer.as_str(), s.signed_at.is_some()), (SignatureStatus::Signed, "Sam Sub", true));
    assert_eq!(s.signed_terms.unwrap().at.millis(), terms, "what was on the page is recorded");
    assert!(w.engine.state().notifications.iter().count() > before, "the team is told");
    // Signed is final through the link; the page now shows who signed.
    w.err(Actor::System, "contract.portal_sign", sign("Sam Sub", true, terms));
    w.err(Actor::System, "contract.portal_decline", link("commitment", c, g, json!({})));
    let view = w.ok(Actor::System, "contract.portal_view", link("commitment", c, g, json!({})));
    assert_eq!((view["status"].clone(), view["signer"].clone()), (json!("signed"), json!("Sam Sub")));
    assert_eq!(w.engine.state().commitments.get(Id::new(c)).unwrap().updated.at.millis(), terms, "signing did not change the terms");
    // Approval accepts it under the signed-contract rule.
    w.ok(pm, "commitment.require_signed", json!({"project": p, "required": true}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(reader, "commitment.reset_signature", json!({"project": p, "id": c}));

    // Changing the lines clears the signature, and the link with it.
    let other = w.ok(eng, "commitment.create", json!({"project": p, "title": "Slab", "lines": [{"cost_code": concrete, "amount": "1000"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(eng, "commitment.send_for_signature", json!({"project": p, "id": other, "email": "sam@keystone.example"}));
    let g2 = generation(&w, other);
    w.ok(eng, "commitment.update", json!({"project": p, "id": other, "lines": [{"cost_code": concrete, "amount": "2000"}]}));
    assert_eq!(w.err(Actor::System, "contract.portal_view", link("commitment", other, g2, json!({}))).to_string(), "this link is not valid");

    // Declining records why; sending again makes a new link and the old one stops.
    w.ok(eng, "commitment.send_for_signature", json!({"project": p, "id": other, "email": "sam@keystone.example"}));
    let g3 = generation(&w, other);
    let seen = w.ok(Actor::System, "contract.portal_view", link("commitment", other, g3, json!({})))["terms"].as_i64().unwrap();
    assert!(seen > 0);
    w.ok(Actor::System, "contract.portal_decline", link("commitment", other, g3, json!({"note": "Wants a longer schedule"})));
    let s = w.engine.state().commitments.get(Id::new(other)).unwrap().signature.clone();
    assert_eq!((s.status, s.note.as_str()), (SignatureStatus::Declined, "Wants a longer schedule"));
    w.err(Actor::System, "contract.portal_sign", link("commitment", other, g3, json!({"name": "Sam Sub", "agree": true, "terms": seen})));
    w.assert_replay();
}

#[test]
fn the_owner_signs_a_prime_contract_and_a_change_order() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let owner = w.ok(admin, "vendor.create", json!({"name": "Riverside Health", "kind": "owner"}))["id"].as_u64().unwrap();
    let concrete = code(&w, "03-300");
    let k = w
        .ok(pm, "prime_contract.create", json!({"project": p, "title": "Clinic", "owner": owner, "retainage": "5", "lines": [{"cost_code": concrete, "description": "Concrete", "amount": "100000"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "prime_contract.approve", json!({"project": p, "id": k}));
    let co = w
        .ok(pm, "prime_change.create", json!({"project": p, "contract": k, "title": "Thicker slab", "lines": [{"cost_code": concrete, "description": "Slab thickening", "amount": "10000"}]}))["id"]
        .as_u64()
        .unwrap();
    for (kind, send, id) in [("prime_contract", "prime_contract.send_for_signature", k), ("prime_change", "prime_change.send_for_signature", co)] {
        w.ok(pm, send, json!({"project": p, "id": id, "email": "owner@riverside.example"}));
        let g = match kind {
            "prime_contract" => w.engine.state().prime_contracts.get(Id::new(id)).unwrap().signature.link.as_ref().unwrap().generation,
            _ => w.engine.state().prime_changes.get(Id::new(id)).unwrap().signature.link.as_ref().unwrap().generation,
        };
        let g = u64::from(g);
        let view = w.ok(Actor::System, "contract.portal_view", link(kind, id, g, json!({})));
        assert_eq!(view["party"], "Riverside Health", "{kind}");
        let terms = view["terms"].as_i64().unwrap();
        w.ok(Actor::System, "contract.portal_sign", link(kind, id, g, json!({"name": "Olivia Owner", "agree": true, "terms": terms})));
        let view = w.ok(Actor::System, "contract.portal_view", link(kind, id, g, json!({})));
        assert_eq!((view["status"].clone(), view["signer"].clone()), (json!("signed"), json!("Olivia Owner")), "{kind}");
    }
    // A link for one kind does not open another's record even with the same number.
    assert!(w.err(Actor::System, "contract.portal_view", link("prime_change", k, 1, json!({}))).to_string().contains("not valid"));
    w.assert_replay();
}

#[test]
fn reminders_are_counted_and_stop_at_three() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let vendor = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let concrete = code(&w, "03-300");
    let c = w
        .ok(pm, "commitment.create", json!({"project": p, "title": "Footings", "vendor": vendor, "lines": [{"cost_code": concrete, "description": "Footings", "amount": "30000"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "commitment.send_for_signature", json!({"project": p, "id": c, "email": "sam@keystone.example"}));
    let g = generation(&w, c);
    let current = |w: &World| w.engine.state().commitments.get(Id::new(c)).unwrap().signature.link.clone().unwrap();
    w.ok(Actor::System, "contract.link_sent", link_params(c, g));
    // A person can't record one; a stale generation does nothing; each real one counts, up to three.
    w.err(pm, "contract.reminder_sent", link_params(c, g));
    w.ok(Actor::System, "contract.reminder_sent", link_params(c, g + 1));
    assert_eq!(current(&w).reminders, 0);
    for n in 1..=5u8 {
        w.ok(Actor::System, "contract.reminder_sent", link_params(c, g));
        assert_eq!(current(&w).reminders, n.min(3));
    }
    assert!(current(&w).reminded.is_some());
    // Once signed, reminders stop being recorded.
    let terms = w.ok(Actor::System, "contract.portal_view", link_params(c, g))["terms"].as_i64().unwrap();
    w.ok(Actor::System, "contract.portal_sign", link("commitment", c, g, json!({"name": "Sam Sub", "agree": true, "terms": terms})));
    assert_eq!(w.engine.state().commitments.get(Id::new(c)).unwrap().signature.status, SignatureStatus::Signed);
    w.assert_replay();
}

fn link_params(c: u64, g: u64) -> Value {
    link("commitment", c, g, json!({}))
}

#[test]
fn several_people_sign_one_contract_in_turn() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let admin = w.admin();
    let vendor = w.ok(admin, "vendor.create", json!({"name": "Keystone Concrete", "kind": "subcontractor"}))["id"].as_u64().unwrap();
    let concrete = code(&w, "03-300");
    let c = w
        .ok(pm, "commitment.create", json!({"project": p, "title": "Footings", "vendor": vendor, "lines": [{"cost_code": concrete, "description": "Footings", "amount": "30000"}]}))["id"]
        .as_u64()
        .unwrap();
    // More signers need a first one; a bad address, too many and a lone "ordered" are refused.
    w.err(pm, "commitment.send_for_signature", json!({"project": p, "id": c, "others": "b@keystone.example"}));
    w.err(pm, "commitment.send_for_signature", json!({"project": p, "id": c, "ordered": true}));
    w.err(pm, "commitment.send_for_signature", json!({"project": p, "id": c, "email": "a@keystone.example", "others": "nope"}));
    w.err(pm, "commitment.send_for_signature", json!({"project": p, "id": c, "email": "a@keystone.example", "others": "1@x.example 2@x.example 3@x.example 4@x.example 5@x.example 6@x.example"}));
    w.ok(
        pm,
        "commitment.send_for_signature",
        json!({"project": p, "id": c, "email": "a@keystone.example", "others": "b@keystone.example\nc@keystone.example", "ordered": true}),
    );
    let sig = |w: &World| w.engine.state().commitments.get(Id::new(c)).unwrap().signature.clone();
    let slots: Vec<(usize, u32, String)> = sig(&w).slots().iter().map(|s| (s.index, s.link.generation, s.link.email.clone())).collect();
    assert_eq!(slots.len(), 3);
    assert!(slots.windows(2).all(|x| x[0].1 != x[1].1), "every link has its own generation");
    let [(_, ga, _), (_, gb, _), (_, gc, _)] = [slots[0].clone(), slots[1].clone(), slots[2].clone()];
    let (ga, gb, gc) = (u64::from(ga), u64::from(gb), u64::from(gc));
    // Each link shows its own page; everyone sees who has signed.
    let view_b = w.ok(Actor::System, "contract.portal_view", link_params(c, gb));
    assert_eq!(
        (view_b["email"].clone(), view_b["ordered"].clone(), view_b["your_turn"].clone()),
        (json!("b@keystone.example"), json!(true), json!(false))
    );
    assert_eq!(view_b["signers"].as_array().unwrap().len(), 3);
    let terms = view_b["terms"].as_i64().unwrap();
    let sign = |g: u64, name: &str| link("commitment", c, g, json!({"name": name, "agree": true, "terms": terms}));
    // Out of turn is refused; the first signs, the contract stays out; the second signs; the third completes it.
    w.err(Actor::System, "contract.portal_sign", sign(gb, "Bea Second"));
    w.err(Actor::System, "contract.portal_sign", sign(gc, "Cal Third"));
    let out = w.ok(Actor::System, "contract.portal_sign", sign(ga, "Amy First"));
    assert_eq!((out["complete"].clone(), out["signed"].clone(), out["of"].clone()), (json!(false), json!(1), json!(3)));
    assert_eq!(sig(&w).status, SignatureStatus::Sent);
    w.err(Actor::System, "contract.portal_sign", sign(ga, "Amy Again"));
    assert_eq!(w.ok(Actor::System, "contract.portal_view", link_params(c, gb))["your_turn"], json!(true));
    w.ok(Actor::System, "contract.portal_sign", sign(gb, "Bea Second"));
    w.err(Actor::System, "contract.portal_sign", sign(gb, "Bea Again"));
    assert_eq!(sig(&w).status, SignatureStatus::Sent);
    let out = w.ok(Actor::System, "contract.portal_sign", sign(gc, "Cal Third"));
    assert_eq!(out["complete"], json!(true));
    let done = sig(&w);
    assert_eq!(done.status, SignatureStatus::Signed);
    assert_eq!((done.signer.as_str(), done.others[0].name.as_str(), done.others[1].name.as_str()), ("Amy First", "Bea Second", "Cal Third"));
    assert!(done.all_signed());
    // The team was told at each step.
    let told: Vec<String> = w.engine.state().notifications.iter().map(|n| n.text.clone()).collect();
    assert!(told.iter().any(|t| t.contains("Amy First signed (1 of 3)")) && told.iter().any(|t| t.contains("all 3 people have signed")), "{told:?}");
    w.assert_replay();

    // In any order: the third can sign first; one declining stops it for everyone.
    let d = w.ok(
        pm,
        "commitment.create",
        json!({"project": p, "title": "Slab", "vendor": vendor, "lines": [{"cost_code": concrete, "description": "Slab", "amount": "9000"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "commitment.send_for_signature", json!({"project": p, "id": d, "email": "a@keystone.example", "others": "b@keystone.example"}));
    let gens: Vec<u64> =
        w.engine.state().commitments.get(Id::new(d)).unwrap().signature.slots().iter().map(|s| u64::from(s.link.generation)).collect();
    let terms_d = w.ok(Actor::System, "contract.portal_view", link("commitment", d, gens[1], json!({})))["terms"].as_i64().unwrap();
    w.ok(Actor::System, "contract.portal_sign", link("commitment", d, gens[1], json!({"name": "Bea Second", "agree": true, "terms": terms_d})));
    assert_eq!(w.engine.state().commitments.get(Id::new(d)).unwrap().signature.status, SignatureStatus::Sent);
    w.ok(Actor::System, "contract.portal_decline", link("commitment", d, gens[0], json!({"note": "Wrong amount"})));
    assert_eq!(w.engine.state().commitments.get(Id::new(d)).unwrap().signature.status, SignatureStatus::Declined);
    w.assert_replay();
}
