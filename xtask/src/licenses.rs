//! `cargo xtask licenses`: every dependency must be usable in an MIT OR Apache-2.0 project, so no
//! GPL, LGPL or AGPL (unless the crate also offers a permissive licence).

use crate::cargo;

/// Licences we accept (SPDX ids).
const ALLOWED: &[&str] = &[
    "MIT",
    "MIT-0",
    "Apache-2.0",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "0BSD",
    "Unicode-3.0",
    "Unicode-DFS-2016",
    "CC0-1.0",
    "Unlicense",
    "BSL-1.0",
    "MPL-2.0",
    "CDLA-Permissive-2.0",
];

pub fn run() -> Result<(), String> {
    let out = cargo().args(["metadata", "--format-version", "1"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))?;
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages")?;
    let mut bad = Vec::new();
    let mut checked = 0usize;
    for p in pkgs {
        if p["source"].is_null() {
            continue; // a workspace member
        }
        checked += 1;
        let name = p["name"].as_str().unwrap_or("?");
        let version = p["version"].as_str().unwrap_or("?");
        match p["license"].as_str() {
            Some(expr) if allowed(expr) => {}
            Some(expr) => bad.push(format!("{name} {version}: `{expr}`")),
            None => bad.push(format!("{name} {version}: no SPDX licence expression (license-file only); review it by hand")),
        }
    }
    if bad.is_empty() {
        println!("licenses: all {checked} dependencies have a permissive licence");
        Ok(())
    } else {
        Err(format!("{} dependenc(ies) without an accepted licence:\n  {}", bad.len(), bad.join("\n  ")))
    }
}

/// Whether an SPDX expression offers an accepted licence: some `OR` branch whose `AND` parts are
/// all accepted. `WITH` exceptions are ignored; old-style `MIT/Apache-2.0` counts as `OR`.
pub fn allowed(expr: &str) -> bool {
    let normalized = expr.replace('/', " OR ").replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = normalized.split_whitespace().collect();
    let mut pos = 0usize;
    parse_or(&tokens, &mut pos).unwrap_or(false) && pos == tokens.len()
}

fn parse_or(t: &[&str], pos: &mut usize) -> Option<bool> {
    let mut ok = parse_and(t, pos)?;
    while t.get(*pos) == Some(&"OR") {
        *pos += 1;
        ok |= parse_and(t, pos)?;
    }
    Some(ok)
}

fn parse_and(t: &[&str], pos: &mut usize) -> Option<bool> {
    let mut ok = parse_atom(t, pos)?;
    while t.get(*pos) == Some(&"AND") {
        *pos += 1;
        ok &= parse_atom(t, pos)?;
    }
    Some(ok)
}

fn parse_atom(t: &[&str], pos: &mut usize) -> Option<bool> {
    let tok = *t.get(*pos)?;
    *pos += 1;
    let ok = if tok == "(" {
        let inner = parse_or(t, pos)?;
        if t.get(*pos) != Some(&")") {
            return None;
        }
        *pos += 1;
        inner
    } else {
        ALLOWED.contains(&tok.trim_end_matches('+'))
    };
    if t.get(*pos) == Some(&"WITH") {
        *pos += 2;
    }
    Some(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spdx_expressions() {
        assert!(allowed("MIT OR Apache-2.0"));
        assert!(allowed("MIT/Apache-2.0"));
        assert!(allowed("Apache-2.0 WITH LLVM-exception OR MIT"));
        assert!(allowed("(MIT OR Apache-2.0) AND Unicode-3.0"));
        assert!(allowed("GPL-3.0 OR MIT"));
        assert!(!allowed("GPL-3.0"));
        assert!(!allowed("LGPL-2.1-or-later"));
        assert!(!allowed("MIT AND GPL-2.0"));
        assert!(!allowed("AGPL-3.0-only"));
        assert!(!allowed("(MIT"));
    }
}
