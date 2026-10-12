//! Typed record ids.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::marker::PhantomData;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A record id typed by the record it names, so an `Id<Rfi>` can't be passed where an
/// `Id<Project>` is expected. Ids are allocated per company and per table, start at 1 and are never
/// reused. They serialize as plain numbers.
pub struct Id<T> {
    raw: u64,
    kind: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// The id with this number. Callers that take ids from input use [`Id::parse`] or check that the
    /// record exists; a made-up id simply names nothing.
    pub const fn new(raw: u64) -> Self {
        Self { raw, kind: PhantomData }
    }

    pub const fn raw(self) -> u64 {
        self.raw
    }

    /// The same number as another kind's id (for generic references such as related items).
    pub const fn cast<U>(self) -> Id<U> {
        Id::new(self.raw)
    }

    /// Parses a decimal id ("12"). Rejects 0, signs, spaces, non-digits and overflow.
    pub fn parse(s: &str) -> Option<Self> {
        if s.is_empty() || s.len() > 20 || !s.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        match s.parse::<u64>() {
            Ok(0) | Err(_) => None,
            Ok(n) => Some(Self::new(n)),
        }
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.raw.cmp(&other.raw)
    }
}

impl<T> Hash for Id<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.raw.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.raw)
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.raw)
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(self.raw)
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        u64::deserialize(d).map(Id::new)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Thing;

    #[test]
    fn parse_accepts_only_positive_decimal() {
        assert_eq!(Id::<Thing>::parse("12"), Some(Id::new(12)));
        for bad in ["", "0", "-1", "+1", " 1", "1 ", "1.0", "x", "99999999999999999999999"] {
            assert_eq!(Id::<Thing>::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn serializes_as_a_number() {
        let id: Id<Thing> = Id::new(7);
        assert_eq!(serde_json::to_string(&id).unwrap(), "7");
        let back: Id<Thing> = serde_json::from_str("7").unwrap();
        assert_eq!(back, id);
        assert_eq!(format!("{id} {id:?}"), "7 #7");
    }
}
