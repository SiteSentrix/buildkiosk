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
