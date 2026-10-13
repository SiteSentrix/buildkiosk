//! Embeds the browser sheet viewer (built by `cargo xtask viewer` into `assets/viewer`) when it
//! exists. Without it the server still works and the viewer page is the one the server draws.

use std::fmt::Write as _;
use std::path::Path;

const FILES: [(&str, &str); 3] = [("START_JS", "start.js"), ("VIEWER_JS", "viewer.js"), ("VIEWER_WASM", "viewer_bg.wasm")];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR")?).join("assets/viewer");
    println!("cargo:rerun-if-changed=assets/viewer");
    println!("cargo:rerun-if-changed=build.rs");
    // All three files or none: a half-built viewer would load and then fail.
    let all = FILES.iter().all(|(_, name)| dir.join(name).is_file());
    let mut out = String::new();
    for (constant, name) in FILES {
        if all {
            let path = dir.join(name);
            writeln!(out, "pub const {constant}: Option<&[u8]> = Some(include_bytes!({:?}));", path.display().to_string())?;
        } else {
            writeln!(out, "pub const {constant}: Option<&[u8]> = None;")?;
        }
    }
    std::fs::write(Path::new(&std::env::var("OUT_DIR")?).join("viewer_assets.rs"), out)?;
    Ok(())
}
