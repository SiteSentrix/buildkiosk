//! A small Rust lexer for the text checks: finds string literals outside comments and outside
//! `#[cfg(test)]` items.

/// A string literal in Rust source.
#[derive(Debug, PartialEq)]
pub struct Literal {
    /// 1-based line where the literal starts.
    pub line: usize,
    pub text: String,
}

/// The string literals of `src` (normal, raw and byte strings), skipping comments, char literals
/// and items marked `#[cfg(test)]`.
pub fn string_literals(src: &str) -> Vec<Literal> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    // Skipping a #[cfg(test)] item: waiting for its `{` (or `;`), then the brace depth inside it.
    let mut pending_skip = false;
    let mut skip_depth = 0usize;
    let at = |j: usize| chars.get(j).copied();
    while let Some(c) = at(i) {
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        // Comments.
        if c == '/' && at(i + 1) == Some('/') {
            while let Some(ch) = at(i) {
                if ch == '\n' {
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == Some('*') {
            let mut depth = 0usize;
            while let Some(ch) = at(i) {
                if ch == '/' && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                } else if ch == '*' && at(i + 1) == Some('/') {
                    depth = depth.saturating_sub(1);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if ch == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        // #[cfg(test)]
        if c == '#' && chars.get(i..i + 12).is_some_and(|s| s.iter().collect::<String>() == "#[cfg(test)]") {
            pending_skip = skip_depth == 0;
            i += 12;
            continue;
        }
        // Strings: "..", r".."/r#".."#, b"..", br"..".
        let (is_raw, prefix_len) = match (c, at(i + 1), at(i + 2)) {
            ('r', Some('"' | '#'), _) if i == 0 || !is_ident(at(i - 1)) => (true, 1),
            ('b', Some('r'), Some('"' | '#')) if i == 0 || !is_ident(at(i - 1)) => (true, 2),
            ('b', Some('"'), _) if i == 0 || !is_ident(at(i - 1)) => (false, 1),
            ('"', _, _) => (false, 0),
            _ => (false, usize::MAX),
        };
        if prefix_len != usize::MAX {
            let start_line = line;
            let mut j = i + prefix_len;
            let mut text = String::new();
            if is_raw {
                let mut hashes = 0usize;
                while at(j) == Some('#') {
                    hashes += 1;
                    j += 1;
                }
                if at(j) != Some('"') {
                    // `r#ident` (a raw identifier), not a string.
                    i = j;
                    continue;
                }
                j += 1;
                loop {
                    match at(j) {
                        None => break,
                        Some('"') if (1..=hashes).all(|k| at(j + k) == Some('#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        Some(ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            text.push(ch);
                            j += 1;
                        }
                    }
                }
            } else {
                j += 1;
                loop {
                    match at(j) {
                        None => break,
                        Some('\\') => {
                            if at(j + 1) == Some('\n') {
                                line += 1;
                            }
                            j += 2;
                        }
                        Some('"') => {
                            j += 1;
                            break;
                        }
                        Some(ch) => {
                            if ch == '\n' {
                                line += 1;
                            }
                            text.push(ch);
                            j += 1;
                        }
                    }
                }
            }
            if skip_depth == 0 && !pending_skip {
                out.push(Literal { line: start_line, text });
            }
            i = j;
            continue;
        }
        // Char literals vs lifetimes.
        if c == '\'' {
            if at(i + 1) == Some('\\') {
                let mut j = i + 2;
                while let Some(ch) = at(j) {
                    j += 1;
                    if ch == '\'' {
                        break;
                    }
                }
                i = j;
                continue;
            }
            if at(i + 2) == Some('\'') {
                i += 3;
                continue;
            }
            i += 1;
            continue;
        }
        // Items under #[cfg(test)].
        if pending_skip {
            if c == '{' {
                pending_skip = false;
                skip_depth = 1;
            } else if c == ';' {
                pending_skip = false;
            }
        } else if skip_depth > 0 {
            if c == '{' {
                skip_depth += 1;
            } else if c == '}' {
                skip_depth -= 1;
            }
        }
        i += 1;
    }
    out
}

fn is_ident(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_alphanumeric() || c == '_')
}

/// Whole-word, case-insensitive matches of `terms` in `text` (multi-word terms match across single
/// spaces).
pub fn terms_in(text: &str, terms: &[&'static str]) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let mut hits = Vec::new();
    for term in terms {
        let mut from = 0usize;
        while let Some(pos) = lower.get(from..).and_then(|s| s.find(term)).map(|p| p + from) {
            let before = lower.get(..pos).and_then(|s| s.chars().next_back());
            let after = lower.get(pos + term.len()..).and_then(|s| s.chars().next());
            let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
            if boundary(before) && boundary(after) {
                hits.push(*term);
                break;
            }
            from = pos + term.len().max(1);
        }
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_literals_and_skips_comments_chars_and_tests() {
        let src = r####"
// "in a comment"
/* "block" /* "nested" */ */
fn f() -> &'static str { let c = '"'; let d = '\''; "real one" }
const R: &str = r#"raw "quoted""#;
const B: &[u8] = b"bytes";
let r#type = 1;
#[cfg(test)]
mod tests { fn g() { "test only" } }
fn h() { "after" }
"####;
        let lits: Vec<String> = string_literals(src).into_iter().map(|l| l.text).collect();
        assert_eq!(lits, vec!["real one", "raw \"quoted\"", "bytes", "after"]);
        assert_eq!(string_literals("\n\n  \"x\"")[0].line, 3);
    }

    #[test]
    fn terms_match_whole_words() {
        let t = ["vendorx", "big suite"];
        assert_eq!(terms_in("Export to VendorX now", &t), vec!["vendorx"]);
        assert!(terms_in("vendorxtra", &t).is_empty());
        assert_eq!(terms_in("the Big Suite.", &t), vec!["big suite"]);
    }
}
