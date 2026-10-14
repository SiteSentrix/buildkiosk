//! Finding what a language catalog has to cover: `buildkiosk i18n-harvest`.
//!
//! The pages' wording is the text the source code supplies, as opposed to the names and notes people
//! type. Two things are put together to tell them apart: the pieces of text (and runs of text and live
//! values, such as `"Paid " (date)`) found in the source files, and the text actually on the pages of a
//! demo company, visited like a person would, through every link. A piece of text on a page that the
//! source also supplies is wording, and goes in the list.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use buildkiosk_store::DataDir;
use buildkiosk_web::i18n;
use tower::ServiceExt;

use crate::{App, router};

// ---- reading the source ---------------------------------------------------------------------

enum Tok {
    Str(String),
    Group(char, Vec<Tok>),
    Other,
}

struct Lexer {
    c: Vec<char>,
    i: usize,
}

impl Lexer {
    fn peek(&self, k: usize) -> Option<char> {
        self.c.get(self.i + k).copied()
    }

    fn string(&mut self) -> String {
        // Past the opening quote.
        let mut out = String::new();
        while let Some(ch) = self.peek(0) {
            self.i += 1;
            match ch {
                '"' => break,
                '\\' => {
                    let Some(e) = self.peek(0) else { break };
                    self.i += 1;
                    match e {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        '0' => out.push('\0'),
                        '\\' | '"' | '\'' => out.push(e),
                        'u' => {
                            let mut hex = String::new();
                            if self.peek(0) == Some('{') {
                                self.i += 1;
                                while let Some(h) = self.peek(0) {
                                    self.i += 1;
                                    if h == '}' {
                                        break;
                                    }
                                    hex.push(h);
                                }
                            }
                            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                out.push(c);
                            }
                        }
                        'x' => {
                            let hex: String = (0..2).filter_map(|k| self.peek(k)).collect();
                            self.i += hex.len();
                            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                                out.push(c);
                            }
                        }
                        '\n' => {
                            while self.peek(0).is_some_and(char::is_whitespace) {
                                self.i += 1;
                            }
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        out
    }

    fn raw_string(&mut self) -> String {
        // At the `#`s or the quote after the `r`.
        let mut hashes = 0;
        while self.peek(0) == Some('#') {
            hashes += 1;
            self.i += 1;
        }
        if self.peek(0) != Some('"') {
            return String::new();
        }
        self.i += 1;
        let mut out = String::new();
        while let Some(ch) = self.peek(0) {
            if ch == '"' && (1..=hashes).all(|k| self.peek(k) == Some('#')) {
                self.i += 1 + hashes;
                break;
            }
            out.push(ch);
            self.i += 1;
        }
        out
    }

    fn seq(&mut self, close: Option<char>) -> Vec<Tok> {
        let mut out = Vec::new();
        // The last two words, to know a `mod tests { … }` (whose text is for the tests, not for people).
        let mut words: (String, String) = (String::new(), String::new());
        while let Some(ch) = self.peek(0) {
            if ch.is_whitespace() {
                self.i += 1;
            } else if ch == '/' && self.peek(1) == Some('/') {
                while self.peek(0).is_some_and(|c| c != '\n') {
                    self.i += 1;
                }
            } else if ch == '/' && self.peek(1) == Some('*') {
                let mut depth = 0;
                while let Some(c) = self.peek(0) {
                    if c == '/' && self.peek(1) == Some('*') {
                        depth += 1;
                        self.i += 2;
                    } else if c == '*' && self.peek(1) == Some('/') {
                        depth -= 1;
                        self.i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        self.i += 1;
                    }
                }
            } else if ch == '"' {
                self.i += 1;
                let s = self.string();
                out.push(Tok::Str(s));
            } else if ch == '\'' {
                // A character like 'x' or '\n', or a lifetime like 'a.
                if self.peek(1) == Some('\\') {
                    self.i += 2;
                    while self.peek(0).is_some_and(|c| c != '\'') {
                        self.i += 1;
                    }
                    self.i += 1;
                } else if self.peek(2) == Some('\'') {
                    self.i += 3;
                } else {
                    self.i += 1;
                    while self.peek(0).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        self.i += 1;
                    }
                }
                out.push(Tok::Other);
            } else if matches!(ch, '(' | '[' | '{') {
                self.i += 1;
                let end = match ch {
                    '(' => ')',
                    '[' => ']',
                    _ => '}',
                };
                let inner = self.seq(Some(end));
                if !(ch == '{' && words.0 == "mod" && words.1 == "tests") {
                    out.push(Tok::Group(ch, inner));
                }
            } else if Some(ch) == close {
                self.i += 1;
                return out;
            } else if ch.is_alphabetic() || ch == '_' {
                let start = self.i;
                while self.peek(0).is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    self.i += 1;
                }
                let ident: String = self.c.get(start..self.i).unwrap_or(&[]).iter().collect();
                words = (std::mem::take(&mut words.1), ident.clone());
                match (ident.as_str(), self.peek(0)) {
                    ("r" | "br", Some('"' | '#')) => {
                        let s = self.raw_string();
                        out.push(Tok::Str(s));
                    }
                    ("b", Some('"')) => {
                        self.i += 1;
                        let _ = self.string();
                        out.push(Tok::Other);
                    }
                    _ => out.push(Tok::Other),
                }
            } else {
                self.i += 1;
                out.push(Tok::Other);
            }
        }
        out
    }
}

/// `{}`, `{name}`, `{:>5}` in a format string are values; `{{` and `}}` are braces.
fn format_values(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '}' if chars.peek() == Some(&'}') => {
                chars.next();
                out.push('}');
            }
            '{' => {
                for n in chars.by_ref() {
                    if n == '}' {
                        break;
                    }
                }
                out.push('\u{1}');
            }
            other => out.push(other),
        }
    }
    out.replace('\u{1}', "{}")
}

fn walk(toks: &[Tok], out: &mut HashSet<String>) {
    let mut run: Vec<&Tok> = Vec::new();
    let flush = |run: &mut Vec<&Tok>, out: &mut HashSet<String>| {
        if run.iter().any(|t| matches!(t, Tok::Str(_))) {
            for t in run.iter() {
                if let Tok::Str(s) = t {
                    out.insert(format_values(s));
                }
            }
            if run.len() > 1 {
                let joined: String = run
                    .iter()
                    .map(|t| match t {
                        Tok::Str(s) => format_values(s),
                        _ => "{}".to_string(),
                    })
                    .collect();
                out.insert(joined);
            }
        }
        run.clear();
    };
    for t in toks {
        match t {
            Tok::Str(_) | Tok::Group('(', _) => run.push(t),
            _ => flush(&mut run, out),
        }
        if let Tok::Group(_, inner) = t {
            walk(inner, out);
        }
    }
    flush(&mut run, out);
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a pattern with values is specific enough to be wording: fixed text of its own at an end, or plenty of it.
fn specific(pattern: &str) -> bool {
    let parts: Vec<&str> = pattern.split("{}").collect();
    let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
    let run = |s: &str| {
        let (mut best, mut now) = (0, 0);
        for c in s.chars() {
            now = if c.is_alphabetic() { now + 1 } else { 0 };
            best = best.max(now);
        }
        best
    };
    let ends = parts.first().is_some_and(|p| run(p) >= 3) || parts.last().is_some_and(|p| run(p) >= 3);
    ends || parts.iter().map(|p| letters(p)).sum::<usize>() >= 8
}

/// The pieces of text, and runs of text with values, the source files supply.
/// Returns every piece of text and run of text with values, and those from the page-building and model
/// sources that read like wording a person would be shown (so pages the demo company can't reach are covered).
fn source_wording(root: &Path) -> Result<(HashSet<String>, HashSet<String>), String> {
    let mut found = HashSet::new();
    let mut ui = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))?.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if !matches!(name.as_str(), "target" | "tests" | ".git" | "assets") {
                    stack.push(path);
                }
            } else if name.ends_with(".rs") && !matches!(name.as_str(), "tests.rs") {
                let text = std::fs::read_to_string(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                let mut lexer = Lexer { c: text.chars().collect(), i: 0 };
                let toks = lexer.seq(None);
                let mut here = HashSet::new();
                walk(&toks, &mut here);
                let shown = path.components().any(|c| c.as_os_str() == "web") || path.components().any(|c| c.as_os_str() == "model");
                if shown {
                    ui.extend(here.iter().filter(|s| reads_like_wording(s)).cloned());
                }
                found.extend(here);
            }
        }
    }
    Ok((tidy(found), tidy(ui)))
}

/// The pieces of wording in a source text, as the catalogs key them: cut at the separators, single spaces, no
/// dangling separator, and a label that ends in a colon also without it (the page cuts there when a value follows).
fn pieces_of(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (piece, _) in i18n::split_pieces(s) {
        let p = squash(piece);
        let p = p.trim_end_matches('·').trim().to_string();
        if !p.chars().any(char::is_alphabetic)
            || p.len() > 1500
            || is_drawing_path(&p)
            || p.contains(",,")
            || p == "BuildKiosk"
            || i18n::localize_dates(&p, i18n::Lang::Es) != p
        {
            continue;
        }
        if let Some(bare) = p.strip_suffix(':').map(str::trim).filter(|b| b.chars().any(char::is_alphabetic)) {
            out.push(bare.to_string());
        }
        out.push(p);
    }
    out
}

fn tidy(set: HashSet<String>) -> HashSet<String> {
    // Text laid out over several lines (a sample to type in) keeps its layout, so it is not translated.
    set.iter().filter(|s| !s.contains('\n') && !s.trim_start().starts_with('#')).flat_map(|s| pieces_of(s)).collect()
}

/// Icon outlines (`M12 16a4 4 0 1 0 0-8…`) are text in the page but not words.
fn is_drawing_path(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit() || " .,-".contains(c) || "MmLlHhVvCcSsQqTtAaZz".contains(c))
}

/// Whether a piece of source text looks like words a person is shown (a capital first, no code or address characters).
fn reads_like_wording(s: &str) -> bool {
    let s = squash(s);
    let mut chars = s.chars();
    let first = chars.next();
    first.is_some_and(char::is_uppercase)
        && s.chars().filter(|c| c.is_alphabetic()).count() >= 3
        && s.len() <= 1500
        && !s.contains(['/', '=', '_', '<', '>', ';', '\\', '|', '[', ']', '$', '@', '%', '\u{1}'])
        && !s.contains("{}{}")
        && !s.starts_with("Content")
        && s.split(' ').all(|w| w.len() <= 40)
}

// ---- visiting the pages ---------------------------------------------------------------------

struct Visitor {
    app: Arc<App>,
    cookies: HashMap<String, String>,
}

impl Visitor {
    async fn send(&mut self, req: Request<Body>) -> Result<(StatusCode, axum::http::HeaderMap, String), String> {
        let res = router(self.app.clone()).oneshot(req).await.map_err(|e| format!("{e}"))?;
        let (parts, body) = res.into_parts();
        for v in parts.headers.get_all("set-cookie") {
            if let Some((k, val)) = v.to_str().ok().and_then(|c| c.split(';').next()).and_then(|c| c.split_once('=')) {
                self.cookies.insert(k.to_string(), val.to_string());
            }
        }
        let bytes = axum::body::to_bytes(body, 32 * 1024 * 1024).await.map_err(|e| e.to_string())?;
        Ok((parts.status, parts.headers, String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn cookie_header(&self) -> String {
        self.cookies.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("; ")
    }

    async fn get(&mut self, uri: &str) -> Result<(StatusCode, axum::http::HeaderMap, String), String> {
        let req = Request::get(uri).header("cookie", self.cookie_header()).body(Body::empty()).map_err(|e| e.to_string())?;
        self.send(req).await
    }
}

fn csrf_of(html: &str) -> Option<String> {
    html.split("name=\"_csrf\" value=\"").nth(1).and_then(|r| r.split('"').next()).map(str::to_string)
}

fn form_encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { char::from(b).to_string() } else { format!("%{b:02X}") }).collect()
}

/// The addresses a page links to on this site.
fn links(html: &str) -> Vec<String> {
    html.split("href=\"")
        .skip(1)
        .filter_map(|r| r.split('"').next())
        .map(|h| h.replace("&amp;", "&"))
        .filter(|h| h.starts_with('/') && !h.starts_with("//"))
        .collect()
}

/// Which pages are alike (the same address with other numbers) so a few of each are enough.
fn shape(uri: &str) -> String {
    let (path, query) = uri.split_once('?').unwrap_or((uri, ""));
    let path: String =
        path.split('/').map(|s| if s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() { "N" } else { s }).collect::<Vec<_>>().join("/");
    let mut keys: Vec<&str> = query.split('&').filter_map(|kv| kv.split('=').next()).filter(|k| !k.is_empty()).collect();
    keys.sort_unstable();
    format!("{path}?{}", keys.join("&"))
}

fn skip(uri: &str) -> bool {
    let path = uri.split(['?', '#']).next().unwrap_or("");
    path.starts_with("/static")
        || path.starts_with("/logout")
        || path.starts_with("/language")
        || path.starts_with("/do/")
        || path.starts_with("/mcp")
        || path.starts_with("/api")
        || path.contains("/files/")
        || path.contains("/download")
        || path.contains("/photos/file")
        || path.contains("/thumb")
        || [".pdf", ".csv", ".zip", ".xml", ".png", ".jpg", ".svg", ".ics", ".js", ".wasm", ".css", ".xlsx", ".json", ".ifc"]
            .iter()
            .any(|e| path.ends_with(e))
}

/// Signs in to the demo company and visits every page reachable by links; the HTML of each.
async fn crawl(app: Arc<App>, email: &str, password: &str, per_shape: usize, limit: usize) -> Result<Vec<(String, String)>, String> {
    let mut v = Visitor { app, cookies: HashMap::new() };
    let mut pages: Vec<(String, String)> = Vec::new();
    // The sign-in page, as a visitor sees it.
    let (_, _, login) = v.get("/login").await?;
    let token = csrf_of(&login).ok_or("the sign-in page has no form token")?;
    pages.push(("/login".to_string(), login));
    let body = format!("_csrf={}&email={}&password={}", form_encode(&token), form_encode(email), form_encode(password));
    let req = Request::post("/login")
        .header("cookie", v.cookie_header())
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::from(body))
        .map_err(|e| e.to_string())?;
    let (status, ..) = v.send(req).await?;
    if status != StatusCode::SEE_OTHER {
        return Err("the demo user could not sign in".into());
    }
    let mut queue: VecDeque<String> = VecDeque::from(["/".to_string()]);
    let mut seen: HashSet<String> = HashSet::new();
    let mut per: HashMap<String, usize> = HashMap::new();
    while let Some(uri) = queue.pop_front() {
        if pages.len() >= limit {
            break;
        }
        if skip(&uri) || !seen.insert(uri.clone()) {
            continue;
        }
        let count = per.entry(shape(&uri)).or_default();
        if *count >= per_shape {
            continue;
        }
        *count += 1;
        let (status, headers, body) = v.get(&uri).await?;
        let is_html = headers.get("content-type").and_then(|c| c.to_str().ok()).is_some_and(|c| c.starts_with("text/html"));
        if !is_html || status.is_server_error() {
            continue;
        }
        queue.extend(links(&body));
        pages.push((uri, body));
    }
    Ok(pages)
}

// ---- the list -------------------------------------------------------------------------------

/// Visits the demo company in `data` and writes the wording list to `out`. Returns a summary line.
pub fn run(data: &Path, sources: &Path, out: &Path, email: &str, password: &str) -> Result<String, String> {
    let app = Arc::new(App::open(DataDir::open(data).map_err(|e| e.to_string())?, None)?);
    let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().map_err(|e| e.to_string())?;
    let pages = runtime.block_on(crawl(app, email, password, 4, 6000))?;

    let (source, ui) = source_wording(sources)?;
    let exact: HashSet<&str> = source.iter().filter(|s| !s.contains("{}")).map(String::as_str).collect();
    let mut patterns: Vec<(Vec<String>, &str)> = source
        .iter()
        .filter(|s| s.contains("{}") && specific(s))
        .map(|s| (s.split("{}").map(str::to_string).collect::<Vec<_>>(), s.as_str()))
        .collect();
    patterns.sort_by_key(|(parts, _)| std::cmp::Reverse(parts.iter().map(String::len).sum::<usize>()));

    let labels: Vec<String> = buildkiosk_engine::all_commands().map(|c| c.label.to_string()).collect::<BTreeSet<_>>().into_iter().rev().collect();
    let mut keys: BTreeSet<String> = BTreeSet::new();
    let mut seen_pieces: HashSet<String> = HashSet::new();
    let mut dynamic = 0usize;
    if let Ok(log) = std::env::var("BUILDKIOSK_HARVEST_LOG") {
        let _ = std::fs::write(log, pages.iter().map(|(u, _)| format!("{u}\n")).collect::<String>());
    }
    for (_, page) in &pages {
        for piece in i18n::page_wording(page) {
            if !seen_pieces.insert(piece.clone()) {
                continue;
            }
            if exact.contains(piece.as_str()) {
                keys.insert(piece);
            } else if let Some((_, pattern)) = patterns.iter().find(|(parts, _)| i18n::capture(parts, &piece).is_some()) {
                keys.insert((*pattern).to_string());
            } else if let Some(label) = labels.iter().find(|l| piece.starts_with(&format!("{l} by "))) {
                // A history line: what was done (a command's name) and who did it.
                keys.insert(format!("{label} by {{}}"));
            } else if let Some(prefix) =
                piece.rsplit_once(" #").filter(|(p, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && exact.contains(*p)).map(|(p, _)| p)
            {
                // A numbered record: a kind of record from the source and its number.
                keys.insert(format!("{prefix} #{{}}"));
            } else {
                dynamic += 1;
                if let Ok(log) = std::env::var("BUILDKIOSK_HARVEST_OTHER") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(log) {
                        let _ = writeln!(f, "{piece}");
                    }
                }
            }
        }
    }
    // Every change shows in a history as what was done and who did it.
    for c in buildkiosk_engine::all_commands().filter(|c| !c.is_query()) {
        keys.insert(format!("{} by {{}}", c.label));
    }
    let from_pages = keys.len();
    for w in ui.iter().filter(|w| !w.contains("{}") || specific(w)) {
        keys.insert(w.clone());
    }
    let mut text =
        String::from("# Every piece of wording the language catalogs must translate. Made by `buildkiosk i18n-harvest`; do not edit by hand.\n");
    for k in &keys {
        text.push_str(k);
        text.push('\n');
    }
    std::fs::write(out, text).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(format!(
        "{} pages visited; {} pieces of wording on them ({} of those are people's own text); {} entries from the pages, {} in all (adding wording the demo can't reach), written to {}",
        pages.len(),
        seen_pieces.len(),
        dynamic,
        from_pages,
        keys.len(),
        out.display()
    ))
}
