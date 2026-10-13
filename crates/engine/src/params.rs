//! Reading command parameters.
//!
//! Parameters come from forms, the HTTP API, MCP clients and the CLI: all untrusted. [`Params`]
//! reads them leniently in form (a number may arrive as `12` or `"12"`, a flag as `true` or `"on"`)
//! and strictly in content (lengths, ranges, dates and amounts are checked), and every failure is a
//! [`EngineError::BadParams`] naming the parameter.

use std::borrow::Cow;
use std::fmt::Display;
use std::ops::RangeInclusive;

use buildkiosk_core::{Date, Id, Money, Percent, text};
use buildkiosk_model::Choice;
use serde_json::{Map, Value};

use crate::EngineError;

/// The most ids a list parameter may hold.
pub const MAX_IDS: usize = 1000;
/// The most line items an array parameter may hold.
pub const MAX_ITEMS: usize = 5000;

/// A command's parameters (a JSON object).
#[derive(Clone, Copy)]
pub struct Params<'a> {
    obj: Option<&'a Map<String, Value>>,
    cmd: &'static str,
}

impl<'a> Params<'a> {
    pub fn new(v: &'a Value, cmd: &'static str) -> Params<'a> {
        Params { obj: v.as_object(), cmd }
    }

    pub fn cmd(&self) -> &'static str {
        self.cmd
    }

    /// The error for a bad parameter.
    pub fn bad(&self, key: &str, msg: impl Display) -> EngineError {
        EngineError::BadParams { cmd: self.cmd.to_string(), msg: format!("`{key}` {msg}") }
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.obj.and_then(|o| o.get(key)).filter(|v| !v.is_null())
    }

    /// The key was given, even as `null` or `""` (to clear a field in an update).
    pub fn present(&self, key: &str) -> bool {
        self.obj.is_some_and(|o| o.contains_key(key))
    }

    /// The parameter names given.
    pub fn keys(&self) -> impl Iterator<Item = &'a str> {
        self.obj.into_iter().flat_map(|o| o.keys().map(String::as_str))
    }

    fn text_of(&self, key: &str) -> Result<Option<Cow<'a, str>>, EngineError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(Cow::Borrowed(s.as_str()))),
            Some(Value::Number(n)) => Ok(Some(Cow::Owned(n.to_string()))),
            Some(Value::Bool(b)) => Ok(Some(Cow::Owned(b.to_string()))),
            Some(_) => Err(self.bad(key, "must be text")),
        }
    }

    /// One line of text, cleaned and limited; `None` when absent or blank.
    pub fn line(&self, key: &str, max: usize) -> Result<Option<String>, EngineError> {
        let Some(s) = self.text_of(key)? else { return Ok(None) };
        let l = text::line(&s, max).map_err(|e| self.bad(key, e))?;
        Ok((!l.is_empty()).then_some(l))
    }

    /// A required line of text.
    pub fn req_line(&self, key: &str, max: usize) -> Result<String, EngineError> {
        self.line(key, max)?.ok_or_else(|| self.bad(key, "is required"))
    }

    /// One line of text, `""` when absent or blank (for optional text fields).
    pub fn line_or_empty(&self, key: &str, max: usize) -> Result<String, EngineError> {
        Ok(self.line(key, max)?.unwrap_or_default())
    }

    /// Multi-line text, cleaned and limited; `None` when absent or blank.
    pub fn text(&self, key: &str, max: usize) -> Result<Option<String>, EngineError> {
        let Some(s) = self.text_of(key)? else { return Ok(None) };
        let t = text::multiline(&s, max).map_err(|e| self.bad(key, e))?;
        Ok((!t.is_empty()).then_some(t))
    }

    pub fn req_text(&self, key: &str, max: usize) -> Result<String, EngineError> {
        self.text(key, max)?.ok_or_else(|| self.bad(key, "is required"))
    }

    pub fn text_or_empty(&self, key: &str, max: usize) -> Result<String, EngineError> {
        Ok(self.text(key, max)?.unwrap_or_default())
    }

    /// An email address, lowercased.
    pub fn email(&self, key: &str) -> Result<Option<String>, EngineError> {
        let Some(s) = self.line(key, text::EMAIL)? else { return Ok(None) };
        text::email(&s).map(Some).ok_or_else(|| self.bad(key, "is not a valid email address"))
    }

    fn id_of<T>(&self, key: &str, v: &Value) -> Result<Option<Id<T>>, EngineError> {
        match v {
            Value::Number(n) => n.as_u64().filter(|n| *n > 0).map(Id::new).map(Some).ok_or_else(|| self.bad(key, "must be a positive id")),
            Value::String(s) if s.trim().is_empty() => Ok(None),
            Value::String(s) => Id::parse(s.trim()).map(Some).ok_or_else(|| self.bad(key, format!("`{s}` is not an id"))),
            Value::Null => Ok(None),
            _ => Err(self.bad(key, "must be an id")),
        }
    }

    /// An optional id; `None` when absent, null or blank.
    pub fn opt_id<T>(&self, key: &str) -> Result<Option<Id<T>>, EngineError> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => self.id_of(key, v),
        }
    }

    pub fn id<T>(&self, key: &str) -> Result<Id<T>, EngineError> {
        self.opt_id(key)?.ok_or_else(|| self.bad(key, "is required"))
    }

    /// A list of ids: a JSON array of ids, or text with ids separated by commas. Duplicates are
    /// dropped (first occurrence kept). Empty when absent.
    pub fn ids<T>(&self, key: &str) -> Result<Vec<Id<T>>, EngineError> {
        let mut out: Vec<Id<T>> = Vec::new();
        let mut push = |id: Option<Id<T>>| -> Result<(), EngineError> {
            if let Some(id) = id
                && !out.contains(&id)
            {
                if out.len() >= MAX_IDS {
                    return Err(self.bad(key, format!("holds more than {MAX_IDS} ids")));
                }
                out.push(id);
            }
            Ok(())
        };
        match self.get(key) {
            None => {}
            Some(Value::Array(items)) => {
                if items.len() > MAX_IDS * 2 {
                    return Err(self.bad(key, format!("holds more than {MAX_IDS} ids")));
                }
                for v in items {
                    push(self.id_of(key, v)?)?;
                }
            }
            Some(Value::String(s)) => {
                for part in s.split(',').take(MAX_IDS * 2) {
                    push(self.id_of(key, &Value::String(part.trim().to_string()))?)?;
                }
            }
            Some(v) => push(self.id_of(key, v)?)?,
        }
        Ok(out)
    }

    /// A list of short words: an array of text, or one text separated by commas. Blank items are dropped and
    /// no more than `max` are read.
    pub fn words(&self, key: &str, max: usize) -> Result<Vec<String>, EngineError> {
        let mut out = Vec::new();
        let mut push = |s: &str| {
            let s = s.trim();
            if !s.is_empty() && s.len() <= 80 && out.len() < max {
                out.push(s.to_string());
            }
        };
        match self.get(key) {
            None => {}
            Some(Value::Array(items)) => {
                for v in items.iter().take(max.saturating_mul(2)) {
                    match v {
                        Value::String(s) => push(s),
                        _ => return Err(self.bad(key, "must be a list of text")),
                    }
                }
            }
            Some(Value::String(s)) => s.split(',').take(max.saturating_mul(2)).for_each(&mut push),
            Some(_) => return Err(self.bad(key, "must be a list of text")),
        }
        Ok(out)
    }

    /// A date as `YYYY-MM-DD`; `None` when absent or blank.
    pub fn date(&self, key: &str) -> Result<Option<Date>, EngineError> {
        let Some(s) = self.line(key, 40)? else { return Ok(None) };
        Date::parse(&s).map(Some).ok_or_else(|| self.bad(key, format!("`{s}` is not a date (YYYY-MM-DD)")))
    }

    pub fn req_date(&self, key: &str) -> Result<Date, EngineError> {
        self.date(key)?.ok_or_else(|| self.bad(key, "is required"))
    }

    /// An amount ("1,234.56" or a number); `None` when absent or blank.
    pub fn money(&self, key: &str) -> Result<Option<Money>, EngineError> {
        let Some(s) = self.line(key, 40)? else { return Ok(None) };
        Money::parse(&s).map(Some).map_err(|e| self.bad(key, e))
    }

    pub fn money_or_zero(&self, key: &str) -> Result<Money, EngineError> {
        Ok(self.money(key)?.unwrap_or(Money::ZERO))
    }

    /// A percentage ("10", "12.5%"); `None` when absent or blank.
    pub fn percent(&self, key: &str) -> Result<Option<Percent>, EngineError> {
        let Some(s) = self.line(key, 40)? else { return Ok(None) };
        Percent::parse(&s).map(Some).map_err(|e| self.bad(key, e))
    }

    /// A flag: `true`/`false`, or text such as "on", "yes", "1", "off", "no", "0".
    pub fn flag(&self, key: &str) -> Result<Option<bool>, EngineError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(Value::Number(n)) => match n.as_u64() {
                Some(0) => Ok(Some(false)),
                Some(1) => Ok(Some(true)),
                _ => Err(self.bad(key, "must be true or false")),
            },
            Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "on" | "yes" | "1" | "y" => Ok(Some(true)),
                "false" | "off" | "no" | "0" | "n" => Ok(Some(false)),
                "" => Ok(None),
                _ => Err(self.bad(key, "must be true or false")),
            },
            Some(_) => Err(self.bad(key, "must be true or false")),
        }
    }

    pub fn flag_or(&self, key: &str, default: bool) -> Result<bool, EngineError> {
        Ok(self.flag(key)?.unwrap_or(default))
    }

    /// A whole number within `range`; `None` when absent or blank.
    pub fn int(&self, key: &str, range: RangeInclusive<i64>) -> Result<Option<i64>, EngineError> {
        let n = match self.get(key) {
            None => return Ok(None),
            Some(Value::Number(n)) => n.as_i64().ok_or_else(|| self.bad(key, "must be a whole number"))?,
            Some(Value::String(s)) if s.trim().is_empty() => return Ok(None),
            Some(Value::String(s)) => s.trim().parse::<i64>().map_err(|_| self.bad(key, format!("`{s}` is not a whole number")))?,
            Some(_) => return Err(self.bad(key, "must be a whole number")),
        };
        if range.contains(&n) { Ok(Some(n)) } else { Err(self.bad(key, format!("must be between {} and {}", range.start(), range.end()))) }
    }

    /// One of a [`Choice`]'s keys or labels; `None` when absent or blank.
    pub fn choice<C: Choice>(&self, key: &str) -> Result<Option<C>, EngineError> {
        let Some(s) = self.line(key, 100)? else { return Ok(None) };
        C::parse(&s).map(Some).ok_or_else(|| {
            let keys: Vec<&str> = C::ALL.iter().map(|c| c.key()).collect();
            self.bad(key, format!("`{s}` is not one of: {}", keys.join(", ")))
        })
    }

    /// A JSON object parameter (maps such as `{tool: level}`); `None` when absent.
    pub fn object(&self, key: &str) -> Option<&'a Map<String, Value>> {
        self.get(key).and_then(Value::as_object)
    }

    /// An array of objects (line items), each read as its own [`Params`]. Empty when absent.
    pub fn items(&self, key: &str) -> Result<Vec<Params<'a>>, EngineError> {
        match self.get(key) {
            None => Ok(Vec::new()),
            Some(Value::Array(items)) => {
                if items.len() > MAX_ITEMS {
                    return Err(self.bad(key, format!("holds more than {MAX_ITEMS} items")));
                }
                items
                    .iter()
                    .map(|v| if v.is_object() { Ok(Params::new(v, self.cmd)) } else { Err(self.bad(key, "must be a list of objects")) })
                    .collect()
            }
            Some(_) => Err(self.bad(key, "must be a list of objects")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buildkiosk_model::Priority;
    use serde_json::json;

    struct Thing;

    #[test]
    fn reads_lenient_forms() {
        let v = json!({"a": "  hi  ", "n": "12", "m": 12, "f": "on", "g": 0, "d": "2026-10-12", "ids": "3, 4,3", "arr": [1, "2"], "money": "1,234.5", "p": "Urgent", "blank": "  "});
        let p = Params::new(&v, "test.cmd");
        assert_eq!(p.req_line("a", 10).unwrap(), "hi");
        assert_eq!(p.id::<Thing>("n").unwrap(), Id::new(12));
        assert_eq!(p.id::<Thing>("m").unwrap(), Id::new(12));
        assert_eq!(p.flag("f").unwrap(), Some(true));
        assert_eq!(p.flag("g").unwrap(), Some(false));
        assert_eq!(p.date("d").unwrap(), Date::parse("2026-10-12"));
        assert_eq!(p.ids::<Thing>("ids").unwrap(), vec![Id::new(3), Id::new(4)]);
        assert_eq!(p.ids::<Thing>("arr").unwrap(), vec![Id::new(1), Id::new(2)]);
        assert_eq!(p.money("money").unwrap(), Some(Money::from_cents(123_450)));
        assert_eq!(p.choice::<Priority>("p").unwrap(), Some(Priority::Urgent));
        assert_eq!(p.line("blank", 10).unwrap(), None);
        assert_eq!(p.line("missing", 10).unwrap(), None);
        assert!(p.present("blank") && !p.present("missing"));
        assert_eq!(p.int("n", 0..=20).unwrap(), Some(12));
    }

    #[test]
    fn rejects_bad_values_with_the_key_name() {
        let v = json!({"a": "x".repeat(20), "n": "12x", "neg": -1, "d": "12/10/2026", "o": {}, "p": "nope", "f": "maybe"});
        let p = Params::new(&v, "test.cmd");
        let e = p.req_line("a", 10).unwrap_err().to_string();
        assert!(e.contains("`a`") && e.contains("test.cmd"), "{e}");
        assert!(p.id::<Thing>("n").is_err());
        assert!(p.id::<Thing>("neg").is_err());
        assert!(p.date("d").is_err());
        assert!(p.req_line("o", 10).is_err());
        assert!(p.choice::<Priority>("p").is_err());
        assert!(p.flag("f").is_err());
        assert!(p.id::<Thing>("missing").is_err());
        assert!(p.int("n", 0..=5).is_err());
    }

    #[test]
    fn non_object_params_read_as_empty() {
        let v = json!([1, 2]);
        let p = Params::new(&v, "test.cmd");
        assert_eq!(p.line("a", 5).unwrap(), None);
        assert!(!p.present("a"));
    }
}
