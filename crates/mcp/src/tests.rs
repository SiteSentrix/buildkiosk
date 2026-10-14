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
        r#"{"id":1,"method":"resources/read","params":"x"}"#,
        r#"{"id":1,"method":"resources/read","params":{"uri":5}}"#,
        r#"{"id":1,"method":"resources/read","params":{"uri":"buildkiosk://document/99999999999999999999/1"}}"#,
        r#"{"id":1,"method":"resources/read","params":{"uri":"buildkiosk://document/1/"}}"#,
        r#"{"id":1,"method":"resources/list","params":{"cursor":[1]}}"#,
        r#"{"id":1,"method":"resources/list","params":{"cursor":"18446744073709551615"}}"#,
        r#"{"id":1,"method":"resources/list","params":"x"}"#,
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

fn call_tool(t: &mut Tenant, who: Actor, name: &str, args: Value) -> Value {
    send(t, who, json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": name, "arguments": args}}))
}

#[test]
fn results_are_structured_too() {
    let dir = temp("structured");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    // An object comes back as it is, beside the text.
    let r = call_tool(&mut t, admin, "project_get", json!({"project": 1}));
    assert_eq!(r["result"]["structuredContent"]["name"], "Tower One", "{r}");
    let text: Value = serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, r["result"]["structuredContent"]);
    // A list is wrapped, because structured content is an object.
    let r = call_tool(&mut t, admin, "rfi_list", json!({"project": 1}));
    assert_eq!(r["result"]["structuredContent"]["result"], json!([]), "{r}");
    // A failure says what kind and why.
    let r = call_tool(&mut t, admin, "rfi_create", json!({"project": 1}));
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(r["result"]["structuredContent"]["error"]["kind"], "bad_params");
    assert!(r["result"]["structuredContent"]["error"]["message"].as_str().unwrap().contains("subject"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn documents_are_resources_for_the_people_who_may_open_them() {
    let dir = temp("resources");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let stranger = Actor::Person(buildkiosk_core::Id::new(99));
    let put = |t: &Tenant, bytes: &[u8]| t.store.blobs.put(bytes).unwrap();
    let (text_sha, text_size) = put(&t, b"Pour the slab at 4,000 psi.");
    let (bin_sha, bin_size) = put(&t, &[0, 159, 146, 150, 255]);
    let add = |t: &mut Tenant, folder: Option<u64>, title: &str, sha: &str, size: u64, name: &str, mime: &str| {
        let mut args = json!({"project": 1, "title": title, "sha256": sha, "size": size, "name": name, "mime": mime});
        if let Some(f) = folder {
            args["folder"] = json!(f);
        }
        let r = call_tool(t, Actor::Person(buildkiosk_core::Id::new(1)), "document_add", args);
        assert_eq!(r["result"]["isError"], false, "{r}");
    };
    add(&mut t, None, "Concrete notes", &text_sha, text_size, "notes.txt", "text/plain");
    add(&mut t, None, "Scan", &bin_sha, bin_size, "scan.bin", "application/octet-stream");

    // The handshake offers resources; the list names both, with type and size.
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert!(r["result"]["capabilities"]["resources"].is_object());
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}));
    let list = r["result"]["resources"].as_array().unwrap();
    assert_eq!(list.len(), 2, "{r}");
    assert_eq!(
        (
            list[0]["uri"].as_str().unwrap(),
            list[0]["name"].as_str().unwrap(),
            list[0]["mimeType"].as_str().unwrap(),
            list[0]["size"].as_u64().unwrap()
        ),
        ("buildkiosk://document/1/1", "Concrete notes", "text/plain", text_size)
    );
    assert!(r["result"].get("nextCursor").is_none());
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/templates/list"}));
    assert!(r["result"]["resourceTemplates"][0]["uriTemplate"].as_str().unwrap().starts_with("buildkiosk://document/"));

    // Text is read as text, anything else as base64.
    let read =
        |t: &mut Tenant, who: Actor, uri: &str| send(t, who, json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": uri}}));
    let r = read(&mut t, admin, "buildkiosk://document/1/1");
    assert_eq!(r["result"]["contents"][0]["text"], "Pour the slab at 4,000 psi.", "{r}");
    let r = read(&mut t, admin, "buildkiosk://document/1/2");
    let blob = r["result"]["contents"][0]["blob"].as_str().unwrap();
    assert_eq!(base64::Engine::decode(&base64::engine::general_purpose::STANDARD, blob).unwrap(), vec![0, 159, 146, 150, 255]);

    // Someone with no access sees none and can't read by guessing the address; wrong project and junk are not found.
    let r = send(&mut t, stranger, json!({"jsonrpc": "2.0", "id": 5, "method": "resources/list"}));
    assert_eq!(r["result"]["resources"].as_array().unwrap().len(), 0);
    for (who, uri) in [
        (stranger, "buildkiosk://document/1/1"),
        (admin, "buildkiosk://document/2/1"),
        (admin, "buildkiosk://document/1/99"),
        (admin, "buildkiosk://document/x/y"),
        (admin, "file:///etc/passwd"),
        (admin, ""),
    ] {
        let r = read(&mut t, who, uri);
        assert_eq!(r["error"]["code"], -32002, "{uri}: {r}");
    }
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 6, "method": "resources/read", "params": {}}));
    assert_eq!(r["error"]["code"], -32602);
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 7, "method": "resources/list", "params": {"cursor": "banana"}}));
    assert_eq!(r["error"]["code"], -32602);

    // A deleted document disappears; long lists come in pages.
    let r = call_tool(&mut t, admin, "document_delete", json!({"project": 1, "id": 2}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(read(&mut t, admin, "buildkiosk://document/1/2")["error"]["code"], -32002);
    for n in 0..120 {
        add(&mut t, None, &format!("Copy {n}"), &text_sha, text_size, "notes.txt", "text/plain");
    }
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"}));
    assert_eq!((r["result"]["resources"].as_array().unwrap().len(), r["result"]["nextCursor"].as_str()), (100, Some("100")));
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 9, "method": "resources/list", "params": {"cursor": "100"}}));
    assert_eq!((r["result"]["resources"].as_array().unwrap().len(), r["result"].get("nextCursor").is_none()), (21, true));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drawings_and_photos_are_resources_too() {
    let dir = temp("sheets");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let stranger = Actor::Person(buildkiosk_core::Id::new(99));
    let (pdf_sha, pdf_size) = t.store.blobs.put(b"%PDF-1.4 A101").unwrap();
    let (png_sha, png_size) = t.store.blobs.put(b"\x89PNG\r\n\x1a\n-site").unwrap();
    let r = call_tool(
        &mut t,
        admin,
        "drawing_add",
        json!({"project": 1, "number": "A101", "title": "First floor plan", "revision": "A", "sha256": pdf_sha, "name": "A101.pdf", "size": pdf_size, "mime": "application/pdf"}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    let r = call_tool(
        &mut t,
        admin,
        "photo_add",
        json!({"project": 1, "sha256": png_sha, "name": "IMG_1.png", "size": png_size, "mime": "image/png", "caption": "Footing forms"}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");

    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}));
    let uris: Vec<&str> = r["result"]["resources"].as_array().unwrap().iter().map(|x| x["uri"].as_str().unwrap()).collect();
    assert_eq!(uris, vec!["buildkiosk://drawing/1/1", "buildkiosk://photo/1/1"], "{r}");
    let names: Vec<&str> = r["result"]["resources"].as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["A101 First floor plan", "Footing forms"]);
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/templates/list"}));
    assert_eq!(r["result"]["resourceTemplates"].as_array().unwrap().len(), 3);

    let read =
        |t: &mut Tenant, who: Actor, uri: &str| send(t, who, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": uri}}));
    let r = read(&mut t, admin, "buildkiosk://drawing/1/1");
    let blob = r["result"]["contents"][0]["blob"].as_str().unwrap();
    assert_eq!(base64::Engine::decode(&base64::engine::general_purpose::STANDARD, blob).unwrap(), b"%PDF-1.4 A101");
    assert_eq!(r["result"]["contents"][0]["mimeType"], "application/pdf");
    let r = read(&mut t, admin, "buildkiosk://photo/1/1");
    assert_eq!(r["result"]["contents"][0]["mimeType"], "image/png", "{r}");
    // A person with no access gets nothing, and a document address can't reach a photo.
    for uri in ["buildkiosk://drawing/1/1", "buildkiosk://photo/1/1"] {
        assert_eq!(read(&mut t, stranger, uri)["error"]["code"], -32002, "{uri}");
    }
    assert_eq!(read(&mut t, admin, "buildkiosk://document/1/1")["error"]["code"], -32002);
    assert_eq!(read(&mut t, admin, "buildkiosk://photo/1/2")["error"]["code"], -32002);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prompts_are_offered_filled_in_and_name_real_tools() {
    let dir = temp("prompts");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert!(r["result"]["capabilities"]["prompts"].is_object());
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/list"}));
    let list = r["result"]["prompts"].as_array().unwrap().clone();
    assert!(list.len() >= 4 && list.iter().all(|p| p["name"].is_string() && p["arguments"].is_array()), "{r}");

    // Every tool a prompt mentions exists.
    let tools: std::collections::HashSet<String> = all_commands().map(|c| tool_name(c.id)).collect();
    for p in &list {
        let args: Map<String, Value> =
            p["arguments"].as_array().unwrap().iter().map(|a| (a["name"].as_str().unwrap().to_string(), json!("1"))).collect();
        let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 3, "method": "prompts/get", "params": {"name": p["name"], "arguments": args}}));
        let text = r["result"]["messages"][0]["content"]["text"].as_str().unwrap_or_else(|| panic!("{r}"));
        for word in text.split(|c: char| !(c.is_ascii_lowercase() || c == '_')).filter(|w| w.contains('_')) {
            let looks_like_tool = ["_list", "_get", "_create", "_lookahead", "_loading"].iter().any(|s| word.ends_with(s));
            assert!(!looks_like_tool || tools.contains(word), "prompt {} names {word}, which is not a tool", p["name"]);
        }
    }
    // Arguments are filled in; a missing required one, an unknown one and an unknown prompt are refused.
    let r = send(
        &mut t,
        admin,
        json!({"jsonrpc": "2.0", "id": 4, "method": "prompts/get", "params": {"name": "project_status", "arguments": {"project": "7"}}}),
    );
    assert!(r["result"]["messages"][0]["content"]["text"].as_str().unwrap().contains("project 7"), "{r}");
    for params in [
        json!({"name": "project_status"}),
        json!({"name": "project_status", "arguments": {"project": "1", "bogus": "x"}}),
        json!({"name": "nothing"}),
        json!({}),
        json!("x"),
    ] {
        let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 5, "method": "prompts/get", "params": params}));
        assert_eq!(r["error"]["code"], -32602, "{params}: {r}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_client_can_follow_a_resource_and_is_told_when_it_changes() {
    let dir = temp("subscribe");
    let (_data, mut t) = company(&dir);
    let admin = Actor::Person(buildkiosk_core::Id::new(1));
    let stranger = Actor::Person(buildkiosk_core::Id::new(99));
    let now = Timestamp::from_millis(1_760_000_100_000);
    let put = |t: &Tenant, bytes: &[u8]| t.store.blobs.put(bytes).unwrap();
    let (v1, n1) = put(&t, b"first draft");
    let (v2, n2) = put(&t, b"second draft");
    let r = call_tool(
        &mut t,
        admin,
        "document_add",
        json!({"project": 1, "title": "Notes", "sha256": v1, "size": n1, "name": "n.txt", "mime": "text/plain"}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    let uri = "buildkiosk://document/1/1";
    let say = |s: &mut Session, t: &mut Tenant, who: Actor, v: Value| -> Value {
        serde_json::from_str(&s.handle_line(t, who, now, &v.to_string()).expect("a reply")).unwrap()
    };

    // The handshake offers subscriptions only to a connection that keeps state.
    let mut s = Session::default();
    let r = say(&mut s, &mut t, admin, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert_eq!(r["result"]["capabilities"]["resources"]["subscribe"], true);
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    assert_eq!(r["result"]["capabilities"]["resources"]["subscribe"], false);
    let r = send(&mut t, admin, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/subscribe", "params": {"uri": uri}}));
    assert_eq!(r["error"]["code"], -32601);

    // Something you can't open can't be followed; what you can, can.
    let mut other = Session::default();
    let r = say(&mut other, &mut t, stranger, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/subscribe", "params": {"uri": uri}}));
    assert_eq!(r["error"]["code"], -32002);
    for bad in [json!({}), json!({"uri": 5}), json!({"uri": "buildkiosk://document/1/99"}), json!({"uri": "file:///etc/passwd"})] {
        let r = say(&mut s, &mut t, admin, json!({"jsonrpc": "2.0", "id": 4, "method": "resources/subscribe", "params": bad}));
        assert!(r["error"].is_object(), "{bad}");
    }
    let r = say(&mut s, &mut t, admin, json!({"jsonrpc": "2.0", "id": 5, "method": "resources/subscribe", "params": {"uri": uri}}));
    assert!(r["result"].is_object(), "{r}");
    assert_eq!(s.subscriptions(), 1);

    // Nothing changed: nothing to say. A new version: one notification, then quiet again.
    assert!(s.poll(&t, admin).is_empty());
    let r = call_tool(
        &mut t,
        admin,
        "document_add_version",
        json!({"project": 1, "id": 1, "sha256": v2, "size": n2, "name": "n.txt", "mime": "text/plain"}),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    let sent = s.poll(&t, admin);
    assert_eq!(sent.len(), 1);
    let note: Value = serde_json::from_str(&sent[0]).unwrap();
    assert_eq!((note["method"].as_str(), note["params"]["uri"].as_str(), note.get("id")), (Some("notifications/resources/updated"), Some(uri), None));
    assert!(s.poll(&t, admin).is_empty());

    // Deleting it is a change too (a read now says not found); unsubscribing stops the news.
    let r = call_tool(&mut t, admin, "document_delete", json!({"project": 1, "id": 1}));
    assert_eq!(r["result"]["isError"], false, "{r}");
    assert_eq!(s.poll(&t, admin).len(), 1);
    let r = say(&mut s, &mut t, admin, json!({"jsonrpc": "2.0", "id": 6, "method": "resources/unsubscribe", "params": {"uri": uri}}));
    assert!(r["result"].is_object());
    assert_eq!(s.subscriptions(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
