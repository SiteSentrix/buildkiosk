//! `cargo xtask stats`: tests and lines of Rust per crate.

use std::path::Path;

pub fn run(root: &Path) -> Result<(), String> {
    let files = crate::repo_files(root)?;
    let mut rows: Vec<(String, usize, usize)> = Vec::new();
    for f in files.iter().filter(|f| f.ends_with(".rs")) {
        let krate = {
            let mut parts = f.split('/');
            match (parts.next(), parts.next()) {
                (Some(top @ ("crates" | "apps")), Some(name)) => format!("{top}/{name}"),
                (Some(top), _) => top.to_string(),
                _ => continue,
            }
        };
        let text = std::fs::read_to_string(root.join(f)).unwrap_or_default();
        let lines = text.lines().count();
        let tests = text.matches("#[test]").count() + text.matches("proptest! {").count();
        match rows.iter_mut().find(|r| r.0 == krate) {
            Some(r) => {
                r.1 += lines;
                r.2 += tests;
            }
            None => rows.push((krate, lines, tests)),
        }
    }
    rows.sort();
    println!("{:<28} {:>8} {:>7}", "crate", "lines", "tests");
    for (k, l, t) in &rows {
        println!("{k:<28} {l:>8} {t:>7}");
    }
    let (l, t) = rows.iter().fold((0, 0), |(a, b), r| (a + r.1, b + r.2));
    println!("{:<28} {l:>8} {t:>7}", "total");
    println!("(tests = #[test] functions plus proptest! blocks)");
    Ok(())
}
