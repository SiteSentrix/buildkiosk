//! Money and percentages.
//!
//! Amounts are whole cents in an `i64` and percentages are hundredths of a percent, so financial
//! math never touches floating point. Every operation is checked: an overflow is a `None` or an
//! error, never a wrap or a panic.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Why a money or percentage string was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MoneyError {
    Empty,
    Invalid,
    TooManyDecimals,
    TooLarge,
}

impl fmt::Display for MoneyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            MoneyError::Empty => "no amount given",
            MoneyError::Invalid => "not a number (use digits, an optional sign, commas and a decimal point, e.g. 1,234.50)",
            MoneyError::TooManyDecimals => "at most two decimal places",
            MoneyError::TooLarge => "the amount is too large",
        })
    }
}

impl std::error::Error for MoneyError {}

/// An amount in cents of the company's currency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Money(i64);

impl Money {
    pub const ZERO: Money = Money(0);

    /// The largest magnitude accepted from input (ten trillion), far enough below `i64::MAX` that
    /// sums of thousands of line items can't overflow.
    pub const MAX_INPUT_CENTS: i64 = 1_000_000_000_000_000;

    pub const fn from_cents(cents: i64) -> Self {
        Money(cents)
    }

    /// Whole currency units (dollars), or `None` on overflow.
    pub const fn from_units(units: i64) -> Option<Self> {
        match units.checked_mul(100) {
            Some(c) => Some(Money(c)),
            None => None,
        }
    }

    pub const fn cents(self) -> i64 {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub fn checked_add(self, other: Money) -> Option<Money> {
        self.0.checked_add(other.0).map(Money)
    }

    pub fn checked_sub(self, other: Money) -> Option<Money> {
        self.0.checked_sub(other.0).map(Money)
    }

    pub fn checked_neg(self) -> Option<Money> {
        self.0.checked_neg().map(Money)
    }

    pub fn checked_mul_int(self, n: i64) -> Option<Money> {
        self.0.checked_mul(n).map(Money)
    }

    /// The sum of `items`, or `None` if it overflows.
    pub fn sum<I: IntoIterator<Item = Money>>(items: I) -> Option<Money> {
        items.into_iter().try_fold(Money::ZERO, Money::checked_add)
    }

    /// `self × p`, rounded half away from zero to the cent (retainage, markups, % of a value).
    pub fn percent(self, p: Percent) -> Option<Money> {
        // i64 × i64 always fits in i128.
        let product = i128::from(self.0) * i128::from(p.0);
        i64::try_from(div_round(product, 10_000)).ok().map(Money)
    }

    /// `self / of` as a percentage (e.g. percent complete), or `None` when `of` is zero.
    pub fn ratio(self, of: Money) -> Option<Percent> {
        if of.0 == 0 {
            return None;
        }
        let scaled = i128::from(self.0) * 10_000;
        let q = div_round(scaled, i128::from(of.0));
        i64::try_from(q).ok().map(Percent)
    }

    /// Parses "1234", "1,234.5", "$1,234.56", "-12.30", "(12.30)" (negative). At most two decimals
    /// and [`Money::MAX_INPUT_CENTS`].
    pub fn parse(input: &str) -> Result<Money, MoneyError> {
        let (negative, digits) = strip_sign(input, true)?;
        let cents = parse_fixed(digits, 15)?;
        if cents > Money::MAX_INPUT_CENTS {
            return Err(MoneyError::TooLarge);
        }
        Ok(Money(if negative { -cents } else { cents }))
    }

    /// Without thousands separators, for forms and JSON: "-1234.56".
    pub fn plain(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        format!("{sign}{}.{:02}", abs / 100, abs % 100)
    }
}

impl fmt::Display for Money {
    /// With thousands separators: "-1,234.56".
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        write!(f, "{sign}{}.{:02}", group_thousands(&(abs / 100).to_string()), abs % 100)
    }
}

impl Serialize for Money {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.plain())
    }
}

impl<'de> Deserialize<'de> for Money {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(DecimalVisitor { what: "an amount such as \"1234.56\"" }).and_then(|s| Money::parse(&s).map_err(de::Error::custom))
    }
}

/// A percentage in hundredths of a percent: 10% is 1000, 37.25% is 3725.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Percent(i64);

impl Percent {
    pub const ZERO: Percent = Percent(0);
    pub const HUNDRED: Percent = Percent(10_000);
    /// The largest magnitude accepted from input (10,000%).
    pub const MAX_INPUT: i64 = 1_000_000;

    pub const fn from_hundredths(h: i64) -> Self {
        Percent(h)
    }

    pub const fn hundredths(self) -> i64 {
        self.0
    }

    pub const fn from_whole(p: i64) -> Option<Self> {
        match p.checked_mul(100) {
            Some(h) => Some(Percent(h)),
            None => None,
        }
    }

    /// Limited to 0–100% (percent complete).
    pub fn clamp_0_100(self) -> Self {
        Percent(self.0.clamp(0, 10_000))
    }

    /// Parses "10", "10%", "37.25 %", "-2.5". At most two decimals.
    pub fn parse(input: &str) -> Result<Percent, MoneyError> {
        let trimmed = input.trim();
        let trimmed = trimmed.strip_suffix('%').unwrap_or(trimmed);
        let (negative, digits) = strip_sign(trimmed, false)?;
        let h = parse_fixed(digits, 7)?;
        if h > Percent::MAX_INPUT {
            return Err(MoneyError::TooLarge);
        }
        Ok(Percent(if negative { -h } else { h }))
    }

    /// "37.25", "10", "-2.5" (no % sign), for forms and JSON.
    pub fn plain(self) -> String {
        let sign = if self.0 < 0 { "-" } else { "" };
        let abs = self.0.unsigned_abs();
        let (whole, frac) = (abs / 100, abs % 100);
        match frac {
            0 => format!("{sign}{whole}"),
            f if f.is_multiple_of(10) => format!("{sign}{whole}.{}", f / 10),
            f => format!("{sign}{whole}.{f:02}"),
        }
    }
}

impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", self.plain())
    }
}

impl Serialize for Percent {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.plain())
    }
}

impl<'de> Deserialize<'de> for Percent {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(DecimalVisitor { what: "a percentage such as \"10\" or \"37.5\"" })
            .and_then(|s| Percent::parse(&s).map_err(de::Error::custom))
    }
}

/// Accepts a string or a JSON number and hands back its text, so that decimals are parsed exactly.
struct DecimalVisitor {
    what: &'static str,
}

impl Visitor<'_> for DecimalVisitor {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.what)
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
        Ok(v.to_string())
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
        Ok(v.to_string())
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
        Ok(v.to_string())
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<String, E> {
        if v.is_finite() { Ok(format!("{v}")) } else { Err(E::custom("not a finite number")) }
    }
}

/// Splits off a sign: a leading `-` or `+`, or (for money) wrapping parentheses, and an optional `$`.
fn strip_sign(input: &str, money: bool) -> Result<(bool, &str), MoneyError> {
    let mut s = input.trim();
    if s.is_empty() {
        return Err(MoneyError::Empty);
    }
    let mut negative = false;
    if money && let Some(inner) = s.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
        negative = true;
        s = inner.trim();
    }
    for _ in 0..2 {
        if let Some(rest) = s.strip_prefix('-') {
            if negative {
                return Err(MoneyError::Invalid);
            }
            negative = true;
            s = rest.trim_start();
        } else if let Some(rest) = s.strip_prefix('+') {
            s = rest.trim_start();
        }
        if money && let Some(rest) = s.strip_prefix('$') {
            s = rest.trim_start();
        }
    }
    if s.is_empty() {
        return Err(MoneyError::Invalid);
    }
    Ok((negative, s))
}

/// Parses an unsigned decimal with at most two decimals into hundredths: "1,234.5" → 123450.
/// Commas may separate digits in the integer part. At most `max_int_digits` integer digits.
fn parse_fixed(s: &str, max_int_digits: usize) -> Result<i64, MoneyError> {
    let (int_part, frac_part) = match s.split_once('.') {
        Some((i, f)) => (i, f),
        None => (s, ""),
    };
    if int_part.is_empty() && frac_part.is_empty() {
        return Err(MoneyError::Invalid);
    }
    let mut value: i64 = 0;
    let mut digits = 0usize;
    let mut prev_comma = false;
    for (i, c) in int_part.chars().enumerate() {
        if c == ',' {
            if i == 0 || prev_comma {
                return Err(MoneyError::Invalid);
            }
            prev_comma = true;
            continue;
        }
        let d = c.to_digit(10).ok_or(MoneyError::Invalid)?;
        prev_comma = false;
        digits += 1;
        if digits > max_int_digits {
            return Err(MoneyError::TooLarge);
        }
        value = value.checked_mul(10).and_then(|v| v.checked_add(i64::from(d))).ok_or(MoneyError::TooLarge)?;
    }
    if prev_comma {
        return Err(MoneyError::Invalid);
    }
    let mut frac_chars = frac_part.chars();
    let tenths = frac_chars.next().map(|c| c.to_digit(10).ok_or(MoneyError::Invalid)).transpose()?.unwrap_or(0);
    let hundredths = frac_chars.next().map(|c| c.to_digit(10).ok_or(MoneyError::Invalid)).transpose()?.unwrap_or(0);
    if let Some(c) = frac_chars.next() {
        return Err(if c.is_ascii_digit() { MoneyError::TooManyDecimals } else { MoneyError::Invalid });
    }
    value.checked_mul(100).and_then(|v| v.checked_add(i64::from(tenths * 10 + hundredths))).ok_or(MoneyError::TooLarge)
}

/// `n / d` rounded half away from zero; `d` must not be zero (callers check).
fn div_round(n: i128, d: i128) -> i128 {
    if d == 0 {
        return 0;
    }
    let q = n / d;
    let r = n % d;
    if r.unsigned_abs() * 2 >= d.unsigned_abs() { q + (n.signum() * d.signum()) } else { q }
}

/// "1234567" → "1,234,567".
fn group_thousands(digits: &str) -> String {
    let n = digits.chars().count();
    let mut out = String::with_capacity(n + n / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (n - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parses_common_forms() {
        let cases = [
            ("0", 0),
            ("12", 1200),
            ("12.5", 1250),
            ("12.05", 1205),
            (".5", 50),
            ("1,234.56", 123_456),
            ("$1,234.56", 123_456),
            ("-$5", -500),
            ("$-5", -500),
            ("(12.30)", -1230),
            ("+7", 700),
            ("  42.10  ", 4210),
        ];
        for (s, cents) in cases {
            assert_eq!(Money::parse(s), Ok(Money::from_cents(cents)), "{s:?}");
        }
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(Money::parse(""), Err(MoneyError::Empty));
        assert_eq!(Money::parse("1.234"), Err(MoneyError::TooManyDecimals));
        assert_eq!(Money::parse("99999999999999999"), Err(MoneyError::TooLarge));
        for bad in ["abc", ",1", "1,", "1,,0", "--1", "(-1)", "1.2.3", "$", "-", "1e5", "1 000"] {
            assert!(Money::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn displays_with_grouping_and_sign() {
        assert_eq!(Money::from_cents(123_456_789).to_string(), "1,234,567.89");
        assert_eq!(Money::from_cents(-5).to_string(), "-0.05");
        assert_eq!(Money::from_cents(-123_456).plain(), "-1234.56");
        assert_eq!(Money::from_cents(i64::MIN).plain(), "-92233720368547758.08");
    }

    #[test]
    fn percent_of_money_rounds_half_away_from_zero() {
        let ten = Percent::parse("10").unwrap();
        assert_eq!(Money::from_cents(1005).percent(ten), Some(Money::from_cents(101)));
        assert_eq!(Money::from_cents(-1005).percent(ten), Some(Money::from_cents(-101)));
        assert_eq!(Money::from_cents(1004).percent(ten), Some(Money::from_cents(100)));
        assert_eq!(Money::from_cents(i64::MAX).percent(Percent::from_hundredths(20_000)), None);
        assert_eq!(Money::from_cents(250).ratio(Money::from_cents(1000)), Some(Percent::from_hundredths(2500)));
        assert_eq!(Money::from_cents(1).ratio(Money::ZERO), None);
    }

    #[test]
    fn percent_parse_and_display() {
        assert_eq!(Percent::parse("37.25%"), Ok(Percent::from_hundredths(3725)));
        assert_eq!(Percent::parse(" 10 % "), Ok(Percent::from_hundredths(1000)));
        assert_eq!(Percent::parse("-2.5"), Ok(Percent::from_hundredths(-250)));
        assert_eq!(Percent::from_hundredths(3750).to_string(), "37.5%");
        assert_eq!(Percent::from_hundredths(1000).plain(), "10");
        assert_eq!(Percent::from_hundredths(-1).plain(), "-0.01");
        assert!(Percent::parse("(10)").is_err());
        assert!(Percent::parse("10.001").is_err());
    }

    #[test]
    fn serde_uses_exact_decimal_strings() {
        let m = Money::from_cents(-123_456);
        assert_eq!(serde_json::to_string(&m).unwrap(), "\"-1234.56\"");
        assert_eq!(serde_json::from_str::<Money>("\"-1234.56\"").unwrap(), m);
        assert_eq!(serde_json::from_str::<Money>("1500").unwrap(), Money::from_cents(150_000));
        assert_eq!(serde_json::from_str::<Money>("12.5").unwrap(), Money::from_cents(1250));
        assert!(serde_json::from_str::<Money>("0.1234").is_err());
        assert_eq!(serde_json::from_str::<Percent>("\"12.5\"").unwrap(), Percent::from_hundredths(1250));
    }

    #[test]
    fn sums_are_checked() {
        assert_eq!(Money::sum([Money::from_cents(1), Money::from_cents(2)]), Some(Money::from_cents(3)));
        assert_eq!(Money::sum([Money::from_cents(i64::MAX), Money::from_cents(1)]), None);
    }

    proptest! {
        #[test]
        fn plain_and_display_round_trip(c in -Money::MAX_INPUT_CENTS..=Money::MAX_INPUT_CENTS) {
            let m = Money::from_cents(c);
            prop_assert_eq!(Money::parse(&m.plain()), Ok(m));
            prop_assert_eq!(Money::parse(&m.to_string()), Ok(m));
        }

        #[test]
        fn percent_round_trips(h in -Percent::MAX_INPUT..=Percent::MAX_INPUT) {
            let p = Percent::from_hundredths(h);
            prop_assert_eq!(Percent::parse(&p.to_string()), Ok(p));
        }

        #[test]
        fn parse_never_panics(s in "\\PC{0,40}") {
            let _ = Money::parse(&s);
            let _ = Percent::parse(&s);
        }

        #[test]
        fn percent_never_panics(c in any::<i64>(), h in any::<i64>()) {
            let _ = Money::from_cents(c).percent(Percent::from_hundredths(h));
            let _ = Money::from_cents(c).ratio(Money::from_cents(h));
        }
    }
}
