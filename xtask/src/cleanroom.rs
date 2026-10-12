//! `cargo xtask cleanroom`: catches material that must not enter the repository (AGENTS.md ›
//! Clean-room): vendor copyright notices, copyrighted classification and contract-form notices,
//! and test fixtures exported from other products.
//!
//! A line that has to quote one of these phrases (this file, a policy explaining what is banned)
//! says `cleanroom-ok`.

use std::path::Path;

/// Phrases that only appear in material copied from vendors or standards bodies (lowercase).
const PHRASES: &[&str] = &[
    "procore technologies",
    "© procore",
    "(c) procore",
    "copyright procore",
    "copyright (c) procore",
    "copyright © procore",
    "aia document g70",
    "aia® document",
    "american institute of architects. all rights",
    "masterformat®",
    "masterformat ®",
    "© construction specifications institute",
    "construction specifications institute. all rights",
];

/// Directories whose files are test inputs.
const FIXTURE_DIRS: &[&str] = &["fixtures", "testdata", "corpus"];

const ALLOW: &str = "cleanroom-ok";

/// This file lists the phrases it rejects.
const SELF: &str = "xtask/src/cleanroom.rs";

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let mut hits = Vec::new();
    for path in files.iter().filter(|p| p.as_str() != SELF) {
        let Ok(bytes) = std::fs::read(root.join(path)) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        let fixture = path.split('/').any(|d| FIXTURE_DIRS.contains(&d));
        if fixture && !text.contains(ALLOW) && text.to_lowercase().contains("procore") {
            hits.push(format!("{path}: a test fixture that mentions another product looks exported from it; write fixtures from scratch"));
        }
        if bytes.contains(&0) {
            continue;
        }
        for (n, line) in text.lines().enumerate() {
            if line.contains(ALLOW) {
                continue;
            }
            let lower = line.to_lowercase();
            for phrase in PHRASES {
                if lower.contains(phrase) {
                    hits.push(format!("{path}:{}: \"{phrase}\"", n + 1));
                }
            }
        }
    }
    if hits.is_empty() {
        println!("cleanroom: no vendor notices or exported fixtures in {} files", files.len());
        Ok(())
    } else {
        Err(format!("{} clean-room problem(s) (see AGENTS.md › Clean-room):\n  {}", hits.len(), hits.join("\n  ")))
    }
}
