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

#[test]
fn signed_contract_workflow() {
    use buildkiosk_model::finance::SignatureStatus;
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"commitments": "standard"}}));
    let concrete = code(&w, "03-300");
    let sig = |w: &World, c: u64| w.engine.state().commitments.get(buildkiosk_core::Id::new(c)).unwrap().signature.clone();
    let c = w.ok(eng, "commitment.create", json!({"project": p, "title": "Footings", "lines": [{"cost_code": concrete, "amount": "30000"}]}))["id"]
        .as_u64()
        .unwrap();
    let empty = w.ok(eng, "commitment.create", json!({"project": p, "title": "Empty"}))["id"].as_u64().unwrap();

    // Without the rule, approval doesn't look at the signature.
    let loose = w.ok(eng, "commitment.create", json!({"project": p, "title": "Loose", "lines": [{"cost_code": concrete, "amount": "1"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "commitment.approve", json!({"project": p, "id": loose}));

    // Turning the rule on is an admin's choice, and then an unsigned contract can't be approved.
    w.err(eng, "commitment.require_signed", json!({"project": p, "required": true}));
    w.err(pm, "commitment.require_signed", json!({"project": p}));
    w.ok(pm, "commitment.require_signed", json!({"project": p, "required": true}));
    w.err(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(eng, "commitment.send_for_signature", json!({"project": p, "id": empty}));

    // Send it out, take it back, send again, and it comes back declined.
    w.ok(eng, "commitment.send_for_signature", json!({"project": p, "id": c, "to": "sam@example.com", "on": "2026-10-12"}));
    let s = sig(&w, c);
    assert_eq!((s.status, s.sent_to.as_str(), s.sent_on.unwrap().to_string()), (SignatureStatus::Sent, "sam@example.com", "2026-10-12".to_string()));
    w.err(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.err(eng, "commitment.send_for_signature", json!({"project": p, "id": c, "on": "tomorrow"}));
    w.err(eng, "commitment.decline_signature", json!({"project": p, "id": empty}));
    w.ok(eng, "commitment.decline_signature", json!({"project": p, "id": c, "note": "Wants a longer schedule"}));
    assert_eq!((sig(&w, c).status, sig(&w, c).note), (SignatureStatus::Declined, "Wants a longer schedule".to_string()));
    w.ok(eng, "commitment.reset_signature", json!({"project": p, "id": c}));
    assert_eq!(sig(&w, c).status, SignatureStatus::NotSent);
    w.err(eng, "commitment.reset_signature", json!({"project": p, "id": c}));

    // Recording the signature: a signer is needed, and it can't pre-date sending.
    w.ok(eng, "commitment.send_for_signature", json!({"project": p, "id": c, "on": "2026-10-12"}));
    w.err(eng, "commitment.record_signature", json!({"project": p, "id": c}));
    w.err(eng, "commitment.record_signature", json!({"project": p, "id": c, "signer": "Sam Sub", "on": "2026-10-01"}));
    w.ok(eng, "commitment.record_signature", json!({"project": p, "id": c, "signer": "Sam Sub", "on": "2026-10-14", "note": "Wet ink, scanned"}));
    let s = sig(&w, c);
    assert_eq!(
        (s.status, s.signer.as_str(), s.signed_on.unwrap().to_string(), s.sent_on.unwrap().to_string()),
        (SignatureStatus::Signed, "Sam Sub", "2026-10-14".to_string(), "2026-10-12".to_string())
    );
    w.err(eng, "commitment.record_signature", json!({"project": p, "id": c, "signer": "Sam Sub"}));
    w.err(eng, "commitment.send_for_signature", json!({"project": p, "id": c}));
    w.err(eng, "commitment.reset_signature", json!({"project": p, "id": c}));

    // Changing the amounts makes the signature void; the contract has to be signed again.
    w.ok(pm, "commitment.update", json!({"project": p, "id": c, "title": "Footings and piers"}));
    assert_eq!(sig(&w, c).status, SignatureStatus::Signed, "a new title leaves the signature alone");
    w.ok(pm, "commitment.update", json!({"project": p, "id": c, "lines": [{"cost_code": concrete, "amount": "32000"}]}));
    assert_eq!(sig(&w, c).status, SignatureStatus::NotSent);
    w.err(pm, "commitment.approve", json!({"project": p, "id": c}));
    w.ok(pm, "commitment.record_signature", json!({"project": p, "id": c, "signer": "Sam Sub"}));
    w.ok(pm, "commitment.approve", json!({"project": p, "id": c}));
    assert_eq!(w.engine.state().commitments.get(buildkiosk_core::Id::new(c)).unwrap().status, CommitmentStatus::Approved);
    // An approved commitment on a project that requires signatures keeps its signature; complete ones are closed.
    w.err(pm, "commitment.reset_signature", json!({"project": p, "id": c}));
    w.ok(pm, "commitment.complete", json!({"project": p, "id": c}));
    w.err(pm, "commitment.record_signature", json!({"project": p, "id": c, "signer": "x"}));
    // Switching the rule off lets an unsigned one through again.
    w.ok(pm, "commitment.require_signed", json!({"project": p, "required": false}));
    let again = w.ok(eng, "commitment.create", json!({"project": p, "title": "Rebar", "lines": [{"cost_code": concrete, "amount": "5000"}]}))["id"]
        .as_u64()
        .unwrap();
    w.ok(pm, "commitment.approve", json!({"project": p, "id": again}));
    w.assert_replay();
}

#[test]
fn budget_import_from_a_spreadsheet_file() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let admin = w.admin();
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    w.ok(pm, "member.update", json!({"project": p, "person": eng, "overrides": {"budget": "standard"}}));
    let count = |w: &World| w.engine.state().cost_codes.iter().filter(|c| c.project == Some(p) && c.deleted.is_none()).count();

    // A header names the columns, in any order, with quoted cells that hold commas.
    let text = "Notes,Cost code,Original budget\n\"Concrete, slab and footings\",03-300,\"120,000.00\"\n,05-100,80000\n";
    let r = w.ok(eng, "budget.import", json!({"project": p, "text": text}));
    assert_eq!((r["lines"].as_u64(), r["codes_added"].as_u64()), (Some(2), Some(0)));
    let line = |w: &World, c: &str| {
        let id = w.engine.state().cost_codes.iter().find(|x| x.code == c).unwrap().id;
        w.engine.state().budget_lines.iter().find(|b| b.cost_code == id && b.deleted.is_none()).map(|b| (b.original, b.notes.clone()))
    };
    assert_eq!(line(&w, "03-300"), Some((money(120_000), "Concrete, slab and footings".to_string())));
    assert_eq!(line(&w, "05-100").unwrap().0, money(80_000));

    // The old shape still works, tabs too; a header with no amount column is refused.
    w.ok(eng, "budget.import", json!({"project": p, "text": "03-300\t125000\tRevised"}));
    assert_eq!(line(&w, "03-300"), Some((money(125_000), "Revised".to_string())));
    w.err(eng, "budget.import", json!({"project": p, "text": "Cost code,Notes\n03-300,x"}));

    // Codes the project doesn't have: refused unless asked for, and then only by a project admin.
    let new = "Cost code,Description,Amount\n99-100,Owner allowances,25000\n99-200,,1\n";
    w.err(eng, "budget.import", json!({"project": p, "text": new}));
    w.err(eng, "budget.import", json!({"project": p, "text": new, "add_codes": true}));
    // The second new code has no description, so nothing at all is changed.
    w.err(admin, "budget.import", json!({"project": p, "text": new, "add_codes": true}));
    assert_eq!(count(&w), 25);
    let ok = "Cost code,Description,Amount\n99-100,Owner allowances,25000\n99-100,,5000\n03-300,Ignored for known codes,126000\n";
    let r = w.ok(admin, "budget.import", json!({"project": p, "text": ok, "add_codes": true}));
    assert_eq!((r["lines"].as_u64(), r["codes_added"].as_u64()), (Some(3), Some(1)));
    assert_eq!(count(&w), 26);
    let added = w.engine.state().cost_codes.iter().find(|c| c.code == "99-100").unwrap().clone();
    assert_eq!((added.description.as_str(), added.project), ("Owner allowances", Some(p)));
    assert_eq!(line(&w, "99-100").unwrap().0, money(5_000), "a later line for the same code wins");
    assert_eq!(line(&w, "03-300").unwrap().0, money(126_000));
    // The new code is usable like any other, and only on this project.
    let other = w.project("Harbor Office");
    w.err(admin, "budget.set_line", json!({"project": other, "cost_code": added.id.raw(), "original": "1"}));
    // A locked budget refuses the import.
    w.ok(admin, "budget.lock", json!({"project": p}));
    w.err(admin, "budget.import", json!({"project": p, "text": ok, "add_codes": true}));
    w.assert_replay();
}
