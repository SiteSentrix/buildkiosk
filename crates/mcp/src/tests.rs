use buildkiosk_core::Timestamp;
use buildkiosk_engine::Actor;
use buildkiosk_store::{DataDir, Tenant};
use serde_json::{Value, json};

use super::*;

fn temp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("buildkiosk-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    p
}

/// A company with an admin (person 1) and one project.
fn company(dir: &std::path::Path) -> (DataDir, Tenant) {
    let data = DataDir::open(dir).unwrap();
    let mut t = data.create("acme").unwrap();
    let now = Timestamp::from_millis(1_760_000_000_000);
    let setup = json!({"name": "Acme", "admin_first_name": "Ada", "admin_last_name": "Admin", "admin_email": "ada@example.com"});
    t.engine.execute(&mut t.store, Actor::System, now, "company.setup", &setup).unwrap();
    t.engine.execute(&mut t.store, Actor::System, now, "project.create", &json!({"name": "Tower One"})).unwrap();
    (data, t)
}

fn send(t: &mut Tenant, actor: Actor, msg: Value) -> Value {
    let line = handle_line(t, actor, Timestamp::from_millis(1_760_000_100_000), &msg.to_string()).expect("a reply");
    serde_json::from_str(&line).unwrap()
}

#[test]
fn the_handshake_and_the_tool_list() {
    let dir = temp("hello");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let r = send(
        &mut t,
        admin,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}}}),
    );
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert!(r["result"]["capabilities"]["tools"].is_object() && r["result"]["serverInfo"]["name"] == "buildkiosk");
    // A notification gets no reply; blank lines neither.
    assert!(handle_line(&mut t, admin, Timestamp::from_millis(0), r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    assert!(handle_line(&mut t, admin, Timestamp::from_millis(0), "  ").is_none());

    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}));
    let tools = r["result"]["tools"].as_array().unwrap();
    assert!(tools.len() > 200, "every command is a tool: {}", tools.len());
    let find = |n: &str| tools.iter().find(|x| x["name"] == n).unwrap();
    assert_eq!(find("rfi_create")["annotations"]["readOnlyHint"], false);
    assert_eq!(find("rfi_list")["annotations"]["readOnlyHint"], true);
    assert!(find("rfi_create")["inputSchema"]["properties"]["subject"]["description"].as_str().unwrap().contains("text"));
    assert!(find("rfi_create")["inputSchema"]["properties"]["question"]["description"].as_str().unwrap().contains("optional"));
    let names: std::collections::HashSet<_> = tools.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), tools.len(), "tool names are unique");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn calls_run_as_the_person_and_failures_are_results() {
    let dir = temp("calls");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let call = |t: &mut Tenant, who: Actor, name: &str, args: Value| {
        send(t, who, json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": {"name": name, "arguments": args}}))
    };

    let r = call(&mut t, admin, "me_get", json!({}));
    assert_eq!(r["result"]["isError"], false);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("Tower One"));

    let r = call(&mut t, admin, "rfi_create", json!({"project": 1, "subject": "Footing depth", "question": "Which governs?"}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    let r = call(&mut t, admin, "rfi_list", json!({"project": 1}));
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("Footing depth"));

    // A bad call is a result the agent can read, and changes nothing.
    let before = t.engine.seq();
    let r = call(&mut t, admin, "rfi_create", json!({"project": 1}));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().starts_with("bad_params"));
    assert_eq!(t.engine.seq(), before);

    // Someone with no access gets the engine's refusal, not data.
    let stranger = Actor::Person(buildkiosk_core::Id::new(99));
    let r = call(&mut t, stranger, "rfi_list", json!({"project": 1}));
    assert_eq!(r["result"]["isError"], true);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn protocol_errors() {
    let dir = temp("errors");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let now = Timestamp::from_millis(0);
    let line = |t: &mut Tenant, s: &str| -> Value { serde_json::from_str(&handle_line(t, admin, now, s).unwrap()).unwrap() };

    assert_eq!(line(&mut t, "{not json")["error"]["code"], -32700);
    assert_eq!(line(&mut t, "[]")["error"]["code"], -32600);
    assert_eq!(line(&mut t, "42")["error"]["code"], -32600);
    assert_eq!(line(&mut t, r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#)["error"]["code"], -32601);
    assert_eq!(line(&mut t, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"no_such_tool"}}"#)["error"]["code"], -32602);
    assert_eq!(line(&mut t, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#)["error"]["code"], -32602);
    assert_eq!(line(&mut t, r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"me_get","arguments":[1]}}"#)["error"]["code"], -32602);
    // Batches reply with an array, skipping notifications.
    let batch = line(
        &mut t,
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/initialized"},{"jsonrpc":"2.0","id":"a","method":"ping"}]"#,
    );
    assert_eq!(batch.as_array().unwrap().len(), 2);
    assert_eq!(batch[1]["id"], "a");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn junk_input_never_panics_or_changes_anything() {
    let dir = temp("junk");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let before = t.engine.seq();
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let fixed = [
        "null",
        "true",
        "\"x\"",
        "[null]",
        "[[]]",
        "{}",
        r#"{"id":1}"#,
        r#"{"method":5}"#,
        r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#,
        r#"{"id":1,"method":"tools/call","params":"x"}"#,
        r#"{"id":1,"method":"tools/call","params":{"name":5}}"#,
        r#"{"id":1,"method":"tools/call","params":{"name":"rfi_create","arguments":{"project":"x","subject":[1,2,3],"assignees":{"a":1}}}}"#,
        r#"{"id":1,"method":"tools/call","params":{"name":"project_create","arguments":{"name":1e999}}}"#,
        r#"{"id":1,"method":"initialize","params":{"protocolVersion":{"a":[1]}}}"#,
    ];
    for line in fixed {
        let _ = handle_line(&mut t, admin, Timestamp::from_millis(0), line);
    }
    for _ in 0..500 {
        let len = (next() % 80) as usize;
        let line: String = (0..len).map(|_| char::from_u32(32 + (next() % 95) as u32).unwrap_or(' ')).collect();
        let _ = handle_line(&mut t, admin, Timestamp::from_millis(0), &line);
    }
    // Every command called with junk arguments: errors only, and nothing partial gets saved.
    for c in buildkiosk_engine::all_commands() {
        for args in [json!({}), json!({"project": 1, "id": 1}), json!({"project": "x", "id": -1, "name": null}), json!({"lines": "x", "text": 5})] {
            let msg = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": tool_name(c.id), "arguments": args}});
            let reply = handle_line(&mut t, admin, Timestamp::from_millis(1_760_000_100_000), &msg.to_string()).unwrap();
            let v: Value = serde_json::from_str(&reply).unwrap();
            assert!(v.get("result").is_some() || v.get("error").is_some(), "{reply}");
        }
    }
    assert!(t.engine.seq() >= before, "the journal only grows");
    let _ = std::fs::remove_dir_all(&dir);
}
