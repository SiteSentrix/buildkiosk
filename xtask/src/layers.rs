//! Dependency layering rules (docs/architecture.md › Layers).
//!
//! The rule engine works on a small model so it can be unit-tested; `from_metadata` builds the
//! model from `cargo metadata`.

use serde_json::Value;

/// Where a workspace crate sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    /// A layered crate: may depend only on strictly lower layers (or same-layer crates listed in
    /// [`INTRA_LAYER_ORDER`]).
    Layer(u8),
    /// Layer 0 and depends on no workspace crate at all.
    Standalone,
    /// Test helpers: may depend on anything up to layer 5; other crates may use them only as a
    /// dev-dependency.
    Testkit,
    /// Binaries and build tooling: exempt.
    Exempt,
}

impl Class {
    fn layer(self) -> Option<u8> {
        match self {
            Class::Layer(l) => Some(l),
            Class::Standalone => Some(0),
            Class::Testkit => Some(6),
            Class::Exempt => None,
        }
    }
}

/// The layering table. Names are package names without the `buildkiosk-` prefix. Append new
/// crates; changing a crate's layer is an architecture decision (docs/architecture.md).
pub const TABLE: &[(&str, Class)] = &[
    ("core", Class::Standalone),
    ("model", Class::Layer(1)),
    ("finance", Class::Layer(2)),
    ("schedule", Class::Layer(2)),
    ("engine", Class::Layer(5)),
    ("testkit", Class::Testkit),
    ("store", Class::Layer(6)),
    ("web", Class::Layer(6)),
    ("mcp", Class::Layer(6)),
    ("server", Class::Layer(6)),
    // The browser half of the sheet viewer (WebAssembly); built by `cargo xtask viewer`.
    ("viewer", Class::Layer(6)),
    // apps and tooling
    ("buildkiosk", Class::Exempt),
    ("xtask", Class::Exempt),
];

/// Allowed orderings within a layer (earlier may be used by later).
pub const INTRA_LAYER_ORDER: &[&[&str]] = &[&["store", "server"], &["web", "server"], &["mcp", "server"], &["store", "mcp"]];

/// External crates of the web stack and network I/O, allowed only from layer 6 up. Entries ending
/// in `*` are prefixes.
pub const WEB_STACK: &[&str] = &["axum*", "tokio*", "hyper*", "tower*", "maud*", "http-body*", "multer", "reqwest", "mio", "socket2"];

/// First layer allowed to use the web stack.
pub const WEB_MIN_LAYER: u8 = 6;

pub fn short_name(pkg: &str) -> &str {
    pkg.strip_prefix("buildkiosk-").unwrap_or(pkg)
}

pub fn classify(pkg: &str) -> Option<Class> {
    let s = short_name(pkg);
    TABLE.iter().find(|(n, _)| *n == s).map(|(_, c)| *c)
}

fn intra_layer_allowed(from: &str, to: &str) -> bool {
    let (from, to) = (short_name(from), short_name(to));
    INTRA_LAYER_ORDER.iter().any(|chain| match (chain.iter().position(|n| *n == from), chain.iter().position(|n| *n == to)) {
        (Some(f), Some(t)) => t < f,
        _ => false,
    })
}

fn is_web_stack(name: &str) -> bool {
    WEB_STACK.iter().any(|p| match p.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => name == *p,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepKind {
    Normal,
    Dev,
    Build,
}

#[derive(Debug, Clone)]
pub struct Dep {
    pub name: String,
    pub kind: DepKind,
    /// The dependency is a workspace member.
    pub workspace: bool,
}

#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub deps: Vec<Dep>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    Unregistered { krate: String },
    Upward { krate: String, dep: String, from: u8, to: u8, kind: DepKind },
    StandaloneHasWorkspaceDep { krate: String, dep: String },
    TestkitAsNormalDep { krate: String },
    WebStackBelowL6 { krate: String, dep: String, layer: u8 },
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Violation::Unregistered { krate } => write!(f, "{krate}: unknown workspace crate; register it in xtask/src/layers.rs TABLE"),
            Violation::Upward { krate, dep, from, to, kind } => {
                write!(f, "{krate} (L{from}) -> {dep} (L{to}) [{kind:?}]: may only depend on strictly lower layers")
            }
            Violation::StandaloneHasWorkspaceDep { krate, dep } => write!(f, "{krate}: a standalone crate must not depend on workspace crate {dep}"),
            Violation::TestkitAsNormalDep { krate } => write!(f, "{krate}: buildkiosk-testkit may only be a dev-dependency"),
            Violation::WebStackBelowL6 { krate, dep, layer } => {
                write!(f, "{krate} (L{layer}) depends on `{dep}`; the web stack is only allowed in layer 6 and up")
            }
        }
    }
}

/// Checks every rule; violations sorted for stable output.
pub fn check(crates: &[Crate]) -> Vec<Violation> {
    let mut out = Vec::new();
    for c in crates {
        let Some(class) = classify(&c.name) else {
            out.push(Violation::Unregistered { krate: c.name.clone() });
            continue;
        };
        if class == Class::Exempt {
            continue;
        }
        let layer = class.layer().unwrap_or(0);
        for d in &c.deps {
            if d.name == c.name {
                continue;
            }
            if d.workspace {
                if class == Class::Standalone {
                    out.push(Violation::StandaloneHasWorkspaceDep { krate: c.name.clone(), dep: d.name.clone() });
                    continue;
                }
                match classify(&d.name) {
                    None => {}
                    Some(Class::Testkit) if d.kind != DepKind::Dev => out.push(Violation::TestkitAsNormalDep { krate: c.name.clone() }),
                    Some(Class::Testkit) => {}
                    Some(dc) => {
                        let to = dc.layer().unwrap_or(u8::MAX);
                        if to >= layer && !(to == layer && intra_layer_allowed(&c.name, &d.name)) {
                            out.push(Violation::Upward {
                                krate: c.name.clone(),
                                dep: d.name.clone(),
                                from: layer,
                                to: if to == u8::MAX { 7 } else { to },
                                kind: d.kind,
                            });
                        }
                    }
                }
            } else if layer < WEB_MIN_LAYER && is_web_stack(&d.name) && d.kind == DepKind::Normal {
                out.push(Violation::WebStackBelowL6 { krate: c.name.clone(), dep: d.name.clone(), layer });
            }
        }
    }
    out.sort_by_key(|v| v.to_string());
    out.dedup();
    out
}

/// Builds the model from `cargo metadata --format-version 1 --no-deps`.
pub fn from_metadata(meta: &Value) -> Result<Vec<Crate>, String> {
    let pkgs = meta["packages"].as_array().ok_or("metadata: no packages array")?;
    let members: Vec<&str> = pkgs.iter().filter_map(|p| p["name"].as_str()).collect();
    let mut out = Vec::new();
    for p in pkgs {
        let name = p["name"].as_str().ok_or("package without name")?.to_owned();
        let mut deps = Vec::new();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dname = d["name"].as_str().unwrap_or_default().to_owned();
            let kind = match d["kind"].as_str() {
                Some("dev") => DepKind::Dev,
                Some("build") => DepKind::Build,
                _ => DepKind::Normal,
            };
            let workspace = members.contains(&dname.as_str()) || d["path"].is_string();
            deps.push(Dep { name: dname, kind, workspace });
        }
        out.push(Crate { name, deps });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

pub fn describe(class: Option<Class>) -> String {
    match class {
        Some(Class::Layer(l)) => format!("L{l}"),
        Some(Class::Standalone) => "L0 standalone".into(),
        Some(Class::Testkit) => "testkit".into(),
        Some(Class::Exempt) => "exempt".into(),
        None => "UNREGISTERED".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use DepKind::*;

    fn c(name: &str, deps: &[(&str, DepKind, bool)]) -> Crate {
        Crate { name: name.into(), deps: deps.iter().map(|(n, k, w)| Dep { name: (*n).into(), kind: *k, workspace: *w }).collect() }
    }

    #[test]
    fn clean_downward_graph_passes() {
        let g = [
            c("buildkiosk-core", &[("serde", Normal, false)]),
            c("buildkiosk-model", &[("buildkiosk-core", Normal, true)]),
            c("buildkiosk-engine", &[("buildkiosk-model", Normal, true), ("buildkiosk-testkit", Dev, true)]),
            c("buildkiosk-server", &[("buildkiosk-engine", Normal, true), ("buildkiosk-web", Normal, true), ("axum", Normal, false)]),
            c("buildkiosk", &[("buildkiosk-server", Normal, true)]),
        ];
        assert!(check(&g).is_empty(), "{:?}", check(&g));
    }

    #[test]
    fn upward_and_sideways_dependencies_flagged() {
        let v = check(&[c("buildkiosk-model", &[("buildkiosk-engine", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 1, to: 5, .. }]));
        let v = check(&[c("buildkiosk-finance", &[("buildkiosk-schedule", Normal, true)])]);
        assert!(matches!(v[..], [Violation::Upward { from: 2, to: 2, .. }]));
        assert!(check(&[c("buildkiosk-server", &[("buildkiosk-store", Normal, true)])]).is_empty());
        assert!(!check(&[c("buildkiosk-store", &[("buildkiosk-server", Normal, true)])]).is_empty());
    }

    #[test]
    fn web_stack_below_l6_flagged() {
        for dep in ["axum", "tokio", "hyper-util", "tower-http", "maud"] {
            let v = check(&[c("buildkiosk-engine", &[(dep, Normal, false)])]);
            assert!(matches!(v[..], [Violation::WebStackBelowL6 { layer: 5, .. }]), "{dep}");
        }
        assert!(check(&[c("buildkiosk-web", &[("maud", Normal, false)])]).is_empty());
    }

    #[test]
    fn standalone_testkit_and_unregistered() {
        assert!(matches!(check(&[c("buildkiosk-core", &[("buildkiosk-model", Normal, true)])])[..], [Violation::StandaloneHasWorkspaceDep { .. }]));
        assert!(matches!(check(&[c("buildkiosk-engine", &[("buildkiosk-testkit", Normal, true)])])[..], [Violation::TestkitAsNormalDep { .. }]));
        assert!(matches!(check(&[c("buildkiosk-mystery", &[])])[..], [Violation::Unregistered { .. }]));
        assert!(check(&[c("xtask", &[("buildkiosk-server", Normal, true)])]).is_empty());
    }
}
