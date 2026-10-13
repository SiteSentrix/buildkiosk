//! The prime contract, its change orders and owner invoices.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Money;
use buildkiosk_engine::view::{contract_summary, invoice_summary};
use buildkiosk_model::finance::InvoiceStatus;
use buildkiosk_testkit::World;
use serde_json::json;

fn code(w: &World, code: &str) -> u64 {
    w.engine.state().cost_codes.iter().find(|c| c.code == code).unwrap().id.raw()
}

fn m(s: &str) -> Money {
    Money::parse(s).unwrap()
}

#[test]
fn billing_with_retainage_change_orders_and_payment() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    w.ok(
        pm,
        "member.update",
        json!({"project": p, "person": eng, "overrides": {"prime_contracts": "standard", "invoicing": "standard", "change_orders": "standard"}}),
    );
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));

    // The contract is a draft until an admin approves it; it can't be invoiced before.
    let k = w.ok(
        eng,
        "prime_contract.create",
        json!({"project": p, "title": "Clinic contract", "retainage": "10", "lines": [
            {"cost_code": concrete, "description": "Concrete", "amount": "100000"},
            {"cost_code": steel, "description": "Steel", "amount": "50000"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(eng, "owner_invoice.create", json!({"project": p, "contract": k}));
    w.err(eng, "prime_contract.approve", json!({"project": p, "id": k}));
    w.err(eng, "prime_contract.create", json!({"project": p, "title": "Bad", "retainage": "150"}));
    w.err(sub, "prime_contract.get", json!({"project": p, "id": k}));
    w.ok(pm, "prime_contract.approve", json!({"project": p, "id": k}));

    // Invoice 1: 40,000 of concrete; 10% retainage withheld.
    let i1 = w.ok(
        eng,
        "owner_invoice.create",
        json!({"project": p, "contract": k, "period_end": "2026-10-31", "lines": [{"row": "c.0", "amount": "40,000"}, {"row": "c.1", "amount": ""}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let summary = |w: &World, id: u64| {
        let inv = w.engine.state().owner_invoices.iter().find(|i| i.id.raw() == id).unwrap().clone();
        invoice_summary(w.engine.state(), &inv).unwrap()
    };
    let s = summary(&w, i1);
    assert_eq!((s.this_period, s.retainage_this, s.net_due, s.scheduled), (m("40000"), m("4000"), m("36000"), m("150000")));
    w.err(eng, "owner_invoice.update", json!({"project": p, "id": i1, "lines": [{"row": "c.0", "amount": "100,000.01"}]}));
    w.err(eng, "owner_invoice.update", json!({"project": p, "id": i1, "lines": [{"row": "c.9", "amount": "5"}]}));
    w.err(eng, "owner_invoice.update", json!({"project": p, "id": i1, "lines": [{"row": "c.0", "amount": "5"}, {"row": "c.0", "amount": "6"}]}));
    w.err(eng, "owner_invoice.approve", json!({"project": p, "id": i1}));
    w.ok(eng, "owner_invoice.submit", json!({"project": p, "id": i1}));
    w.err(eng, "owner_invoice.update", json!({"project": p, "id": i1, "notes": "x"}).clone());
    w.ok(pm, "owner_invoice.approve", json!({"project": p, "id": i1}));
    w.err(eng, "owner_invoice.pay", json!({"project": p, "id": i1}));
    w.ok(pm, "owner_invoice.pay", json!({"project": p, "id": i1, "paid_on": "2026-11-10"}));
    assert_eq!(w.engine.state().owner_invoices.iter().next().unwrap().status, InvoiceStatus::Paid);

    // A change order adds a billable row once approved; the contract's lines are now fixed.
    let co = w.ok(eng, "prime_change.create", json!({"project": p, "contract": k, "title": "Thicker slab", "lines": [{"cost_code": concrete, "description": "Slab thickening", "amount": "10000"}]}))["id"].as_u64().unwrap();
    w.ok(pm, "prime_change.approve", json!({"project": p, "id": co}));
    w.err(pm, "prime_contract.update", json!({"project": p, "id": k, "lines": [{"cost_code": concrete, "amount": "1"}]}));
    w.err(pm, "prime_contract.void", json!({"project": p, "id": k}));

    // Invoice 2: more concrete, steel and the change order row; retainage on the new work only.
    let key = format!("p.{co}.0");
    let i2 = w.ok(
        eng,
        "owner_invoice.create",
        json!({"project": p, "contract": k, "lines": [
            {"row": "c.0", "amount": "30000"}, {"row": "c.1", "amount": "20000"}, {"row": key, "amount": "5000"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    let s = summary(&w, i2);
    assert_eq!(s.previous, m("40000"), "invoice 1 was billed");
    assert_eq!(
        (s.this_period, s.to_date, s.retainage_to_date, s.retainage_this, s.net_due),
        (m("55000"), m("95000"), m("9500"), m("5500"), m("49500"))
    );
    assert_eq!(s.rows[0].previous, m("40000"));
    assert_eq!(s.rows[0].balance(), m("30000"));
    assert_eq!(s.rows.len(), 3);
    w.err(eng, "owner_invoice.update", json!({"project": p, "id": i2, "lines": [{"row": key, "amount": "10,000.01"}]}));
    w.ok(eng, "owner_invoice.submit", json!({"project": p, "id": i2}));
    w.err(pm, "prime_change.void", json!({"project": p, "id": co}));
    w.err(pm, "owner_invoice.void", json!({"project": p, "id": i1}));

    // The contract roll-up: billed counts submitted work; paid is the net of paid invoices.
    let contract = w.engine.state().prime_contracts.iter().next().unwrap().clone();
    let c = contract_summary(w.engine.state(), &contract).unwrap();
    assert_eq!((c.original, c.changes, c.revised), (m("150000"), m("10000"), m("160000")));
    assert_eq!((c.billed, c.retainage_held, c.paid, c.balance_to_finish), (m("95000"), m("9500"), m("36000"), m("65000")));
    let got = w.ok(pm, "owner_invoice.get", json!({"project": p, "id": i2}));
    assert_eq!(got["totals"]["net_due"], json!(m("49500")));

    // A draft doesn't count; voiding a submitted one removes it from billing.
    w.ok(pm, "owner_invoice.void", json!({"project": p, "id": i2}));
    let c = contract_summary(w.engine.state(), &contract).unwrap();
    assert_eq!(c.billed, m("40000"));
    w.ok(pm, "owner_invoice.delete", json!({"project": p, "id": i2}));
    w.ok(w.admin(), "owner_invoice.restore", json!({"project": p, "id": i2}));
    w.assert_replay();
}

#[test]
fn drafts_are_private_and_projects_are_isolated() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"invoicing": "standard"}}));
    let steel = code(&w, "05-100");
    let k = w.ok(pm, "prime_contract.create", json!({"project": p, "title": "C", "lines": [{"cost_code": steel, "amount": "1000"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "prime_contract.approve", json!({"project": p, "id": k}));
    let draft =
        w.ok(pm, "owner_invoice.create", json!({"project": p, "contract": k, "lines": [{"row": "c.0", "amount": "100"}]}))["id"].as_u64().unwrap();
    w.err(eng, "owner_invoice.get", json!({"project": p, "id": draft}));
    assert!(w.ok(eng, "owner_invoice.list", json!({"project": p})).as_array().unwrap().is_empty());
    w.err(eng, "owner_invoice.submit", json!({"project": p, "id": draft}));
    let other = w.project("Other");
    w.err(pm, "owner_invoice.get", json!({"project": other, "id": draft}));
    w.err(pm, "owner_invoice.create", json!({"project": other, "contract": k}));
    let empty = w.ok(pm, "owner_invoice.create", json!({"project": p, "contract": k}))["id"].as_u64().unwrap();
    w.err(pm, "owner_invoice.submit", json!({"project": p, "id": empty}));
    w.assert_replay();
}

#[test]
fn commitment_invoices_bill_the_schedule_and_its_change_orders() {
    use buildkiosk_engine::view::{commitment_billing, commitment_invoice_summary};
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let sub = w.member(p, "Sam", "Sub", "Subcontractor");
    w.ok(
        pm,
        "member.update",
        json!({"project": p, "person": eng, "overrides": {"invoicing": "standard", "commitments": "standard", "change_orders": "standard"}}),
    );
    let (concrete, steel) = (code(&w, "03-300"), code(&w, "05-100"));
    let c = w.ok(
        eng,
        "commitment.create",
        json!({"project": p, "title": "Slab", "retainage": "5", "lines": [
            {"cost_code": concrete, "description": "Footings", "amount": "40000"}, {"cost_code": steel, "description": "Rebar", "amount": "20000"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(eng, "commitment.create", json!({"project": p, "title": "Bad", "retainage": "101"}));
    w.err(eng, "commitment_invoice.create", json!({"project": p, "commitment": c}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(sub, "commitment_invoice.create", json!({"project": p, "commitment": c}));

    let i1 = w.ok(
        eng,
        "commitment_invoice.create",
        json!({"project": p, "commitment": c, "period_end": "2026-10-31", "lines": [{"row": "c.0", "amount": "20000"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    let summary = |w: &World, id: u64| {
        let inv = w.engine.state().commitment_invoices.iter().find(|i| i.id.raw() == id).unwrap().clone();
        commitment_invoice_summary(w.engine.state(), &inv).unwrap()
    };
    let s = summary(&w, i1);
    assert_eq!((s.this_period, s.retainage_this, s.net_due, s.scheduled), (m("20000"), m("1000"), m("19000"), m("60000")));
    w.err(eng, "commitment_invoice.update", json!({"project": p, "id": i1, "lines": [{"row": "c.0", "amount": "40000.01"}]}));
    w.ok(eng, "commitment_invoice.submit", json!({"project": p, "id": i1}));
    w.ok(pm, "commitment_invoice.approve", json!({"project": p, "id": i1}));
    w.ok(pm, "commitment_invoice.pay", json!({"project": p, "id": i1, "paid_on": "2026-11-12"}));

    // Once invoiced, the lines are fixed and the commitment can't be voided; a change order adds a row.
    w.err(pm, "commitment.update", json!({"project": p, "id": c, "lines": [{"cost_code": concrete, "amount": "1"}]}));
    w.err(pm, "commitment.void", json!({"project": p, "id": c}));
    let cco = w.ok(eng, "commitment_change.create", json!({"project": p, "commitment": c, "title": "Extra rebar", "lines": [{"cost_code": steel, "description": "Added rebar", "amount": "5000"}]}))["id"].as_u64().unwrap();
    w.ok(pm, "commitment_change.approve", json!({"project": p, "id": cco}));
    let i2 = w.ok(
        eng,
        "commitment_invoice.create",
        json!({"project": p, "commitment": c, "lines": [
            {"row": "c.0", "amount": "10000"}, {"row": "c.1", "amount": "20000"}, {"row": format!("p.{cco}.0"), "amount": "5000"}
        ]}),
    )["id"]
        .as_u64()
        .unwrap();
    let s = summary(&w, i2);
    assert_eq!((s.previous, s.this_period, s.scheduled), (m("20000"), m("35000"), m("65000")));
    assert_eq!((s.retainage_this, s.net_due), (m("1750"), m("33250")));
    w.err(eng, "commitment_invoice.update", json!({"project": p, "id": i2, "lines": [{"row": "p.999.0", "amount": "1"}]}));
    w.ok(eng, "commitment_invoice.submit", json!({"project": p, "id": i2}));
    w.err(pm, "commitment_invoice.void", json!({"project": p, "id": i1}));
    w.err(pm, "commitment_change.void", json!({"project": p, "id": cco}));

    let commitment = w.engine.state().commitments.iter().next().unwrap().clone();
    let b = commitment_billing(w.engine.state(), &commitment).unwrap();
    assert_eq!((b.original, b.changes, b.revised), (m("60000"), m("5000"), m("65000")));
    assert_eq!((b.billed, b.retainage_held, b.paid, b.balance_to_finish), (m("55000"), m("2750"), m("19000"), m("10000")));
    // Owner invoices and commitment invoices are separate: the owner side is untouched.
    assert!(w.engine.state().owner_invoices.iter().next().is_none());
    // Drafts are private to their author and admins.
    let draft = w.ok(eng, "commitment_invoice.create", json!({"project": p, "commitment": c, "lines": []}))["id"].as_u64().unwrap();
    w.err(sub, "commitment_invoice.get", json!({"project": p, "id": draft}));
    w.ok(pm, "commitment_invoice.get", json!({"project": p, "id": draft}));
    w.assert_replay();
}

#[test]
fn prime_documents_are_signed_and_budget_views_have_defaults() {
    use buildkiosk_model::finance::SignatureStatus;
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let sup = w.member(p, "Sue", "Super", "Superintendent");
    let steel = code(&w, "05-100");
    let k = w.ok(pm, "prime_contract.create", json!({"project": p, "title": "Clinic contract", "lines": [{"cost_code": steel, "amount": "50000"}]}))
        ["id"]
        .as_u64()
        .unwrap();
    let status = |w: &World| w.engine.state().prime_contracts.iter().next().unwrap().signature.status;
    // Required signatures hold back approval, and a change to the lines voids a recorded signature.
    w.ok(pm, "commitment.require_signed", json!({"project": p, "required": true}));
    w.err(pm, "prime_contract.approve", json!({"project": p, "id": k}));
    w.err(pm, "prime_contract.record_signature", json!({"project": p, "id": k}));
    w.err(pm, "prime_contract.decline_signature", json!({"project": p, "id": k}));
    w.err(pm, "prime_contract.reset_signature", json!({"project": p, "id": k}));
    w.ok(pm, "prime_contract.send_for_signature", json!({"project": p, "id": k, "to": "Owner rep", "on": "2026-10-10"}));
    assert_eq!(status(&w), SignatureStatus::Sent);
    w.err(pm, "prime_contract.record_signature", json!({"project": p, "id": k, "signer": "Olga Owner", "on": "2026-10-01"}));
    w.ok(pm, "prime_contract.decline_signature", json!({"project": p, "id": k, "note": "wants a change"}));
    w.ok(pm, "prime_contract.send_for_signature", json!({"project": p, "id": k}));
    w.ok(pm, "prime_contract.record_signature", json!({"project": p, "id": k, "signer": "Olga Owner", "note": "wet signature on file"}));
    assert_eq!(status(&w), SignatureStatus::Signed);
    w.ok(pm, "prime_contract.update", json!({"project": p, "id": k, "lines": [{"cost_code": steel, "amount": "55000"}]}));
    assert_eq!(status(&w), SignatureStatus::NotSent, "what was signed is no longer what is written");
    w.ok(pm, "prime_contract.record_signature", json!({"project": p, "id": k, "signer": "Olga Owner"}));
    w.ok(pm, "prime_contract.approve", json!({"project": p, "id": k}));
    w.err(pm, "prime_contract.reset_signature", json!({"project": p, "id": k}));

    // A change order needs its own signature under the same rule.
    let c = w.ok(
        pm,
        "prime_change.create",
        json!({"project": p, "contract": k, "title": "Extra steel", "lines": [{"cost_code": steel, "amount": "2000"}]}),
    )["id"]
        .as_u64()
        .unwrap();
    w.err(pm, "prime_change.approve", json!({"project": p, "id": c}));
    w.ok(pm, "prime_change.record_signature", json!({"project": p, "id": c, "signer": "Olga Owner"}));
    w.err(sup, "prime_change.reset_signature", json!({"project": p, "id": c}));
    w.ok(pm, "prime_change.approve", json!({"project": p, "id": c}));
    w.ok(pm, "commitment.require_signed", json!({"project": p, "required": false}));

    // A default budget view is each person's own choice.
    let v = w.ok(pm, "budget.view_save", json!({"project": p, "name": "Cost only", "columns": ["revised", "cost"]}))["id"].as_u64().unwrap();
    w.ok(pm, "budget.view_default", json!({"project": p, "id": v}));
    let me = |w: &World, who| buildkiosk_engine::view::default_budget_view(w.engine.state(), who, p);
    assert_eq!((me(&w, pm).map(|x| x.raw()), me(&w, sup)), (Some(v), None));
    w.ok(sup, "budget.view_default", json!({"project": p, "id": v}));
    w.ok(pm, "budget.view_default", json!({"project": p}));
    assert_eq!((me(&w, pm), me(&w, sup).map(|x| x.raw())), (None, Some(v)));
    w.err(sup, "budget.view_default", json!({"project": p, "id": 9999}));
    w.assert_replay();
}
