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
