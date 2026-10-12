//! `cargo xtask brands`: no other vendor's product or company names in what users and agents read.
//!
//! BuildKiosk names its own features; where a comparison is needed, text says "the reference
//! product". Scanned: string literals in Rust sources under `crates/` and `apps/` (page text,
//! command labels and params docs, MCP descriptions), skipping comments and test code; the
//! `description` of every `Cargo.toml`; text files under `packaging/` and non-Markdown files under
//! `docs/`. Markdown docs (README, ROADMAP, docs/*.md) may name the reference product.
//!
//! A line that must keep a name (an integration that has to say what it connects to) says
//! `brand-ok` in a comment on that line.

use std::path::Path;

use crate::scan;

/// Vendor product and company names, lowercase, matched case-insensitively as whole words.
const DENY: &[&str] = &[
    "procore",
    "autodesk",
    "plangrid",
    "bluebeam",
    "buildertrend",
    "aconex",
    "primavera",
    "textura",
    "fieldwire",
    "revit",
    "navisworks",
    "bim 360",
    "newforma",
    "raken",
    // The predecessor app's demo used a real company's name; never again.
    "bechtel",
];

const ALLOW: &str = "brand-ok";

/// This file lists the names it rejects.
const SELF: &str = "xtask/src/brands.rs";

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let mut hits = Vec::new();
    for path in &files {
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        let allowed = |line: usize| lines.get(line.saturating_sub(1)).is_some_and(|l| l.contains(ALLOW));
        if is_rust_to_scan(path) {
            for lit in scan::string_literals(&text) {
                if allowed(lit.line) {
                    continue;
                }
                for term in scan::terms_in(&lit.text, DENY) {
                    hits.push(format!("{path}:{}: \"{term}\" in \"{}\"", lit.line, short(&lit.text)));
                }
            }
        } else if path == "Cargo.toml" || path.ends_with("/Cargo.toml") {
            for (n, l) in lines.iter().enumerate() {
                if l.trim_start().starts_with("description") && !l.contains(ALLOW) {
                    for term in scan::terms_in(l, DENY) {
                        hits.push(format!("{path}:{}: \"{term}\" in the package description", n + 1));
                    }
                }
            }
        } else if path.starts_with("packaging/") || (path.starts_with("docs/") && !path.ends_with(".md")) {
            for (n, l) in lines.iter().enumerate() {
                if !l.contains(ALLOW) {
                    for term in scan::terms_in(l, DENY) {
                        hits.push(format!("{path}:{}: \"{term}\"", n + 1));
                    }
                }
            }
        }
    }
    if hits.is_empty() {
        println!("brands: no vendor names in user-visible text");
        Ok(())
    } else {
        Err(format!(
            "{} vendor name(s) in user-visible text (say \"the reference product\" or name the feature; mark a required integration name `{ALLOW}`):\n  {}",
            hits.len(),
            hits.join("\n  ")
        ))
    }
}

fn is_rust_to_scan(path: &str) -> bool {
    if !path.ends_with(".rs") || path == SELF || !(path.starts_with("crates/") || path.starts_with("apps/")) {
        return false;
    }
    let mut parts = path.split('/').rev();
    let file = parts.next().unwrap_or_default();
    let test_file = file == "tests.rs" || file.starts_with("tests_") || file.ends_with("_tests.rs");
    let test_dir = parts.any(|d| d == "tests" || d == "benches" || d == "testkit");
    !test_file && !test_dir
}

fn short(s: &str) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() > 80 { format!("{}…", one_line.chars().take(80).collect::<String>()) } else { one_line }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_code_and_xtask_are_not_scanned() {
        assert!(is_rust_to_scan("crates/web/src/pages/rfi.rs"));
        assert!(!is_rust_to_scan("crates/engine/tests/rfi.rs"));
        assert!(!is_rust_to_scan("crates/testkit/src/lib.rs"));
        assert!(!is_rust_to_scan("xtask/src/brands.rs"));
        assert!(!is_rust_to_scan("crates/web/src/style.css"));
    }
}
