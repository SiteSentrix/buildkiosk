//! Embeds the browser sheet viewer (built by `cargo xtask viewer` into `assets/viewer`) when it
//! exists, compressed with gzip. Without it the server still works and the viewer page is the one
//! the server draws.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

const FILES: [(&str, &str); 3] = [("START_JS", "start.js"), ("VIEWER_JS", "viewer.js"), ("VIEWER_WASM", "viewer_bg.wasm")];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR")?).join("assets/viewer");
    let out_dir = std::env::var("OUT_DIR")?;
    println!("cargo:rerun-if-changed=assets/viewer");
    println!("cargo:rerun-if-changed=build.rs");
    // All three files or none: a half-built viewer would load and then fail.
    let all = FILES.iter().all(|(_, name)| dir.join(name).is_file());
    let mut out = String::new();
    for (constant, name) in FILES {
        if all {
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            gz.write_all(&std::fs::read(dir.join(name))?)?;
            let target = Path::new(&out_dir).join(format!("{name}.gz"));
            std::fs::write(&target, gz.finish()?)?;
            writeln!(out, "pub const {constant}: Option<&[u8]> = Some(include_bytes!({:?}));", target.display().to_string())?;
        } else {
            writeln!(out, "pub const {constant}: Option<&[u8]> = None;")?;
        }
    }
    std::fs::write(Path::new(&out_dir).join("viewer_assets.rs"), out)?;
    Ok(())
}
