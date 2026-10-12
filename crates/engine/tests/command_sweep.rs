//! Every command, fed junk parameters by every kind of actor, returns a result or a clean error:
//! never a panic (the engine reports those as `EngineError::Internal`).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use buildkiosk_engine::{Actor, EngineError, all_commands};
use buildkiosk_testkit::World;
use proptest::prelude::*;
use serde_json::{Map, Value, json};

/// Junk values: wrong types, huge numbers, hostile strings, deep nesting.
fn junk() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>().prop_map(|f| serde_json::Number::from_f64(f).map(Value::Number).unwrap_or(Value::Null)),
        "\\PC{0,30}".prop_map(Value::String),
        Just(json!("1")),
        Just(json!("2026-02-30")),
        Just(json!("-0")),
        Just(json!("99999999999999999999999")),
        Just(json!("1,000,000,000,000,000.00")),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            proptest::collection::vec(inner.clone(), 0..4).prop_map(Value::Array),
            proptest::collection::btree_map("[a-z_]{1,10}", inner, 0..4).prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

/// Parameter names commands use, so junk lands on real keys.
const KEYS: &[&str] = &[
    "project",
    "id",
    "person",
    "name",
    "subject",
    "question",
    "body",
    "assignees",
    "manager",
    "due",
    "status",
    "levels",
    "overrides",
    "template",
    "text",
    "email",
    "is_user",
    "company_admin",
    "active",
    "value",
    "start",
    "completion",
    "response",
    "official",
    "open",
    "private",
    "cost_amount",
    "schedule_days",
    "members",
    "vendor",
    "kind",
    "q",
    "mine",
    "overdue",
    "deleted",
    "first_name",
    "last_name",
    "code",
    "description",
    "replace",
];

fn params() -> impl Strategy<Value = Value> {
    proptest::collection::vec((proptest::sample::select(KEYS), junk()), 0..8).prop_map(|kv| {
        let mut m = Map::new();
        for (k, v) in kv {
            m.insert(k.to_string(), v);
        }
        Value::Object(m)
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn no_command_panics_on_junk(actor in 0usize..3, raw in params(), top_level in junk()) {
        let mut w = World::new();
        let p = w.project("Clinic");
        let sub = w.member(p, "Sam", "Sub", "Subcontractor");
        let actors = [Actor::System, w.admin(), Actor::Person(sub)];
        for spec in all_commands() {
            for params in [&raw, &top_level] {
                if let Err(EngineError::Internal { cmd, msg }) = w.run(actors[actor], spec.id, params.clone()) {
                    prop_assert!(false, "{cmd} panicked on {params}: {msg}");
                }
            }
        }
        w.assert_replay();
    }
}

#[test]
fn every_command_is_documented_and_named_by_tool() {
    let mut ids = std::collections::HashSet::new();
    for c in all_commands() {
        assert!(ids.insert(c.id), "duplicate command id {}", c.id);
        assert!(c.id.contains('.') && c.id.chars().all(|ch| ch.is_ascii_lowercase() || ch == '.' || ch == '_'), "bad id {}", c.id);
        assert!(!c.label.is_empty() && !c.params.is_empty(), "{} needs a label and a params doc", c.id);
    }
}
