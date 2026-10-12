//! The journal: failed writes change nothing, no-op commands aren't journaled, and replaying any
//! sequence of commands rebuilds the same state.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_core::Timestamp;
use buildkiosk_engine::{Engine, EngineError};
use buildkiosk_model::SnapshotIn;
use buildkiosk_testkit::{FailingJournal, World};
use proptest::prelude::*;
use serde_json::{Value, json};

#[test]
fn a_failed_journal_write_changes_nothing() {
    let mut w = World::new();
    let before = w.engine.state().clone();
    let seq = w.engine.seq();
    let admin = w.admin();
    let e = w.engine.execute(&mut FailingJournal, admin, Timestamp::from_millis(w.clock), "project.create", &json!({"name": "X"})).unwrap_err();
    assert!(matches!(e, EngineError::Storage(_)), "{e}");
    assert!(w.engine.state() == &before);
    assert_eq!(w.engine.seq(), seq);
}

#[test]
fn queries_and_no_op_updates_are_not_journaled() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let n = w.journal.entries.len();
    let admin = w.admin();
    w.ok(admin, "project.list", json!({}));
    w.ok(admin, "project.update", json!({"project": p}));
    assert_eq!(w.journal.entries.len(), n);
    w.ok(admin, "project.update", json!({"project": p, "name": "Clinic B"}));
    assert_eq!(w.journal.entries.len(), n + 1);
    let last = w.journal.entries.last().unwrap();
    assert_eq!(last.cmd, "project.update");
    assert_eq!(last.actor, Some(w.admin));
    assert!(last.touches("projects", p.raw()));
}

#[test]
fn snapshots_round_trip_through_json() {
    let mut w = World::new();
    let p = w.project("Clinic");
    let admin = w.admin;
    w.ok(admin, "rfi.create", json!({"project": p, "subject": "s", "question": "q", "assignees": [admin], "open": true}));
    let text = serde_json::to_string(&w.engine.state().snapshot(w.engine.seq())).unwrap();
    let (state, seq) = serde_json::from_str::<SnapshotIn>(&text).unwrap().into_state().unwrap();
    assert_eq!(seq, w.engine.seq());
    assert!(&state == w.engine.state());
    // Entries after a snapshot replay on top of it.
    let mut e = Engine::from_state(state, seq);
    w.ok(admin, "rfi.respond", json!({"project": p, "id": 1, "body": "answer"}));
    e.apply(w.journal.entries.last().unwrap()).unwrap();
    assert!(e.state() == w.engine.state());
    // Out-of-order entries are refused.
    assert!(e.apply(w.journal.entries.last().unwrap()).is_err());
}

/// A random but plausible command, built from small id and text pools so that sequences hit real
/// records and real conflicts.
fn arb_command() -> impl Strategy<Value = (usize, &'static str, Value)> {
    let actor = 0usize..4;
    let id = 1u64..5;
    let text = prop_oneof![Just("Footing depth".to_string()), Just(String::new()), "[a-z ]{0,12}"];
    let cmd = prop_oneof![
        (text.clone()).prop_map(|t| ("project.create", json!({"name": t}))),
        (id.clone(), text.clone()).prop_map(|(p, t)| ("rfi.create", json!({"project": p, "subject": t, "question": "q", "assignees": [1, 2]}))),
        (id.clone(), id.clone(), any::<bool>())
            .prop_map(|(p, i, d)| ("rfi.open", json!({"project": p, "id": i, "due": if d { json!("2026-11-02") } else { Value::Null }}))),
        (id.clone(), id.clone(), text.clone()).prop_map(|(p, i, t)| ("rfi.respond", json!({"project": p, "id": i, "body": t}))),
        (id.clone(), id.clone()).prop_map(|(p, i)| ("rfi.close", json!({"project": p, "id": i}))),
        (id.clone(), id.clone()).prop_map(|(p, i)| ("rfi.reopen", json!({"project": p, "id": i}))),
        (id.clone(), id.clone()).prop_map(|(p, i)| ("rfi.delete", json!({"project": p, "id": i}))),
        (id.clone(), id.clone()).prop_map(|(p, i)| ("rfi.restore", json!({"project": p, "id": i}))),
        (id.clone(), id.clone(), text.clone()).prop_map(|(p, i, t)| ("rfi.update", json!({"project": p, "id": i, "subject": t, "private": true}))),
        (id.clone(), id.clone()).prop_map(|(p, person)| ("member.add", json!({"project": p, "person": person, "template": 4}))),
        (id.clone(), id.clone()).prop_map(|(p, person)| ("member.remove", json!({"project": p, "person": person}))),
        (text.clone()).prop_map(|t| ("vendor.create", json!({"name": t}))),
        (id.clone(), text).prop_map(|(i, t)| ("person.update", json!({"id": i, "title": t}))),
    ];
    (actor, cmd).prop_map(|(a, (c, p))| (a, c, p))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, ..ProptestConfig::default() })]

    #[test]
    fn replay_rebuilds_the_state(cmds in proptest::collection::vec(arb_command(), 1..40)) {
        let mut w = World::new();
        let people = [w.admin, w.user("Sam", "Sub"), w.user("Arch", "Itect"), w.user("Rita", "Reader")];
        w.project("Seed project");
        for (a, cmd, params) in cmds {
            let actor = people[a % people.len()];
            if let Err(e @ (EngineError::Internal { .. } | EngineError::Storage(_))) = w.run(actor, cmd, params) {
                prop_assert!(false, "{e}");
            }
        }
        w.assert_replay();
    }
}
