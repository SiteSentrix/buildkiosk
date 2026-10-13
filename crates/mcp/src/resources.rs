//! Documents as MCP resources.

use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use buildkiosk_core::Id;
use buildkiosk_engine::Actor;
use buildkiosk_engine::model::document::Document;
use buildkiosk_engine::view;
use buildkiosk_store::Tenant;
use serde_json::{Value, json};

/// Resources returned per page of `resources/list`.
const PAGE: usize = 100;
/// The largest file `resources/read` returns (it goes out as one JSON message).
const MAX_READ: u64 = 8 * 1024 * 1024;
const PREFIX: &str = "buildkiosk://document/";

type Failure = (i64, String);

fn uri(doc: &Document) -> String {
    format!("{PREFIX}{}/{}", doc.project, doc.id)
}

/// `buildkiosk://document/{project}/{id}` → (project, id).
fn parse(uri: &str) -> Option<(u64, u64)> {
    let (project, id) = uri.strip_prefix(PREFIX)?.split_once('/')?;
    Some((project.parse().ok()?, id.parse().ok()?))
}

/// Whether a media type is read as text.
fn is_text(mime: &str) -> bool {
    let mime = mime.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    mime.starts_with("text/")
        || mime.ends_with("+json")
        || mime.ends_with("+xml")
        || matches!(mime.as_str(), "application/json" | "application/xml" | "application/csv")
}

pub fn templates() -> Value {
    json!({ "resourceTemplates": [{
        "uriTemplate": format!("{PREFIX}{{project}}/{{id}}"),
        "name": "document",
        "title": "Project document",
        "description": "The current version of a document in a project's Documents tool.",
    }] })
}

/// The documents the person may open, oldest first, a page at a time (`cursor` is how many to skip).
pub fn list(tenant: &Tenant, actor: Actor, params: &Value) -> Result<Value, Failure> {
    let skip = match params.get("cursor") {
        None | Some(Value::Null) => 0,
        Some(Value::String(s)) => s.parse::<usize>().map_err(|_| (-32602, "`cursor` is not one this server gave".to_string()))?,
        Some(_) => return Err((-32602, "`cursor` must be text".to_string())),
    };
    let state = tenant.engine.state();
    let mut docs: Vec<&Document> = state.documents.iter().filter(|d| d.deleted.is_none() && view::can_see_document(state, actor, d)).collect();
    docs.sort_by_key(|d| d.id);
    let total = docs.len();
    let resources: Vec<Value> = docs
        .into_iter()
        .skip(skip)
        .take(PAGE)
        .filter_map(|d| {
            let v = d.current()?;
            let mut r = serde_json::Map::new();
            r.insert("uri".into(), json!(uri(d)));
            r.insert("name".into(), json!(d.title));
            r.insert("title".into(), json!(d.title));
            r.insert("mimeType".into(), json!(v.file.mime));
            r.insert("size".into(), json!(v.file.size));
            if !d.description.is_empty() {
                r.insert("description".into(), json!(d.description));
            }
            Some(Value::Object(r))
        })
        .collect();
    let next = skip.saturating_add(PAGE);
    let mut result = serde_json::Map::new();
    result.insert("resources".into(), Value::Array(resources));
    if next < total {
        result.insert("nextCursor".into(), json!(next.to_string()));
    }
    Ok(Value::Object(result))
}

/// The current version of one document. A document the person can't open is "not found", the same as one
/// that doesn't exist.
pub fn read(tenant: &Tenant, actor: Actor, params: &Value) -> Result<Value, Failure> {
    let wanted = params.get("uri").and_then(Value::as_str).ok_or((-32602, "resources/read needs a `uri`".to_string()))?;
    let not_found = || (-32002, format!("resource `{wanted}` not found"));
    let (project, id) = parse(wanted).ok_or_else(not_found)?;
    let state = tenant.engine.state();
    let doc = state
        .documents
        .get(Id::new(id))
        .filter(|d| d.project == Id::new(project) && d.deleted.is_none() && view::can_see_document(state, actor, d))
        .ok_or_else(not_found)?;
    let file = &doc.current().ok_or_else(not_found)?.file;
    if file.size > MAX_READ {
        return Err((-32602, format!("`{}` is {} bytes; resources over {MAX_READ} bytes can't be read here", file.name, file.size)));
    }
    let unreadable = || (-32603, "the file could not be read".to_string());
    let mut bytes = Vec::new();
    let opened = tenant.store.blobs.open(&file.sha256).map_err(|_| unreadable())?;
    opened.take(MAX_READ + 1).read_to_end(&mut bytes).map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_READ {
        return Err((-32602, format!("resources over {MAX_READ} bytes can't be read here")));
    }
    let content = match (is_text(&file.mime), String::from_utf8(bytes)) {
        (true, Ok(text)) => json!({ "uri": wanted, "mimeType": file.mime, "text": text }),
        (_, Ok(text)) => json!({ "uri": wanted, "mimeType": file.mime, "blob": STANDARD.encode(text.as_bytes()) }),
        (_, Err(e)) => json!({ "uri": wanted, "mimeType": file.mime, "blob": STANDARD.encode(e.as_bytes()) }),
    };
    Ok(json!({ "contents": [content] }))
}
