//! Showing the pages in another language.
//!
//! Pages are written in English. A page that is about to be sent to someone who reads another
//! language is rewritten once, on its way out: every piece of text the page's own wording supplies
//! (headings, buttons, labels, table heads, hints) is looked up in that language's catalog and
//! replaced; anything the catalog doesn't know (names, titles and notes people typed, numbers) is
//! left exactly as it is. Nothing about what is stored changes.
//!
//! A catalog (`i18n/<code>.txt`) has one line per piece of wording: the English text, a tab and the
//! translation. A piece of wording is a run of text up to a ` · ` or `: ` (those separators are kept).
//! Where live values sit inside it, the English has `{}` in their place, and the translation has them
//! too (`{} of {} items done` → `{} de {} elementos hechos`); `{2}` names the second value when a
//! language needs them the other way round. `i18n/en.txt` lists every piece of wording the catalogs
//! must cover: `buildkiosk i18n-harvest` regenerates it from the source and the demo company, and a
//! test fails when a language lacks any of it.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::OnceLock;

/// The languages the interface can be shown in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Es,
    Fr,
    Nl,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::En, Lang::Es, Lang::Fr, Lang::Nl];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Fr => "fr",
            Lang::Nl => "nl",
        }
    }

    /// The language's name in that language.
    pub fn name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Fr => "Français",
            Lang::Nl => "Nederlands",
        }
    }

    pub fn parse(code: &str) -> Option<Lang> {
        let code = code.trim().to_ascii_lowercase();
        let base = code.split(['-', '_']).next().unwrap_or("");
        Lang::ALL.into_iter().find(|l| l.code() == base)
    }

    /// The first language an `Accept-Language` header asks for (best weight first) that is offered.
    pub fn from_accept_language(header: &str) -> Lang {
        let mut wanted: Vec<(f32, usize, Lang)> = Vec::new();
        for (i, part) in header.split(',').take(20).enumerate() {
            let mut bits = part.split(';');
            let tag = bits.next().unwrap_or("").trim();
            let q = bits.find_map(|b| b.trim().strip_prefix("q=")?.trim().parse::<f32>().ok()).unwrap_or(1.0);
            if let Some(l) = Lang::parse(tag).filter(|_| q > 0.0) {
                wanted.push((q, i, l));
            }
        }
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        wanted.first().map_or(Lang::En, |w| w.2)
    }

    fn catalog_text(self) -> &'static str {
        match self {
            Lang::En => "",
            Lang::Es => include_str!("../i18n/es.txt"),
            Lang::Fr => include_str!("../i18n/fr.txt"),
            Lang::Nl => include_str!("../i18n/nl.txt"),
        }
    }

    fn days(self) -> [&'static str; 7] {
        match self {
            Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"],
            Lang::Es => ["lun", "mar", "mié", "jue", "vie", "sáb", "dom"],
            Lang::Fr => ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."],
            Lang::Nl => ["ma", "di", "wo", "do", "vr", "za", "zo"],
        }
    }

    /// The word between the two ends of a range of dates.
    fn range_word(self) -> &'static str {
        match self {
            Lang::En => "to",
            Lang::Es => "al",
            Lang::Fr => "au",
            Lang::Nl => "t/m",
        }
    }

    fn months(self) -> [&'static str; 12] {
        match self {
            Lang::En => ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"],
            Lang::Es => ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"],
            Lang::Fr => ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."],
            Lang::Nl => ["jan.", "feb.", "mrt.", "apr.", "mei", "jun.", "jul.", "aug.", "sep.", "okt.", "nov.", "dec."],
        }
    }
}

/// English wording that stands for a set of values (`{}`), with its translation.
struct Pattern {
    /// The fixed text between the values.
    parts: Vec<String>,
    translation: Vec<Piece>,
    /// How much fixed text there is, to prefer the most specific pattern.
    weight: usize,
}

/// A piece of a translation: text, or the nth value (from 0) captured from the English.
enum Piece {
    Text(String),
    Value(usize),
}

struct Catalog {
    exact: HashMap<String, String>,
    patterns: Vec<Pattern>,
}

/// Splits a translation into text and `{}` / `{2}` references to captured values.
fn pieces(translation: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let (mut text, mut next, mut rest) = (String::new(), 0usize, translation);
    while let Some(at) = rest.find('{') {
        let Some(close) = rest.get(at..).and_then(|r| r.find('}')).map(|c| at + c) else { break };
        let inner = rest.get(at + 1..close).unwrap_or("");
        let index = if inner.is_empty() {
            next += 1;
            Some(next - 1)
        } else {
            inner.parse::<usize>().ok().filter(|n| *n >= 1).map(|n| n - 1)
        };
        match index {
            Some(i) => {
                text.push_str(rest.get(..at).unwrap_or(""));
                if !text.is_empty() {
                    out.push(Piece::Text(std::mem::take(&mut text)));
                }
                out.push(Piece::Value(i));
            }
            None => text.push_str(rest.get(..=close).unwrap_or("")),
        }
        rest = rest.get(close + 1..).unwrap_or("");
    }
    text.push_str(rest);
    if !text.is_empty() {
        out.push(Piece::Text(text));
    }
    out
}

impl Catalog {
    fn parse(text: &str) -> Catalog {
        let mut exact = HashMap::new();
        let mut patterns = Vec::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('\t') else { continue };
            if key.starts_with('#') || key.is_empty() || value.trim().is_empty() {
                continue;
            }
            if key.contains("{}") {
                let parts: Vec<String> = key.split("{}").map(str::to_string).collect();
                let weight = parts.iter().map(String::len).sum();
                patterns.push(Pattern { parts, translation: pieces(value), weight });
            } else {
                exact.insert(key.to_string(), value.to_string());
            }
        }
        patterns.sort_by_key(|p| std::cmp::Reverse(p.weight));
        Catalog { exact, patterns }
    }

    /// The translation of one piece of wording (already trimmed, single spaces).
    fn lookup(&self, core: &str) -> Option<String> {
        if let Some(t) = self.exact.get(core) {
            return Some(t.clone());
        }
        self.patterns.iter().find_map(|p| {
            let values = capture(&p.parts, core)?;
            let mut out = String::with_capacity(core.len() + 16);
            for piece in &p.translation {
                match piece {
                    Piece::Text(t) => out.push_str(t),
                    Piece::Value(i) => out.push_str(values.get(*i).copied()?),
                }
            }
            Some(out)
        })
    }
}

/// The text standing in for each `{}` when `text` fits a pattern with these fixed parts.
pub fn capture<'a>(parts: &[String], text: &'a str) -> Option<Vec<&'a str>> {
    let (first, rest) = parts.split_first()?;
    let (last, middle) = match rest.split_last() {
        Some((last, middle)) => (last, middle),
        None => return (text == first).then(Vec::new),
    };
    let mut pos = first.len();
    if !text.starts_with(first.as_str()) || text.len() < first.len() + last.len() || !text.ends_with(last.as_str()) {
        return None;
    }
    let end = text.len() - last.len();
    let mut values = Vec::with_capacity(parts.len());
    for part in middle {
        // The value before it is at least one character long.
        let from = pos + text.get(pos..)?.chars().next()?.len_utf8();
        let at = from + text.get(from..end)?.find(part.as_str())?;
        values.push(text.get(pos..at)?);
        pos = at + part.len();
    }
    let tail = text.get(pos..end)?;
    if tail.is_empty() {
        return None;
    }
    values.push(tail);
    Some(values)
}

fn catalog(lang: Lang) -> &'static Catalog {
    static EN: OnceLock<Catalog> = OnceLock::new();
    static ES: OnceLock<Catalog> = OnceLock::new();
    static FR: OnceLock<Catalog> = OnceLock::new();
    static NL: OnceLock<Catalog> = OnceLock::new();
    let cell = match lang {
        Lang::En => &EN,
        Lang::Es => &ES,
        Lang::Fr => &FR,
        Lang::Nl => &NL,
    };
    cell.get_or_init(|| Catalog::parse(lang.catalog_text()))
}

// ---- text pieces ------------------------------------------------------------------------------

/// Collapses runs of whitespace to one space.
fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Cuts text at ` · ` and `: `, giving each piece and the separator that follows it.
pub fn split_pieces(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let bytes = text.as_bytes();
    while i < bytes.len() {
        let sep = if text.get(i..).is_some_and(|r| r.starts_with(" · ")) {
            " · "
        } else if text.get(i..).is_some_and(|r| r.starts_with(": ")) {
            ": "
        } else {
            i += 1;
            continue;
        };
        out.push((text.get(start..i).unwrap_or(""), sep));
        i += sep.len();
        start = i;
    }
    out.push((text.get(start..).unwrap_or(""), ""));
    out
}

fn has_letters(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic())
}

/// The pieces of wording in a run of text, trimmed and with single spaces, that could be looked up.
pub fn wording(text: &str) -> Vec<String> {
    split_pieces(text).into_iter().map(|(p, _)| squash(p)).filter(|p| has_letters(p)).collect()
}

/// Translates the pieces of wording in a run of text; `None` when nothing changed.
fn translate_text(text: &str, lang: Lang) -> Option<String> {
    let cat = catalog(lang);
    let mut out = String::with_capacity(text.len() + 16);
    let mut changed = false;
    for (piece, sep) in split_pieces(text) {
        let core = squash(piece);
        let translated = if has_letters(&core) { cat.lookup(&core) } else { None };
        match translated {
            Some(t) => {
                // Keep the spaces the piece had at its ends.
                let lead = piece.len() - piece.trim_start().len();
                let trail = piece.len() - piece.trim_end().len();
                out.push_str(piece.get(..lead).unwrap_or(""));
                out.push_str(&localize_dates(&t, lang));
                out.push_str(piece.get(piece.len() - trail..).unwrap_or(""));
                changed = true;
            }
            None => {
                let dated = localize_dates(piece, lang);
                changed |= dated != piece;
                out.push_str(&dated);
            }
        }
        out.push_str(sep);
    }
    changed.then_some(out)
}

/// Dates and days written the way the language writes them: `Oct 12, 2026` → `12 oct 2026`, `Nov 2 to Nov 13` →
/// `2 nov al 13 nov`, `Oct 2026` → `oct 2026`, `Fri Oct 16, 2026` → `vie 16 oct 2026`, `Fri 2026-10-09` → `vie 2026-10-09`.
pub fn localize_dates(text: &str, lang: Lang) -> Cow<'_, str> {
    if lang == Lang::En {
        return Cow::Borrowed(text);
    }
    let b = text.as_bytes();
    let boundary = |i: usize| i == 0 || b.get(i - 1).is_some_and(|c| !c.is_ascii_alphanumeric());
    let digits = |from: usize, most: usize| -> usize { (0..most).take_while(|k| b.get(from + k).is_some_and(u8::is_ascii_digit)).count() };
    let months = Lang::En.months();
    let local = lang.months();
    // A date starting at `i`: how far it runs and how it is written here.
    let date_at = |i: usize| -> Option<(usize, String)> {
        let m = months.iter().position(|e| text.get(i..i + 3) == Some(*e))?;
        let name = local.get(m).copied()?;
        if !boundary(i) || b.get(i + 3) != Some(&b' ') {
            return None;
        }
        let d = digits(i + 4, 4);
        match d {
            // `Oct 2026`
            4 if !b.get(i + 8).is_some_and(u8::is_ascii_digit) && !b.get(i + 8).is_some_and(|c| *c == b'-') => {
                Some((i + 8, format!("{name} {}", text.get(i + 4..i + 8)?)))
            }
            1 | 2 => {
                let day = text.get(i + 4..i + 4 + d)?;
                let after = i + 4 + d;
                if b.get(after) == Some(&b',') && b.get(after + 1) == Some(&b' ') && digits(after + 2, 5) == 4 {
                    // `Oct 12, 2026`
                    return Some((after + 6, format!("{day} {name} {}", text.get(after + 2..after + 6)?)));
                }
                // `Oct 12, 20266` is a badly written year, not a day and something else.
                if b.get(after) == Some(&b',') && b.get(after + 1) == Some(&b' ') && b.get(after + 2).is_some_and(u8::is_ascii_digit) {
                    return None;
                }
                // `Oct 12` (not the start of a longer number or word)
                (!b.get(after).is_some_and(|c| c.is_ascii_alphanumeric())).then(|| (after, format!("{day} {name}")))
            }
            _ => None,
        }
    };
    let days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let local_days = lang.days();
    let to = lang.range_word();
    let mut out: Option<String> = None;
    let (mut copied, mut i) = (0usize, 0usize);
    while i < b.len() {
        // A weekday that comes before a date: `Fri 2026-10-09`, `Fri Oct 16, 2026`.
        let weekday = days.iter().position(|d| text.get(i..i + 3) == Some(*d)).filter(|_| boundary(i) && b.get(i + 3) == Some(&b' '));
        let next_is_date = |at: usize| digits(at, 4) == 4 || date_at(at).is_some();
        if let Some(w) = weekday.filter(|_| next_is_date(i + 4)) {
            let o = out.get_or_insert_with(String::new);
            o.push_str(text.get(copied..i).unwrap_or(""));
            o.push_str(local_days.get(w).copied().unwrap_or(""));
            copied = i + 3;
            i += 3;
            continue;
        }
        if let Some((mut end, mut written)) = date_at(i) {
            // `Nov 2 to Nov 13`
            while text.get(end..).is_some_and(|r| r.starts_with(" to ")) {
                match date_at(end + 4) {
                    Some((e2, w2)) => {
                        written = format!("{written} {to} {w2}");
                        end = e2;
                    }
                    None => break,
                }
            }
            let o = out.get_or_insert_with(String::new);
            o.push_str(text.get(copied..i).unwrap_or(""));
            o.push_str(&written);
            copied = end;
            i = end;
            continue;
        }
        i += 1;
    }
    match out {
        Some(mut o) => {
            o.push_str(text.get(copied..).unwrap_or(""));
            Cow::Owned(o)
        }
        None => Cow::Borrowed(text),
    }
}

// ---- HTML -------------------------------------------------------------------------------------

fn decode(s: &str) -> Cow<'_, str> {
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&#39;", "'")
            .replace("&#x27;", "'")
            .replace("&nbsp;", "\u{a0}")
            .replace("&amp;", "&"),
    )
}

fn encode(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Index just past the `>` that closes the tag starting at `start` (quotes may hold a `>`).
fn tag_end(html: &str, start: usize) -> usize {
    let mut quote: Option<u8> = None;
    for (k, &c) in html.as_bytes().iter().enumerate().skip(start + 1) {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(c),
            (None, b'>') => return k + 1,
            _ => {}
        }
    }
    html.len()
}

/// The attributes whose value is wording a person reads.
fn reads_as_wording(tag: &str, attr: &str, attrs: &[(String, String)]) -> bool {
    match attr {
        "placeholder" | "title" | "aria-label" | "alt" => true,
        "value" => tag == "input" && attrs.iter().any(|(n, v)| n == "type" && matches!(v.as_str(), "submit" | "button" | "reset")),
        _ => false,
    }
}

/// Rewrites one start tag: wording in its attributes, and the page language on `<html>`.
fn rewrite_tag(tag: &str, f: &mut dyn FnMut(&str) -> Option<String>, lang_code: Option<&str>) -> String {
    let inner = tag.trim_start_matches('<').trim_end_matches('>').trim_end_matches('/');
    let name_end = inner.find(|c: char| c.is_whitespace()).unwrap_or(inner.len());
    let name = inner.get(..name_end).unwrap_or("").to_ascii_lowercase();
    if name.starts_with('/') || name.starts_with('!') || name_end >= inner.len() {
        return tag.to_string();
    }
    // Attributes with their value spans (relative to `tag`).
    struct Attr {
        name: String,
        value: String,
        span: Option<(usize, usize)>,
    }
    let mut attrs: Vec<Attr> = Vec::new();
    let base = tag.len() - tag.trim_start_matches('<').len() + name_end;
    let b = tag.as_bytes();
    let mut i = base;
    while i < b.len() {
        while b.get(i).is_some_and(|c| c.is_ascii_whitespace() || *c == b'/') {
            i += 1;
        }
        if i >= b.len() || b.get(i) == Some(&b'>') {
            break;
        }
        let s = i;
        while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && !matches!(*c, b'=' | b'>' | b'/')) {
            i += 1;
        }
        let attr_name = tag.get(s..i).unwrap_or("").to_ascii_lowercase();
        if b.get(i) == Some(&b'=') {
            i += 1;
            match b.get(i) {
                Some(&q @ (b'"' | b'\'')) => {
                    let vs = i + 1;
                    let ve = tag.get(vs..).and_then(|r| r.find(q as char)).map_or(b.len(), |k| vs + k);
                    attrs.push(Attr { name: attr_name, value: tag.get(vs..ve).unwrap_or("").to_string(), span: Some((vs, ve)) });
                    i = ve + 1;
                }
                _ => {
                    let vs = i;
                    while b.get(i).is_some_and(|c| !c.is_ascii_whitespace() && *c != b'>') {
                        i += 1;
                    }
                    attrs.push(Attr { name: attr_name, value: tag.get(vs..i).unwrap_or("").to_string(), span: None });
                }
            }
        } else {
            attrs.push(Attr { name: attr_name, value: String::new(), span: None });
        }
    }
    let plain: Vec<(String, String)> = attrs.iter().map(|a| (a.name.clone(), a.value.clone())).collect();
    let mut edits: Vec<(usize, usize, String)> = Vec::new();
    for a in &attrs {
        let Some((vs, ve)) = a.span else { continue };
        if name == "html" && a.name == "lang" {
            if let Some(code) = lang_code {
                edits.push((vs, ve, code.to_string()));
            }
        } else if reads_as_wording(&name, &a.name, &plain)
            && let Some(t) = f(&decode(&a.value))
        {
            edits.push((vs, ve, encode(&t)));
        }
    }
    if edits.is_empty() {
        return tag.to_string();
    }
    let mut out = String::with_capacity(tag.len() + 32);
    let mut at = 0usize;
    for (s, e, v) in edits {
        out.push_str(tag.get(at..s).unwrap_or(""));
        out.push_str(&v);
        at = e;
    }
    out.push_str(tag.get(at..).unwrap_or(""));
    out
}

/// Walks a page, handing each run of text (and each attribute that holds wording) to `f`; a
/// `Some` answer replaces it.
fn rewrite(html: &str, f: &mut dyn FnMut(&str) -> Option<String>, lang_code: Option<&str>) -> String {
    let mut out = String::with_capacity(html.len() + html.len() / 8);
    let mut i = 0usize;
    while i < html.len() {
        let next = html.get(i..).and_then(|r| r.find('<')).map_or(html.len(), |k| i + k);
        if next > i {
            let raw = html.get(i..next).unwrap_or("");
            match (!raw.trim().is_empty()).then(|| f(&decode(raw))).flatten() {
                Some(t) => out.push_str(&encode(&t)),
                None => out.push_str(raw),
            }
        }
        i = next;
        if i >= html.len() {
            break;
        }
        if html.get(i..).is_some_and(|r| r.starts_with("<!--")) {
            let end = html.get(i..).and_then(|r| r.find("-->")).map_or(html.len(), |k| i + k + 3);
            out.push_str(html.get(i..end).unwrap_or(""));
            i = end;
            continue;
        }
        let end = tag_end(html, i);
        let tag = html.get(i..end).unwrap_or("");
        out.push_str(&rewrite_tag(tag, f, lang_code));
        i = end;
        // The inside of these is not wording: scripts, styles, and text people typed into a box.
        let name: String = tag.trim_start_matches('<').chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if matches!(name.as_str(), "script" | "style" | "textarea" | "pre") && !tag.ends_with("/>") {
            let close = format!("</{name}");
            let lower = html.get(i..).unwrap_or("").to_ascii_lowercase();
            let stop = lower.find(&close).map_or(html.len(), |k| i + k);
            out.push_str(html.get(i..stop).unwrap_or(""));
            i = stop;
        }
    }
    out
}

/// Plain text (a PDF's wording) translated into `lang` with the page catalogs; text with nothing to translate comes back as it is.
pub fn translate_plain(text: &str, lang: Lang) -> String {
    if lang == Lang::En {
        return text.to_string();
    }
    translate_text(text, lang).unwrap_or_else(|| text.to_string())
}

/// A page translated into `lang`. English is returned as it is.
pub fn translate_html(html: &str, lang: Lang) -> String {
    if lang == Lang::En {
        return html.to_string();
    }
    rewrite(html, &mut |text| translate_text(text, lang), Some(lang.code()))
}

/// Every piece of wording on a page, as the catalogs key it (for building the list of what to translate).
pub fn page_wording(html: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    rewrite(
        html,
        &mut |text| {
            found.extend(wording(text));
            None
        },
        None,
    );
    found
}

/// The key lines of a catalog file: its English, tab-separated from the translation.
pub fn catalog_lines(text: &str) -> Vec<(&str, &str)> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).map(|l| l.split_once('\t').unwrap_or((l, ""))).collect()
}

/// The catalog file of a language (empty for English).
pub fn catalog_source(lang: Lang) -> &'static str {
    lang.catalog_text()
}

/// The English wording every catalog must cover.
pub fn required_wording() -> &'static str {
    include_str!("../i18n/en.txt")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    fn catalog_of(text: &str) -> Catalog {
        Catalog::parse(text)
    }

    /// Every catalog says every piece of wording the pages have, in a form that fits it.
    #[test]
    fn every_catalog_covers_all_the_wording() {
        let required: Vec<&str> = required_wording().lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect();
        assert!(required.len() > 1000, "the list of wording is made by `buildkiosk i18n-harvest`");
        let values = |s: &str| s.matches("{}").count();
        for lang in [Lang::Es, Lang::Fr, Lang::Nl] {
            let lines = catalog_lines(catalog_source(lang));
            let have: std::collections::HashMap<&str, &str> = lines.iter().copied().collect();
            assert_eq!(have.len(), lines.len(), "{}: a piece of wording is translated twice", lang.code());
            let missing: Vec<&&str> = required.iter().filter(|k| !have.contains_key(**k)).take(10).collect();
            assert!(
                missing.is_empty(),
                "{} lacks {} pieces of wording, such as {missing:?}",
                lang.code(),
                required.iter().filter(|k| !have.contains_key(**k)).count()
            );
            let extra: Vec<&&str> = have.keys().filter(|k| !required.contains(k)).take(10).collect();
            assert!(extra.is_empty(), "{} translates wording the pages no longer have: {extra:?}", lang.code());
            let mut same = 0usize;
            for (key, value) in &lines {
                assert!(!value.trim().is_empty(), "{}: `{key}` has no translation", lang.code());
                assert_eq!(value.trim(), *value, "{}: `{key}` has spaces around its translation", lang.code());
                // The values (`{}`) all come through: the same number, or numbered `{1}`, `{2}`.
                let n = values(key);
                let numbered = (1..=n).filter(|i| value.contains(&format!("{{{i}}}"))).count();
                let ok = values(value) == n && numbered == 0 || values(value) == 0 && numbered == n && n > 0 || n == 0 && !value.contains('{');
                assert!(ok, "{}: `{key}` → `{value}` doesn't keep its {n} value(s)", lang.code());
                same += usize::from(key == value);
            }
            assert!(same * 5 < lines.len(), "{}: {same} of {} are the same as the English", lang.code(), lines.len());
        }
    }

    #[test]
    fn languages_are_chosen_from_what_the_browser_asks_for() {
        assert_eq!(Lang::from_accept_language("nl-NL,nl;q=0.9,en;q=0.8"), Lang::Nl);
        assert_eq!(Lang::from_accept_language("de, fr;q=0.7, es;q=0.9"), Lang::Es);
        assert_eq!(Lang::from_accept_language("en-US,en;q=0.9"), Lang::En);
        assert_eq!(Lang::from_accept_language("fr;q=0"), Lang::En);
        assert_eq!(Lang::from_accept_language(""), Lang::En);
        assert_eq!(Lang::parse("ES"), Some(Lang::Es));
        assert_eq!(Lang::parse("fr_CA"), Some(Lang::Fr));
        assert_eq!(Lang::parse("klingon"), None);
    }

    #[test]
    fn values_inside_wording_are_captured_and_can_move() {
        let parts: Vec<String> = "{} of {} items done".split("{}").map(str::to_string).collect();
        assert_eq!(capture(&parts, "3 of 12 items done"), Some(vec!["3", "12"]));
        assert_eq!(capture(&parts, "of  items done"), None);
        assert_eq!(capture(&parts, "3 of 12 items"), None);
        let cat = catalog_of("{} of {} items done\t{2} elementos, {1} hechos\nPaid {}\tPagado el {}\nSave\tGuardar\n");
        assert_eq!(cat.lookup("3 of 12 items done").as_deref(), Some("12 elementos, 3 hechos"));
        assert_eq!(cat.lookup("Paid Oct 12, 2026").as_deref(), Some("Pagado el Oct 12, 2026"));
        assert_eq!(cat.lookup("Save").as_deref(), Some("Guardar"));
        assert_eq!(cat.lookup("Saved"), None);
        assert_eq!(cat.lookup("Paid"), None, "a value needs something in it");
    }

    #[test]
    fn dates_are_written_the_local_way() {
        assert_eq!(localize_dates("Due Oct 12, 2026 and Mar 3, 2027.", Lang::Fr), "Due 12 oct. 2026 and 3 mars 2027.");
        assert_eq!(localize_dates("Nov 2 to Nov 13", Lang::Es), "2 nov al 13 nov");
        assert_eq!(localize_dates("Oct 2026 →", Lang::Nl), "okt. 2026 →");
        assert_eq!(localize_dates("Fri Oct 16, 2026", Lang::Es), "vie 16 oct 2026");
        assert_eq!(localize_dates("Sat 2026-10-10", Lang::Fr), "sam. 2026-10-10");
        assert_eq!(localize_dates("Mon", Lang::Fr), "Mon");
        assert_eq!(localize_dates("Friday 3 items, March 5", Lang::Es), "Friday 3 items, March 5");
        assert_eq!(localize_dates("Oct 12", Lang::Es), "12 oct");
        assert_eq!(localize_dates("Oct 12, 2026", Lang::Nl), "12 okt. 2026");
        assert_eq!(localize_dates("Octopus 12, 2026", Lang::Es), "Octopus 12, 2026");
        assert_eq!(localize_dates("Oct 12, 20266", Lang::Es), "Oct 12, 20266");
        assert_eq!(localize_dates("Oct 12, 2026", Lang::En), "Oct 12, 2026");
    }

    #[test]
    fn text_is_cut_at_the_separators_and_put_back() {
        let pieces = split_pieces("Plan #4: Closeout · 3 of 7 items done");
        assert_eq!(pieces, [("Plan #4", ": "), ("Closeout", " · "), ("3 of 7 items done", "")]);
        assert_eq!(wording("  Save   changes \n"), ["Save changes"]);
        assert!(wording("12 / 34").is_empty());
    }

    #[test]
    fn a_page_is_rewritten_in_place() {
        // Uses the real Spanish catalog for words every catalog has.
        let html = r#"<!DOCTYPE html><html lang="en"><head><title>Projects · BuildKiosk</title></head><body><a href="/">Projects</a><input type="submit" value="Save"><input type="text" value="Save" placeholder="Search"><textarea>Projects</textarea><script>var a = "Projects";</script><p>Ünïcode &amp; Ada</p></body></html>"#;
        let out = translate_html(html, Lang::Es);
        assert!(out.contains(r#"<html lang="es">"#), "{out}");
        assert!(out.contains("<textarea>Projects</textarea>") && out.contains(r#"var a = "Projects";"#), "{out}");
        assert!(out.contains("<p>Ünïcode &amp; Ada</p>"), "{out}");
        assert!(out.contains(r#"<input type="text" value="Save""#), "a text box's value is what someone typed: {out}");
        assert!(!out.contains(">Projects</a>") && !out.contains(r#"type="submit" value="Save""#), "{out}");
        assert_eq!(translate_html(html, Lang::En), html);
    }
}
