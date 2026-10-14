//! Workspace tooling: `cargo xtask <command>`.
//!
//! Pure Rust (std + serde_json). External tools (`cargo`, `git`) are invoked through
//! `std::process::Command`.

mod assets;
mod brands;
mod cleanroom;
mod layers;
mod licenses;
mod scan;
mod stats;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: cargo xtask <command>

commands:
  assets     check that every icon/image/font/asset is attributed in ASSETS.md
  brands     check that user-visible text (labels, params docs, page text, Cargo descriptions) names no other vendor's products
  cleanroom  check for vendor copyright notices and for fixtures exported from other products (AGENTS.md › Clean-room)
  licenses   check that every dependency has a permissive licence (no GPL, LGPL or AGPL)
  layers     enforce the crate dependency layering (docs/architecture.md › Layers)
  wasm       cargo check --target wasm32-unknown-unknown for the layer 0–5 crates and the viewer
  viewer     build the browser sheet viewer (WebAssembly) into crates/server/assets/viewer; needs `wasm-bindgen` (cargo install wasm-bindgen-cli)
  i18n       check that the language catalogs' list of wording (crates/web/i18n/en.txt) has everything the pages say now; `cargo xtask i18n --update` rewrites it
  stats      count tests and lines of Rust per crate
  ci         fmt --check, clippy -D warnings, test, assets, brands, cleanroom, licenses, layers, wasm, i18n (stops at the first failure)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = root();
    let result = match args.first().map(String::as_str) {
        Some("assets") => assets::run(&root),
        Some("brands") => brands::run(&root),
        Some("cleanroom") => cleanroom::run(&root),
        Some("licenses") => licenses::run(),
        Some("layers") => cmd_layers(),
        Some("wasm") => cmd_wasm(),
        Some("viewer") => cmd_viewer(),
        Some("i18n") => cmd_i18n(args.get(1).is_some_and(|a| a == "--update")),
        Some("stats") => stats::run(&root),
        Some("ci") => cmd_ci(),
        Some("-h" | "--help" | "help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Workspace root (parent of the xtask crate).
pub fn root() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.parent().unwrap_or(dir).to_path_buf()
}

/// Tracked and untracked (not ignored) files that exist, relative to `root`.
pub fn repo_files(root: &Path) -> Result<Vec<String>, String> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "--cached", "--others", "--exclude-standard"])
        .output()
        .map_err(|e| format!("git ls-files: {e}"))?;
    if !out.status.success() {
        return Err(format!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).lines().filter(|l| root.join(l).exists()).map(str::to_owned).collect())
}

pub fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: failed to spawn: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{what}: exited with {status}")) }
}

/// `cargo metadata` for the workspace members only.
pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo().args(["metadata", "--format-version", "1", "--no-deps"]).output().map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!("cargo metadata failed:\n{}", String::from_utf8_lossy(&out.stderr)));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("Dependency layering (docs/architecture.md › Layers)\n");
    println!("{:<24} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!("{:<24} {:<14} {}", c.name, layers::describe(layers::classify(&c.name)), ws.join(", "));
    }
    let violations = layers::check(&crates);
    println!();
    if violations.is_empty() {
        println!("OK: {} crates, no layering violations.", crates.len());
        Ok(())
    } else {
        println!("{} violation(s):", violations.len());
        for v in &violations {
            println!("  - {v}");
        }
        Err(format!("{} layering violation(s)", violations.len()))
    }
}

/// Workspace packages that must build for wasm32: every layer 0–5 crate.
fn wasm_set() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .filter(|c| match layers::classify(&c.name) {
            Some(layers::Class::Layer(l)) => l <= 5,
            Some(layers::Class::Standalone) => true,
            _ => false,
        })
        .map(|c| c.name)
        .collect())
}

fn cmd_wasm() -> Result<(), String> {
    let set = wasm_set()?;
    let mut c = cargo();
    c.args(["check", "--target", "wasm32-unknown-unknown"]);
    for pkg in &set {
        c.args(["-p", pkg]);
    }
    // The browser sheet viewer is layer 6 but is itself WebAssembly.
    c.args(["-p", VIEWER_PACKAGE]);
    run(c, &format!("cargo check --target wasm32-unknown-unknown -p {} -p {VIEWER_PACKAGE}", set.join(" -p ")))?;
    println!("wasm32-unknown-unknown: ok ({}, viewer)", set.join(", "));
    Ok(())
}

const VIEWER_PACKAGE: &str = "buildkiosk-viewer";

/// Builds the browser viewer and its loader into `crates/server/assets/viewer`, which the server
/// embeds when it is compiled (and which is not committed). Without these files the server still
/// works: sheets are shown by the page the server draws.
///
/// The loader is made here, not written by hand: `wasm-bindgen` generates `viewer.js` (the glue
/// that starts the module) and the one line in `start.js` calls it.
fn cmd_viewer() -> Result<(), String> {
    let want = viewer_bindgen_version()?;
    let have = Command::new("wasm-bindgen").arg("--version").output().ok().and_then(|o| String::from_utf8(o.stdout).ok()).unwrap_or_default();
    if have.split_whitespace().nth(1) != Some(want.as_str()) {
        return Err(format!(
            "the viewer needs wasm-bindgen {want} (found: {}); install it with `cargo install wasm-bindgen-cli --version {want} --locked`",
            if have.trim().is_empty() { "none" } else { have.trim() }
        ));
    }
    let mut c = cargo();
    c.args(["build", "--release", "--target", "wasm32-unknown-unknown", "-p", VIEWER_PACKAGE]);
    run(c, "cargo build --release --target wasm32-unknown-unknown -p buildkiosk-viewer")?;
    let out = root().join("crates/server/assets/viewer");
    // Start clean so a file from an older build is never served.
    if out.exists() {
        std::fs::remove_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;
    let mut b = Command::new("wasm-bindgen");
    b.current_dir(root())
        .args(["--target", "web", "--no-typescript", "--out-name", "viewer", "--out-dir"])
        .arg(&out)
        .arg("target/wasm32-unknown-unknown/release/buildkiosk_viewer.wasm");
    run(b, "wasm-bindgen --target web")?;
    std::fs::write(out.join("start.js"), "import init from \"./viewer.js\";\ninit();\n").map_err(|e| format!("start.js: {e}"))?;
    for f in ["start.js", "viewer.js", "viewer_bg.wasm"] {
        let size = std::fs::metadata(out.join(f)).map(|m| m.len()).map_err(|e| format!("{f}: {e}"))?;
        println!("  {f}: {} KB", size / 1024);
    }
    println!("viewer: ok. Rebuild the server (cargo build -p buildkiosk --release) to embed it.");
    Ok(())
}

/// The `wasm-bindgen` version the viewer is locked to (the command line tool must match it).
fn viewer_bindgen_version() -> Result<String, String> {
    let lock = std::fs::read_to_string(root().join("Cargo.lock")).map_err(|e| format!("Cargo.lock: {e}"))?;
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l == "name = \"wasm-bindgen\"" {
            let v = lines.next().unwrap_or_default();
            return v
                .strip_prefix("version = \"")
                .and_then(|v| v.strip_suffix('"'))
                .map(str::to_owned)
                .ok_or_else(|| "Cargo.lock: unreadable wasm-bindgen entry".into());
        }
    }
    Err("Cargo.lock has no wasm-bindgen entry".into())
}

/// Visits the demo company's pages and checks that every piece of wording on them is in the list the language
/// catalogs are held to (`crates/web/i18n/en.txt`); the list may hold more (wording that only some days or states show).
fn cmd_i18n(update: bool) -> Result<(), String> {
    let list = root().join("crates/web/i18n/en.txt");
    let out = if update { list.clone() } else { std::env::temp_dir().join(format!("buildkiosk-wording-{}.txt", std::process::id())) };
    let mut c = cargo();
    c.args(["run", "-q", "-p", "buildkiosk", "--", "i18n-harvest", "--out"]).arg(&out);
    run(c, "buildkiosk i18n-harvest")?;
    if update {
        println!("rewrote {}; now translate the new wording in es.txt, fr.txt and nl.txt", list.display());
        return Ok(());
    }
    let read = |p: &std::path::Path| -> Result<std::collections::BTreeSet<String>, String> {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        Ok(text.lines().filter(|l| !l.is_empty() && !l.starts_with('#')).map(str::to_string).collect())
    };
    let (now, have) = (read(&out)?, read(&list)?);
    let _ = std::fs::remove_file(&out);
    let new: Vec<&String> = now.difference(&have).collect();
    if new.is_empty() {
        println!("i18n: all {} pieces of wording on the pages are in the list", now.len());
        return Ok(());
    }
    for w in new.iter().take(30) {
        eprintln!("  new wording: {w}");
    }
    Err(format!(
        "{} pieces of wording on the pages are not in crates/web/i18n/en.txt (so no language has them): run `cargo xtask i18n --update`, then add their translations to es.txt, fr.txt and nl.txt",
        new.len()
    ))
}

fn cmd_ci() -> Result<(), String> {
    type Step = (&'static str, Box<dyn Fn() -> Result<(), String>>);
    let steps: Vec<Step> = vec![
        (
            "fmt",
            Box::new(|| {
                let mut c = cargo();
                c.args(["fmt", "--all", "--", "--check"]);
                run(c, "cargo fmt --all -- --check")
            }),
        ),
        (
            "clippy",
            Box::new(|| {
                let mut c = cargo();
                c.args(["clippy", "--workspace", "--all-targets", "--", "-D", "warnings"]);
                run(c, "cargo clippy --workspace --all-targets -- -D warnings")
            }),
        ),
        (
            "test",
            Box::new(|| {
                let mut c = cargo();
                c.args(["test", "--workspace"]);
                run(c, "cargo test --workspace")
            }),
        ),
        ("assets", Box::new(|| assets::run(&root()))),
        ("brands", Box::new(|| brands::run(&root()))),
        ("cleanroom", Box::new(|| cleanroom::run(&root()))),
        ("licenses", Box::new(licenses::run)),
        ("layers", Box::new(cmd_layers)),
        ("wasm", Box::new(cmd_wasm)),
        ("i18n", Box::new(|| cmd_i18n(false))),
    ];
    let mut done = Vec::new();
    for (name, f) in &steps {
        eprintln!("\n=== ci: {name} ===");
        if let Err(e) = f() {
            println!("\nCI summary:");
            for d in &done {
                println!("  ok    {d}");
            }
            println!("  FAIL  {name}: {e}");
            for (n, _) in steps.iter().skip(done.len() + 1) {
                println!("  skip  {n}");
            }
            return Err(format!("ci failed at `{name}`"));
        }
        done.push(*name);
    }
    println!("\nCI summary: all {} steps passed ({})", done.len(), done.join(", "));
    Ok(())
}
