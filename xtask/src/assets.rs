//! `cargo xtask assets`: every non-code asset must be attributed in `ASSETS.md`.
//!
//! Scans all tracked (and untracked, not ignored) files with an asset extension, plus everything
//! under `assets/` and `docs/images/`, and fails if a path is not listed as `` `path` `` in
//! ASSETS.md (AGENTS.md › Assets).

use std::path::Path;

const ASSET_EXT: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "icns", "svg", "pdf", "psd", "ttf", "otf", "woff", "woff2", "mp4", "webm",
    "wav", "mp3", "dwg", "dxf", "ifc", "rvt",
];
const ASSET_DIRS: &[&str] = &["assets/", "docs/images/"];

pub fn is_asset(path: &str) -> bool {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    ASSET_DIRS.iter().any(|d| path.starts_with(d)) || ASSET_EXT.contains(&ext.as_str())
}

/// Paths that need an ASSETS.md entry but lack one.
pub fn missing(files: &[String], assets_md: &str) -> Vec<String> {
    files.iter().filter(|f| is_asset(f) && !assets_md.contains(&format!("`{f}`"))).cloned().collect()
}

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let md = std::fs::read_to_string(root.join("ASSETS.md")).map_err(|e| format!("ASSETS.md: {e}"))?;
    let miss = missing(&files, &md);
    if miss.is_empty() {
        println!("assets: all {} asset files attributed in ASSETS.md", files.iter().filter(|f| is_asset(f)).count());
        Ok(())
    } else {
        Err(format!("{} asset file(s) lack an ASSETS.md entry (author, source, licence):\n  {}", miss.len(), miss.join("\n  ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_unattributed_assets() {
        let files = vec!["assets/icon.svg".to_string(), "crates/x/src/lib.rs".into(), "docs/images/b.png".into(), "tests/fixture.pdf".into()];
        let md = "| `assets/icon.svg` | me | here | MIT |";
        assert_eq!(missing(&files, md), vec!["docs/images/b.png".to_string(), "tests/fixture.pdf".into()]);
        assert!(!is_asset("crates/x/src/lib.rs"));
    }
}
