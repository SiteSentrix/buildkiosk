//! The wording the browser viewer shows itself (hints and notes under the sheet), in the page's language.
//!
//! The server translates the page's own text; what the viewer writes into the page at run time comes from here. The
//! language is the page's (`<html lang>`); English, or a language with no file, shows the English the code has.

use std::collections::HashMap;

const ES: &str = include_str!("../i18n/es.txt");
const FR: &str = include_str!("../i18n/fr.txt");
const NL: &str = include_str!("../i18n/nl.txt");

/// English wording and how a language says it.
#[derive(Default)]
pub struct Words {
    map: HashMap<String, String>,
}

/// The lines of a catalog: English, a tab, the translation; `#` lines and blank lines are skipped.
pub fn lines(text: &str) -> Vec<(&str, &str)> {
    text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('\t')).collect()
}

/// The catalog of a language code (`es`, `fr`, `nl`); empty for anything else.
pub fn source(lang: &str) -> &'static str {
    match lang.get(..2).unwrap_or("") {
        "es" => ES,
        "fr" => FR,
        "nl" => NL,
        _ => "",
    }
}

impl Words {
    pub fn for_language(lang: &str) -> Words {
        Words { map: lines(source(lang)).into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    /// The text in the language, or as it is when there is no translation (a message the server sent, say).
    pub fn t(&self, english: &str) -> String {
        self.map.get(english).cloned().unwrap_or_else(|| english.to_string())
    }

    /// Wording with values: each `{}` takes the next value, `{2}` the second.
    pub fn tf(&self, english: &str, values: &[&str]) -> String {
        let text = self.map.get(english).map_or(english, String::as_str);
        let mut out = String::with_capacity(text.len() + 16);
        let (mut rest, mut next) = (text, 0usize);
        while let Some(at) = rest.find('{') {
            let Some(close) = rest.get(at..).and_then(|r| r.find('}')).map(|c| at + c) else { break };
            let inner = rest.get(at + 1..close).unwrap_or("");
            let index = if inner.is_empty() {
                next += 1;
                Some(next - 1)
            } else {
                inner.parse::<usize>().ok().and_then(|n| n.checked_sub(1))
            };
            out.push_str(rest.get(..at).unwrap_or(""));
            match index.and_then(|i| values.get(i)) {
                Some(v) => out.push_str(v),
                None => out.push_str(rest.get(at..=close).unwrap_or("")),
            }
            rest = rest.get(close + 1..).unwrap_or("");
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn words_come_out_in_the_language_or_as_they_are() {
        let es = Words::for_language("es-MX");
        assert_eq!(es.t("Loading…"), "Cargando…");
        assert_eq!(es.t("Something the server said"), "Something the server said");
        assert_eq!(Words::for_language("en").t("Loading…"), "Loading…");
        assert_eq!(Words::for_language("").t("Loading…"), "Loading…");
        assert_eq!(
            es.tf("Scale set from a stretch of {}. Lengths and areas drawn on this page use it.", &["12.50 ft"]),
            "Escala fijada a partir de un tramo de 12.50 ft. Las longitudes y áreas dibujadas en esta página la usan."
        );
        assert_eq!(Words::default().tf("{2} then {1}", &["a", "b"]), "b then a");
        assert_eq!(Words::default().tf("no values {} here", &[]), "no values {} here");
    }

    /// The three languages say the same things, keep their values, and every English line is still in the viewer's code.
    #[test]
    fn the_catalogs_agree_with_each_other_and_with_the_code() {
        let code = [include_str!("dom.rs"), include_str!("gantt.rs")].concat();
        let mut keys: Vec<Vec<&str>> = Vec::new();
        for text in [ES, FR, NL] {
            let rows = lines(text);
            assert!(rows.len() > 20);
            let mut seen = std::collections::HashSet::new();
            for (k, v) in &rows {
                assert!(seen.insert(*k), "`{k}` twice");
                assert!(!v.trim().is_empty(), "`{k}` has no translation");
                assert_eq!(k.matches("{}").count(), v.matches("{}").count(), "`{k}` → `{v}` doesn't keep its values");
                assert!(code.contains(k), "`{k}` is no longer in the viewer");
            }
            keys.push(rows.iter().map(|(k, _)| *k).collect());
        }
        assert!(keys.windows(2).all(|w| w[0] == w[1]), "the languages translate different lines");
    }
}
