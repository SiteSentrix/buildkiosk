//! The MCP server (layer 6): the Model Context Protocol over JSON-RPC 2.0, one message per line.
//!
//! Every command in the registry is a tool, named by its id with the dot made an underscore
//! (`rfi.create` → `rfi_create`). A call runs as the person the server was started for, so the
//! engine's permission checks apply exactly as they do in the web app. Queries are marked
//! read-only. Input is untrusted: nothing here panics, and every failure becomes a JSON-RPC error
//! or a tool result with `isError` set.
//!
//! Tool results carry the command's JSON twice: as text (for clients that only read text) and as
//! `structuredContent` (an object; a list or a single value is wrapped as `{"result": …}`). The
//! project's documents are resources: `resources/list` names the ones the person may open and
//! `resources/read` returns the current version's file (text as text, anything else as base64).
#![forbid(unsafe_code)]

use buildkiosk_core::Timestamp;
use buildkiosk_engine::{Actor, all_commands, find_command};
use buildkiosk_store::Tenant;
use serde_json::{Map, Value, json};

/// The protocol version offered when the client names none we can echo.
const DEFAULT_PROTOCOL: &str = "2024-11-05";

const INSTRUCTIONS: &str = "BuildKiosk construction management. Every tool is a command; tool names are command ids with the dot \
written as an underscore (rfi.create is rfi_create). Most project tools take `project` (a project id) and `id` (an item id). \
Start with me_get to see who you are and which projects you can open, then project_list and the tool's own list command. \
Failed calls return isError with the reason; nothing is changed by a failed call. Results come as text and as \
structuredContent. Project documents are resources (resources/list, resources/read).";

/// Handles one line of input. Returns the line to send back, or `None` for notifications and
/// blank lines. `now` is the time commands run at.
pub fn handle_line(tenant: &mut Tenant, actor: Actor, now: Timestamp, line: &str) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let reply = match serde_json::from_str::<Value>(line) {
        Err(e) => Some(error(Value::Null, -32700, &format!("parse error: {e}"))),
        Ok(Value::Array(batch)) => {
            if batch.is_empty() {
                Some(error(Value::Null, -32600, "an empty batch is not a valid request"))
            } else {
                let replies: Vec<Value> = batch.iter().filter_map(|m| handle_message(tenant, actor, now, m)).collect();
                if replies.is_empty() { None } else { Some(Value::Array(replies)) }
            }
        }
        Ok(message) => handle_message(tenant, actor, now, &message),
    };
    reply.map(|v| v.to_string())
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn handle_message(tenant: &mut Tenant, actor: Actor, now: Timestamp, message: &Value) -> Option<Value> {
    let Some(obj) = message.as_object() else { return Some(error(Value::Null, -32600, "a request must be a JSON object")) };
    let Some(method) = obj.get("method").and_then(Value::as_str) else {
        // A response from the client (to a request we never send) or something malformed: ignore.
        return if obj.contains_key("result") || obj.contains_key("error") {
            None
        } else {
            Some(error(obj.get("id").cloned().unwrap_or(Value::Null), -32600, "missing method"))
        };
    };
    // Without an id it is a notification: act on it, never reply.
    let id = obj.get("id").cloned()?;
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    Some(match method {
        "initialize" => ok(id, initialize(&params)),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => match call(tenant, actor, now, &params) {
            Ok(result) => ok(id, result),
            Err((code, msg)) => error(id, code, &msg),
        },
        "resources/list" => match resources::list(tenant, actor, &params) {
            Ok(result) => ok(id, result),
            Err((code, msg)) => error(id, code, &msg),
        },
        "resources/templates/list" => ok(id, resources::templates()),
        "resources/read" => match resources::read(tenant, actor, &params) {
            Ok(result) => ok(id, result),
            Err((code, msg)) => error(id, code, &msg),
        },
        "prompts/list" => ok(id, json!({ "prompts": [] })),
        other => error(id, -32601, &format!("method `{other}` is not supported")),
    })
}

fn initialize(params: &Value) -> Value {
    let version = params.get("protocolVersion").and_then(Value::as_str).filter(|v| v.len() <= 32).unwrap_or(DEFAULT_PROTOCOL);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": { "listChanged": false }, "resources": { "subscribe": false, "listChanged": false } },
        "serverInfo": { "name": "buildkiosk", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

/// The tool name of a command.
pub fn tool_name(command_id: &str) -> String {
    command_id.replace('.', "_")
}

/// The input schema of a command, read best-effort from its parameter documentation
/// (`name: type; other?: type`). Parameters are described, not enforced here: the engine checks
/// every call.
fn input_schema(params_doc: &str) -> Value {
    let mut properties = Map::new();
    let identifier = |n: &str| !n.is_empty() && n.trim_end_matches('?').chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    for segment in params_doc.split(';') {
        // Some docs lead with prose ("then the RFI fields: subject: text"): skip to the first part
        // that reads as parameter names.
        let mut rest = segment;
        while let Some((names, what)) = rest.split_once(':') {
            let list: Vec<&str> = names.split(',').map(str::trim).collect();
            if list.iter().all(|n| identifier(n)) {
                for name in list {
                    let key = name.trim_end_matches('?');
                    let mut description = what.trim().to_string();
                    if name.ends_with('?') {
                        description.push_str(" (optional)");
                    }
                    properties.insert(key.to_string(), json!({ "description": description }));
                }
                break;
            }
            rest = what;
        }
    }
    json!({ "type": "object", "properties": properties, "additionalProperties": true, "description": format!("Parameters: {params_doc}") })
}

fn tools() -> Vec<Value> {
    all_commands()
        .map(|c| {
            json!({
                "name": tool_name(c.id),
                "title": c.label,
                "description": format!("{} ({}). Parameters: {}", c.label, c.id, c.params),
                "inputSchema": input_schema(c.params),
                "annotations": { "readOnlyHint": c.is_query(), "destructiveHint": false, "openWorldHint": false },
            })
        })
        .collect()
}

/// Runs a `tools/call`. Protocol problems are `Err((code, message))`; a command that fails is a
/// successful response whose result has `isError` set, so the agent can read why.
fn call(tenant: &mut Tenant, actor: Actor, now: Timestamp, params: &Value) -> Result<Value, (i64, String)> {
    let name = params.get("name").and_then(Value::as_str).ok_or((-32602, "tools/call needs a `name`".to_string()))?;
    let spec = all_commands().find(|c| tool_name(c.id) == name).ok_or_else(|| (-32602, format!("unknown tool `{name}`")))?;
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ Value::Object(_)) => v.clone(),
        Some(_) => return Err((-32602, "`arguments` must be an object".to_string())),
    };
    debug_assert!(find_command(spec.id).is_some());
    let Tenant { engine, store, .. } = tenant;
    Ok(match engine.execute(store, actor, now, spec.id, &arguments) {
        Ok(value) => {
            let text = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
            let structured = if value.is_object() { value } else { json!({ "result": value }) };
            json!({ "content": [{ "type": "text", "text": text }], "structuredContent": structured, "isError": false })
        }
        Err(e) => json!({
            "content": [{ "type": "text", "text": format!("{}: {e}", e.kind()) }],
            "structuredContent": { "error": { "kind": e.kind(), "message": e.to_string() } },
            "isError": true,
        }),
    })
}

mod resources;

#[cfg(test)]
mod tests;
