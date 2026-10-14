//! Documents, the current revision of drawings, and photos as MCP resources.

use std::io::Read;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use buildkiosk_engine::Actor;
use buildkiosk_engine::model::common::FileRef;
use buildkiosk_engine::model::{Level, Tool};
use buildkiosk_engine::view;
use buildkiosk_store::Tenant;
use serde_json::{Value, json};

/// Resources returned per page of `resources/list`.
const PAGE: usize = 100;
/// The largest file `resources/read` returns (it goes out as one JSON message).
const MAX_READ: u64 = 8 * 1024 * 1024;

/// What kind of thing a resource is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Document,
    Drawing,
    Photo,
}

impl Kind {
    const ALL: [Kind; 3] = [Kind::Document, Kind::Drawing, Kind::Photo];

    fn word(self) -> &'static str {
        match self {
            Kind::Document => "document",
            Kind::Drawing => "drawing",
            Kind::Photo => "photo",
        }
    }

    fn prefix(self) -> String {
        format!("buildkiosk://{}/", self.word())
    }
}

type Failure = (i64, String);

/// A resource the person may open: its address, names and the file behind it.
struct Entry {
    uri: String,
    name: String,
    description: String,
    file: FileRef,
}

fn uri(kind: Kind, project: impl std::fmt::Display, id: impl std::fmt::Display) -> String {
    format!("{}{project}/{id}", kind.prefix())
}

/// `buildkiosk://document/{project}/{id}` → (kind, project, id).
fn parse(uri: &str) -> Option<(Kind, u64, u64)> {
    Kind::ALL.into_iter().find_map(|k| {
        let (project, id) = uri.strip_prefix(&k.prefix())?.split_once('/')?;
        Some((k, project.parse().ok()?, id.parse().ok()?))
    })
}

/// Everything the person may open, kind by kind and by id within each.
fn entries(tenant: &Tenant, actor: Actor) -> Vec<Entry> {
    let state = tenant.engine.state();
    let mut out: Vec<Entry> = Vec::new();
    let mut docs: Vec<_> = state.documents.iter().filter(|d| d.deleted.is_none() && view::can_see_document(state, actor, d)).collect();
    docs.sort_by_key(|d| d.id);
    out.extend(docs.into_iter().filter_map(|d| {
        Some(Entry {
            uri: uri(Kind::Document, d.project, d.id),
            name: d.title.clone(),
            description: d.description.clone(),
            file: d.current()?.file.clone(),
        })
    }));
    let mut sheets: Vec<_> =
        state.drawings.iter().filter(|d| d.deleted.is_none() && view::level(state, actor, d.project, Tool::Drawings) >= Level::Read).collect();
    sheets.sort_by_key(|d| d.id);
    out.extend(sheets.into_iter().filter_map(|d| {
        let rev = d.revisions.last()?;
        Some(Entry {
            uri: uri(Kind::Drawing, d.project, d.id),
            name: format!("{} {}", d.number, d.title),
            description: format!("Revision {}", rev.label),
            file: rev.file.clone()?,
        })
    }));
    let mut shots: Vec<_> =
        state.photos.iter().filter(|p| p.deleted.is_none() && view::level(state, actor, p.project, Tool::Photos) >= Level::Read).collect();
    shots.sort_by_key(|p| p.id);
    out.extend(shots.into_iter().map(|p| Entry {
        uri: uri(Kind::Photo, p.project, p.id),
        name: if p.caption.is_empty() { p.file.name.clone() } else { p.caption.clone() },
        description: p.album.clone(),
        file: p.file.clone(),
    }));
    out
}

/// What a resource is at the moment: the hash of its file, or `None` when the person can't open it
/// (it doesn't exist, was deleted or is hidden from them). A change in this is a change worth telling.
pub fn fingerprint(tenant: &Tenant, actor: Actor, wanted: &str) -> Option<String> {
    let (kind, project, id) = parse(wanted)?;
    let state = tenant.engine.state();
    let project = buildkiosk_core::Id::new(project);
    match kind {
        Kind::Document => {
            let d = state.documents.get(buildkiosk_core::Id::new(id)).filter(|d| d.project == project && d.deleted.is_none())?;
            view::can_see_document(state, actor, d).then(|| d.current().map(|v| v.file.sha256.clone()))?
        }
        Kind::Drawing => {
            let d = state.drawings.get(buildkiosk_core::Id::new(id)).filter(|d| d.project == project && d.deleted.is_none())?;
            if view::level(state, actor, d.project, Tool::Drawings) < Level::Read {
                return None;
            }
            d.revisions.last()?.file.as_ref().map(|f| f.sha256.clone())
        }
        Kind::Photo => {
            let p = state.photos.get(buildkiosk_core::Id::new(id)).filter(|p| p.project == project && p.deleted.is_none())?;
            (view::level(state, actor, p.project, Tool::Photos) >= Level::Read).then(|| p.file.sha256.clone())
        }
    }
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
    let about = |k: Kind| match k {
        Kind::Document => ("Project document", "The current version of a document in a project's Documents tool."),
        Kind::Drawing => ("Drawing sheet", "The file of a drawing's current revision."),
        Kind::Photo => ("Project photo", "A picture in a project's Photos tool."),
    };
    let list: Vec<Value> = Kind::ALL
        .into_iter()
        .map(|k| json!({ "uriTemplate": format!("{}{{project}}/{{id}}", k.prefix()), "name": k.word(), "title": about(k).0, "description": about(k).1 }))
        .collect();
    json!({ "resourceTemplates": list })
}

/// The documents, drawing sheets and photos the person may open, a page at a time (`cursor` is how many to skip).
pub fn list(tenant: &Tenant, actor: Actor, params: &Value) -> Result<Value, Failure> {
    let skip = match params.get("cursor") {
        None | Some(Value::Null) => 0,
        Some(Value::String(s)) => s.parse::<usize>().map_err(|_| (-32602, "`cursor` is not one this server gave".to_string()))?,
        Some(_) => return Err((-32602, "`cursor` must be text".to_string())),
    };
    let all = entries(tenant, actor);
    let total = all.len();
    let resources: Vec<Value> = all
        .into_iter()
        .skip(skip)
        .take(PAGE)
        .map(|e| {
            let mut r = serde_json::Map::new();
            r.insert("uri".into(), json!(e.uri));
            r.insert("name".into(), json!(e.name));
            r.insert("title".into(), json!(e.name));
            r.insert("mimeType".into(), json!(e.file.mime));
            r.insert("size".into(), json!(e.file.size));
            if !e.description.is_empty() {
                r.insert("description".into(), json!(e.description));
            }
            Value::Object(r)
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
    let (kind, project, id) = parse(wanted).ok_or_else(not_found)?;
    let file = entries(tenant, actor).into_iter().find(|e| parse(&e.uri) == Some((kind, project, id))).map(|e| e.file).ok_or_else(not_found)?;
    let file = &file;
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
