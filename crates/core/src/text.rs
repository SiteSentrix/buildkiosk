//! Checks for text that comes from users, files and agents.
//!
//! Everything a person types reaches the engine as untrusted input: these helpers trim it, drop
//! control characters and enforce length limits so a record can't hold a megabyte title or a
//! terminal escape sequence.

use std::fmt;

/// Limit for names, titles and subjects (characters).
pub const SHORT: usize = 255;
/// Limit for descriptions, questions, answers and notes (characters).
pub const LONG: usize = 20_000;
/// Limit for email addresses (RFC 5321 path limit).
pub const EMAIL: usize = 254;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TextError {
    Empty,
    TooLong { max: usize },
}

impl fmt::Display for TextError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextError::Empty => f.write_str("must not be empty"),
            TextError::TooLong { max } => write!(f, "must be at most {max} characters"),
        }
    }
}

impl std::error::Error for TextError {}

/// One line of text: trimmed, tabs and line breaks turned into spaces, other control characters
/// removed, at most `max` characters. May be empty.
pub fn line(s: &str, max: usize) -> Result<String, TextError> {
    let cleaned: String = s.chars().filter_map(|c| if c.is_control() { matches!(c, '\t' | '\n' | '\r').then_some(' ') } else { Some(c) }).collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() > max { Err(TextError::TooLong { max }) } else { Ok(trimmed.to_string()) }
}

/// Like [`line`], but empty text is an error.
pub fn required_line(s: &str, max: usize) -> Result<String, TextError> {
    let l = line(s, max)?;
    if l.is_empty() { Err(TextError::Empty) } else { Ok(l) }
}

/// Multi-line text: trimmed, `\r\n` and `\r` turned into `\n`, tabs kept, other control characters
/// removed, at most `max` characters. May be empty.
pub fn multiline(s: &str, max: usize) -> Result<String, TextError> {
    let normalized = s.replace("\r\n", "\n").replace('\r', "\n");
    let cleaned: String = normalized.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).collect();
    let trimmed = cleaned.trim();
    if trimmed.chars().count() > max { Err(TextError::TooLong { max }) } else { Ok(trimmed.to_string()) }
}

/// A plausible email address, lowercased: one `@`, a non-empty local part, a dotted domain, no
/// spaces or control characters, at most 254 characters.
pub fn email(s: &str) -> Option<String> {
    let e = s.trim().to_lowercase();
    if e.is_empty() || e.chars().count() > EMAIL || e.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let (local, domain) = e.split_once('@')?;
    let domain_ok = !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && !domain.contains("..")
        && domain.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    (!local.is_empty() && local.len() <= 64 && domain_ok).then_some(e)
}

/// A URL-safe identifier made from a name: lowercase ASCII letters and digits separated by single
/// hyphens, at most `max` characters ("Summit Ridge Builders" → "summit-ridge-builders").
pub fn slug(s: &str, max: usize) -> String {
    let mut out = String::new();
    let mut hyphen = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            if hyphen && !out.is_empty() {
                out.push('-');
            }
            hyphen = false;
            out.push(c.to_ascii_lowercase());
        } else {
            hyphen = true;
        }
        if out.len() >= max {
            break;
        }
    }
    out.chars().take(max).collect::<String>().trim_end_matches('-').to_string()
}

/// The first `n` characters of `s`, with an ellipsis when it was cut (for list views).
pub fn ellipsize(s: &str, n: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(n).collect();
    if chars.next().is_some() { format!("{}…", head.trim_end()) } else { head }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn line_cleans_and_limits() {
        assert_eq!(line("  a\tb\nc \u{1b}[31m ", 20), Ok("a b c [31m".to_string()));
        assert_eq!(line("", 5), Ok(String::new()));
        assert_eq!(required_line("  ", 5), Err(TextError::Empty));
        assert_eq!(line("abcdef", 5), Err(TextError::TooLong { max: 5 }));
        assert_eq!(line("ééééé", 5), Ok("ééééé".to_string()));
    }

    #[test]
    fn multiline_keeps_line_breaks() {
        assert_eq!(multiline(" a\r\nb\rc\u{7}\td \n", 50), Ok("a\nb\nc\td".to_string()));
    }

    #[test]
    fn emails() {
        assert_eq!(email(" Ann@Example.COM "), Some("ann@example.com".to_string()));
        for bad in ["", "a", "a@b", "@b.com", "a@.com", "a@b.com.", "a b@c.com", "a@b@c.com", "a@b..com", "a@b_c.com"] {
            assert_eq!(email(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("Summit Ridge Builders, Inc.", 40), "summit-ridge-builders-inc");
        assert_eq!(slug("  --Ünïcode 2026--  ", 40), "n-code-2026");
        assert_eq!(slug("abcdef", 3), "abc");
        assert_eq!(slug("ab cd", 3), "ab");
        assert_eq!(ellipsize("hello world", 5), "hello…");
        assert_eq!(ellipsize("hi", 5), "hi");
    }

    proptest! {
        #[test]
        fn never_panic_and_respect_limits(s in "\\PC{0,300}", max in 0usize..100) {
            if let Ok(l) = line(&s, max) {
                prop_assert!(l.chars().count() <= max);
                prop_assert!(!l.chars().any(char::is_control));
            }
            if let Ok(m) = multiline(&s, max) {
                prop_assert!(m.chars().count() <= max);
            }
            let sl = slug(&s, max);
            prop_assert!(sl.len() <= max);
            let _ = email(&s);
            let _ = ellipsize(&s, max);
        }
    }
}
