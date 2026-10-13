//! Correspondence: draft, send, reply, close, visibility.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_testkit::World;
use serde_json::json;

#[test]
fn draft_send_reply_and_close() {
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");
    let reader = w.member(p, "Rae", "Reader", "Read only");

    let c = w.ok(eng, "correspondence.create", json!({"project": p, "subject": "Notice of delay", "kind": "Notice", "to": [arch.raw()], "cc": [pm.raw()], "reply_by": "2026-10-30", "body": "Steel is late.", "private": true}))["id"]
        .as_u64()
        .unwrap();
    w.err(eng, "correspondence.create", json!({"project": p}));
    w.err(arch, "correspondence.get", json!({"project": p, "id": c}));
    w.err(arch, "correspondence.reply", json!({"project": p, "id": c, "body": "x"}));
    w.ok(eng, "correspondence.update", json!({"project": p, "id": c, "to": []}));
    w.err(eng, "correspondence.send", json!({"project": p, "id": c}));
    w.ok(eng, "correspondence.update", json!({"project": p, "id": c, "to": [arch.raw()]}));
    w.ok(eng, "correspondence.attach", json!({"project": p, "id": c, "sha256": "a".repeat(64), "name": "notice.pdf", "size": 2}));
    w.ok(eng, "correspondence.send", json!({"project": p, "id": c}));
    w.err(eng, "correspondence.send", json!({"project": p, "id": c}));
    w.err(eng, "correspondence.update", json!({"project": p, "id": c, "subject": "x"}));
    assert!(w.engine.state().notifications.iter().any(|n| n.person == arch && n.text.contains("Correspondence #1")));

    // Private: recipients, the author and admins only.
    w.ok(arch, "correspondence.get", json!({"project": p, "id": c}));
    w.err(reader, "correspondence.get", json!({"project": p, "id": c}));
    w.err(reader, "correspondence.reply", json!({"project": p, "id": c, "body": "hi"}));
    w.ok(arch, "correspondence.reply", json!({"project": p, "id": c, "body": "Understood; please send a revised schedule."}));
    assert!(w.engine.state().notifications.iter().any(|n| n.person == eng && n.text.contains("New reply")));
    w.err(arch, "correspondence.close", json!({"project": p, "id": c}));
    w.ok(eng, "correspondence.close", json!({"project": p, "id": c}));
    w.err(arch, "correspondence.reply", json!({"project": p, "id": c, "body": "late"}));
    w.err(eng, "correspondence.close", json!({"project": p, "id": c}));
    w.ok(eng, "correspondence.reopen", json!({"project": p, "id": c}));

    assert_eq!(w.ok(arch, "correspondence.list", json!({"project": p, "q": "steel"})).as_array().unwrap().len(), 1);
    w.err(eng, "correspondence.delete", json!({"project": p, "id": c}));
    w.ok(pm, "correspondence.delete", json!({"project": p, "id": c}));
    w.ok(pm, "correspondence.restore", json!({"project": p, "id": c}));
    let other = w.project("Other");
    w.err(pm, "correspondence.get", json!({"project": other, "id": c}));
    w.assert_replay();
}

#[test]
fn types_numbering_links_and_incoming_email() {
    use buildkiosk_engine::cmd::correspondence::parse_email;
    let mut w = World::new();
    let p = w.project("Riverside Clinic");
    let pm = w.member(p, "Paula", "Manager", "Project manager");
    let eng = w.member(p, "Eli", "Engineer", "Project engineer");
    let arch = w.member(p, "Ann", "Architect", "Owner / design team");

    // Before any type is configured the kind is free text and the plain number is the reference.
    let first =
        w.ok(eng, "correspondence.create", json!({"project": p, "subject": "Hello", "kind": "Anything", "to": [arch.raw()]}))["id"].as_u64().unwrap();
    assert_eq!(w.engine.state().correspondence.iter().next().unwrap().label(), "Correspondence #1");

    // Types are for admins; names and prefixes are unique; the prefix is alphanumeric.
    w.err(eng, "correspondence_type.save", json!({"project": p, "name": "Letter", "prefix": "LTR"}));
    w.err(pm, "correspondence_type.save", json!({"project": p, "name": "Letter", "prefix": "L-1"}));
    let letter = w.ok(pm, "correspondence_type.save", json!({"project": p, "name": "Letter", "prefix": "ltr"}))["id"].as_u64().unwrap();
    w.ok(pm, "correspondence_type.save", json!({"project": p, "name": "Notice", "prefix": "NTC"}));
    w.err(pm, "correspondence_type.save", json!({"project": p, "name": "letter", "prefix": "XX"}));
    w.err(pm, "correspondence_type.save", json!({"project": p, "name": "Memo", "prefix": "NTC"}));
    assert_eq!(w.ok(eng, "correspondence_type.list", json!({"project": p})).as_array().unwrap().len(), 2);

    // With types, kind must be one of them and numbers run within each type.
    w.err(eng, "correspondence.create", json!({"project": p, "subject": "x", "kind": "Telegram"}));
    w.err(eng, "correspondence.create", json!({"project": p, "subject": "x"}));
    let a = w.ok(eng, "correspondence.create", json!({"project": p, "subject": "One", "kind": "letter", "to": [arch.raw()]}))["id"].as_u64().unwrap();
    let b = w.ok(eng, "correspondence.create", json!({"project": p, "subject": "Two", "kind": "Letter", "to": [arch.raw()]}))["id"].as_u64().unwrap();
    let n =
        w.ok(eng, "correspondence.create", json!({"project": p, "subject": "Delay", "kind": "Notice", "to": [arch.raw()]}))["id"].as_u64().unwrap();
    let label = |w: &World, id: u64| w.engine.state().correspondence.iter().find(|c| c.id.raw() == id).unwrap().label();
    assert_eq!((label(&w, a).as_str(), label(&w, b).as_str(), label(&w, n).as_str()), ("LTR-001", "LTR-002", "NTC-001"));
    // Changing the type of a draft re-files it under the new type's next number.
    w.ok(eng, "correspondence.update", json!({"project": p, "id": a, "kind": "Notice"}));
    assert_eq!(label(&w, a), "NTC-002");
    w.ok(eng, "correspondence.update", json!({"project": p, "id": a, "subject": "One again"}));
    assert_eq!(label(&w, a), "NTC-002");
    w.ok(pm, "correspondence_type.delete", json!({"project": p, "id": letter}));
    assert_eq!(label(&w, b), "LTR-002");
    w.err(eng, "correspondence.create", json!({"project": p, "subject": "x", "kind": "Letter"}));
    let _ = first;

    // Links to RFIs and documents of the same project only.
    let rfi = w.ok(eng, "rfi.create", json!({"project": p, "subject": "Footing depth"}))["id"].as_u64().unwrap();
    let other = w.project("Other");
    let foreign = w.ok(w.admin(), "rfi.create", json!({"project": other, "subject": "Elsewhere"}))["id"].as_u64().unwrap();
    w.ok(eng, "correspondence.update", json!({"project": p, "id": n, "rfis": [rfi]}));
    w.err(eng, "correspondence.update", json!({"project": p, "id": n, "rfis": [foreign]}));
    w.err(eng, "correspondence.update", json!({"project": p, "id": n, "documents": [9999]}));
    // Submittals link the same way: the person must be able to see them.
    let sub = w.ok(eng, "submittal.create", json!({"project": p, "title": "Curtain wall"}))["id"].as_u64().unwrap();
    w.ok(eng, "correspondence.update", json!({"project": p, "id": n, "submittals": [sub]}));
    w.err(eng, "correspondence.update", json!({"project": p, "id": n, "submittals": [9999]}));
    let hidden = w.ok(pm, "submittal.create", json!({"project": p, "title": "Pm draft"}))["id"].as_u64().unwrap();
    w.err(eng, "correspondence.update", json!({"project": p, "id": n, "submittals": [hidden]}));
    assert_eq!(w.engine.state().correspondence.iter().find(|c| c.id.raw() == n).unwrap().submittals.len(), 1);
    assert_eq!(w.engine.state().correspondence.iter().find(|c| c.id.raw() == n).unwrap().rfis.len(), 1);

    // Filing an incoming letter: open at once, recipients told, the outside sender recorded.
    w.err(eng, "correspondence.file_incoming", json!({"project": p, "subject": "Claim", "from_name": "Sam Supplier", "kind": "Notice"}));
    let inc = w.ok(eng, "correspondence.file_incoming", json!({"project": p, "subject": "Price increase", "from_name": "Sam Supplier", "from_email": "sam@supplier.example", "kind": "Notice", "to": [arch.raw()], "body": "Steel is up 8%."}))["id"].as_u64().unwrap();
    let rec = w.engine.state().correspondence.iter().find(|c| c.id.raw() == inc).unwrap().clone();
    assert_eq!(
        (rec.status, rec.incoming.as_ref().unwrap().name.as_str()),
        (buildkiosk_model::correspondence::CorrespondenceStatus::Open, "Sam Supplier")
    );
    assert!(w.engine.state().notifications.iter().any(|x| x.person == arch && x.text.contains("came in from Sam Supplier")));
    w.ok(arch, "correspondence.reply", json!({"project": p, "id": inc, "body": "Thanks, noted."}));

    // An email saved from a mail program: headers, multipart text, quoted-printable, matched by address.
    let eml = "From: \"Sam Supplier\" <sam@supplier.example>\r\nTo: Ann Architect <ann.architect@example.com>, nobody@elsewhere.example\r\nCc: eli.engineer@example.com\r\nSubject: Delivery moved\r\nDate: Tue, 13 Oct 2026 09:15:00 -0500\r\nContent-Type: multipart/alternative; boundary=\"XYZ\"\r\n\r\n--XYZ\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nDelivery moves to Friday=2C 10 am.=\r\n Please confirm.\r\n--XYZ\r\nContent-Type: text/html\r\n\r\n<p>html</p>\r\n--XYZ--\r\n";
    let parsed = parse_email(eml);
    assert_eq!(
        (parsed.subject.as_str(), parsed.from_email.as_str(), parsed.to.len(), parsed.cc.len()),
        ("Delivery moved", "sam@supplier.example", 2, 1)
    );
    assert_eq!(parsed.date.unwrap().to_string(), "2026-10-13");
    assert_eq!(parsed.body, "Delivery moves to Friday, 10 am. Please confirm.");
    let e = w.ok(eng, "correspondence.import_email", json!({"project": p, "raw": eml, "kind": "Notice"}))["id"].as_u64().unwrap();
    let rec = w.engine.state().correspondence.iter().find(|c| c.id.raw() == e).unwrap().clone();
    assert_eq!(
        (rec.to.clone(), rec.cc.clone(), rec.date.to_string(), rec.incoming.as_ref().unwrap().email.as_str()),
        (vec![arch], vec![eng], "2026-10-13".to_string(), "sam@supplier.example")
    );
    w.err(eng, "correspondence.import_email", json!({"project": p, "raw": "this is not an email"}));
    w.err(eng, "correspondence.import_email", json!({"project": p, "raw": "From: x@y.example\r\nTo: unknown@y.example\r\nSubject: Hi\r\n\r\nbody"}));
    w.assert_replay();
}
