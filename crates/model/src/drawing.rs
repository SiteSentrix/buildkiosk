//! Drawings: the project's sheet log, each sheet with a history of revisions.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::choice::Choice;
use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::Project;
use crate::{choice, record};

choice! {
    /// The design discipline a sheet belongs to.
    #[derive(Default)]
    pub enum Discipline {
        #[default]
        General = ("general", "General"),
        Civil = ("civil", "Civil"),
        Landscape = ("landscape", "Landscape"),
        Architectural = ("architectural", "Architectural"),
        Interior = ("interior", "Interior"),
        Structural = ("structural", "Structural"),
        Mechanical = ("mechanical", "Mechanical"),
        Plumbing = ("plumbing", "Plumbing"),
        FireProtection = ("fire_protection", "Fire protection"),
        Electrical = ("electrical", "Electrical"),
        Other = ("other", "Other"),
    }
}

choice! {
    /// What a markup draws on a sheet.
    pub enum MarkupKind {
        /// A point with a note.
        Pin = ("pin", "Pin"),
        /// A box around an area.
        Rectangle = ("rect", "Box"),
        /// An arrow from (x, y) to (x2, y2).
        Arrow = ("arrow", "Arrow"),
        /// Words placed at (x, y).
        Text = ("text", "Text"),
        /// A length measured from (x, y) to (x2, y2) at the page's scale.
        Dimension = ("dim", "Length"),
        /// The area of the box from (x, y) to (x2, y2) at the page's scale.
        Area = ("area", "Area"),
    }
}

choice! {
    /// What a page's scale was set in; the lengths and areas measured on it are shown in this unit.
    #[derive(Default)]
    pub enum LengthUnit {
        #[default]
        Feet = ("ft", "Feet"),
        Inches = ("in", "Inches"),
        Meters = ("m", "Metres"),
        Millimeters = ("mm", "Millimetres"),
    }
}

impl LengthUnit {
    /// How many micrometres make one of this unit.
    pub fn micrometres(self) -> u64 {
        match self {
            LengthUnit::Feet => 304_800,
            LengthUnit::Inches => 25_400,
            LengthUnit::Meters => 1_000_000,
            LengthUnit::Millimeters => 1_000,
        }
    }

    /// A length in micrometres as text in this unit: "12.50 ft".
    pub fn length_text(self, um: u64) -> String {
        let value = um as f64 / self.micrometres() as f64;
        match self {
            LengthUnit::Millimeters => format!("{value:.0} mm"),
            _ => format!("{value:.2} {}", self.key()),
        }
    }

    /// An area in square micrometres as text in this unit: "120.5 sq ft".
    pub fn area_text(self, um2: u64) -> String {
        let one = self.micrometres() as f64;
        let value = um2 as f64 / (one * one);
        match self {
            LengthUnit::Millimeters => format!("{value:.0} sq mm"),
            LengthUnit::Meters => format!("{value:.2} sq m"),
            _ => format!("{value:.1} sq {}", self.key()),
        }
    }

    /// Reads a length typed in this unit ("12", "12.5", "0.375": at most three decimals) as micrometres.
    pub fn parse_length(self, text: &str) -> Option<u64> {
        let text = text.trim();
        let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
        let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
        if whole.is_empty() && frac.is_empty() || !digits(whole) || !digits(frac) || frac.len() > 3 || whole.len() > 9 {
            return None;
        }
        let thousandths = whole.parse::<u64>().unwrap_or(0) * 1_000 + format!("{frac:0<3}").parse::<u64>().ok()?;
        let um = thousandths.checked_mul(self.micrometres())? / 1_000;
        (1..=MAX_SCALE_LENGTH_UM).contains(&um).then_some(um)
    }
}

/// The longest real length a scale may be set from or a measurement may give: 10 km.
pub const MAX_SCALE_LENGTH_UM: u64 = 10_000_000_000;

/// The narrowest and widest a page may be (width over height, in thousandths).
pub const MIN_ASPECT: u32 = 100;
pub const MAX_ASPECT: u32 = 100_000;

/// A stretch drawn on a page must be at least this long (1% of the page's height) to set a scale from.
const MIN_SPAN2: u128 = 100 * 1_000 * 100 * 1_000;

/// The scale of one page: a stretch drawn on it (`x`,`y` to `x2`,`y2`) whose real length is known.
/// Any other stretch on the page is measured against it. The page's shape (`aspect`, its width over
/// its height in thousandths) is kept so that measuring needs nothing but this record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageScale {
    /// The page, from 1.
    pub page: u32,
    pub x: u16,
    pub y: u16,
    pub x2: u16,
    pub y2: u16,
    pub aspect: u32,
    /// The real length of the stretch, in micrometres.
    pub length_um: u64,
    pub unit: LengthUnit,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
}

impl PageScale {
    /// The squared length of a step of (`dx`, `dy`) on the page, in a unit that is the same for every
    /// direction (both axes are brought to thousandths of the page's height).
    fn span2(&self, dx: i64, dy: i64) -> u128 {
        let (sx, sy) = (i128::from(dx) * i128::from(self.aspect), i128::from(dy) * 1_000);
        (sx * sx + sy * sy).unsigned_abs()
    }

    fn steps(x: u16, y: u16, x2: u16, y2: u16) -> (i64, i64) {
        (i64::from(x2) - i64::from(x), i64::from(y2) - i64::from(y))
    }

    /// Whether the drawn stretch is long enough to measure against, and the page's shape is sane.
    pub fn usable(&self) -> bool {
        let (dx, dy) = Self::steps(self.x, self.y, self.x2, self.y2);
        (MIN_ASPECT..=MAX_ASPECT).contains(&self.aspect) && self.length_um > 0 && self.span2(dx, dy) >= MIN_SPAN2
    }

    /// The real length of the stretch from (`x`,`y`) to (`x2`,`y2`), in micrometres.
    ///
    /// Only IEEE operations that are exact everywhere (+, −, ×, ÷, square root) are used, so the same
    /// numbers come out on every machine, which the journal needs to replay the same state.
    pub fn length(&self, x: u16, y: u16, x2: u16, y2: u16) -> Option<u64> {
        if !self.usable() {
            return None;
        }
        let (cx, cy) = Self::steps(self.x, self.y, self.x2, self.y2);
        let (dx, dy) = Self::steps(x, y, x2, y2);
        let ratio = (self.span2(dx, dy) as f64 / self.span2(cx, cy) as f64).sqrt();
        to_micrometres(ratio * self.length_um as f64, MAX_SCALE_LENGTH_UM as f64)
    }

    /// The real area of the box from (`x`,`y`) to (`x2`,`y2`), in square micrometres.
    pub fn area(&self, x: u16, y: u16, x2: u16, y2: u16) -> Option<u64> {
        if !self.usable() {
            return None;
        }
        let (cx, cy) = Self::steps(self.x, self.y, self.x2, self.y2);
        let (dx, dy) = Self::steps(x, y, x2, y2);
        let along = (self.span2(cx, cy) as f64).sqrt();
        let width = (dx.unsigned_abs() as f64 * f64::from(self.aspect)) / along * self.length_um as f64;
        let height = (dy.unsigned_abs() as f64 * 1_000.0) / along * self.length_um as f64;
        to_micrometres(width * height, 1e18)
    }
}

fn to_micrometres(value: f64, most: f64) -> Option<u64> {
    (value.is_finite() && (0.0..=most).contains(&value)).then(|| value.round() as u64)
}

/// Where and how far across a page things are placed: 0 to this, left to right and top to bottom.
pub const MARKUP_SCALE: u16 = 10_000;

/// A note drawn on one page of one revision. Positions are fractions of the page
/// (`0..=MARKUP_SCALE`), so they stay right at any zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Markup {
    /// Counts up within the revision.
    pub id: u32,
    /// The page, from 1.
    pub page: u32,
    pub kind: MarkupKind,
    pub x: u16,
    pub y: u16,
    /// The far corner of a box, or the arrow's head.
    #[serde(default)]
    pub x2: u16,
    #[serde(default)]
    pub y2: u16,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    /// For a length: micrometres; for an area: square micrometres, worked out when it was drawn
    /// from the page's scale (and again if the scale changes).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub measure: u64,
    /// The unit `measure` is shown in (the page's scale unit).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<LengthUnit>,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

impl Markup {
    /// The measurement as text ("12.50 ft", "120.5 sq ft"), if this markup measures something.
    pub fn measure_text(&self) -> Option<String> {
        let unit = self.unit?;
        match self.kind {
            MarkupKind::Dimension if self.measure > 0 => Some(unit.length_text(self.measure)),
            MarkupKind::Area if self.measure > 0 => Some(unit.area_text(self.measure)),
            _ => None,
        }
    }
}

/// One issue of a sheet. The newest revision is the current one; older ones stay as history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrawingRevision {
    /// What the design team calls it: "A", "2", "Addendum 3".
    pub label: String,
    /// The issue date printed on the sheet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    /// The set or package it arrived in: "Permit set", "Bulletin 4".
    #[serde(default)]
    pub set: String,
    #[serde(default)]
    pub note: String,
    /// The sheet file; none for a revision logged before the file arrived.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<FileRef>,
    pub by: Option<Id<Person>>,
    pub at: Timestamp,
    /// Notes drawn on its pages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markups: Vec<Markup>,
    /// The scale of the pages that have one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scales: Vec<PageScale>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Drawing {
    pub id: Id<Drawing>,
    pub project: Id<Project>,
    /// The sheet number as written on the drawing, unique in the project: "A-101".
    pub number: String,
    pub title: String,
    #[serde(default)]
    pub discipline: Discipline,
    /// The building, floor or zone it covers.
    #[serde(default)]
    pub area: String,
    /// Oldest first; empty for a sheet that is only on the log so far.
    #[serde(default)]
    pub revisions: Vec<DrawingRevision>,
    pub created: Stamp,
    pub updated: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Stamp>,
}
record!(Drawing, "drawing", trash, project);

impl Drawing {
    pub fn current(&self) -> Option<&DrawingRevision> {
        self.revisions.last()
    }

    pub fn label(&self) -> String {
        format!("{} {}", self.number, self.title).trim().to_string()
    }
}

#[cfg(test)]
mod measure_tests {
    use super::*;

    fn scale(aspect: u32) -> PageScale {
        // A stretch across the middle of the page, 5000 of 10000 wide, known to be 10 feet.
        PageScale {
            page: 1,
            x: 2_500,
            y: 5_000,
            x2: 7_500,
            y2: 5_000,
            aspect,
            length_um: 10 * 304_800,
            unit: LengthUnit::Feet,
            by: None,
            at: Timestamp::from_millis(0),
        }
    }

    #[test]
    fn lengths_follow_the_scale() {
        let s = scale(1_000);
        // The same stretch measures what it was set to; half of it, half; a doubled one, double.
        assert_eq!(s.length(2_500, 5_000, 7_500, 5_000), Some(10 * 304_800));
        assert_eq!(s.length(0, 0, 2_500, 0), Some(5 * 304_800));
        assert_eq!(s.length(0, 0, 10_000, 0), Some(20 * 304_800));
        // Across the page's height on a square page it is the same as across its width.
        assert_eq!(s.length(0, 0, 0, 5_000), Some(10 * 304_800));
        // On a page twice as wide as high, a step down counts for half of the same step across.
        let wide = scale(2_000);
        assert_eq!(wide.length(0, 0, 0, 5_000), Some(5 * 304_800));
        // A diagonal of 5000 across and 5000 down: the across step counts double, so the length is 5/4 of the stretch squared, rooted.
        assert_eq!(wide.length(0, 0, 5_000, 5_000), Some((3_048_000f64 * 1.25f64.sqrt()).round() as u64));
        // Direction doesn't matter.
        assert_eq!(s.length(7_500, 5_000, 2_500, 5_000), s.length(2_500, 5_000, 7_500, 5_000));
    }

    #[test]
    fn areas_follow_the_scale() {
        let s = scale(1_000);
        // 5000 × 5000 units of the page is 10 ft × 10 ft.
        assert_eq!(s.area(0, 0, 5_000, 5_000), Some(100 * 304_800 * 304_800));
        assert_eq!(s.area(1_000, 1_000, 1_000, 6_000), Some(0));
        let wide = scale(2_000);
        // 10 ft across, and 5000 down is only 5 ft on a page twice as wide as it is high.
        assert_eq!(wide.area(0, 0, 5_000, 5_000), Some(50 * 304_800 * 304_800));
    }

    #[test]
    fn a_bad_scale_measures_nothing() {
        let mut s = scale(1_000);
        s.x2 = s.x + 10;
        assert!(!s.usable() && s.length(0, 0, 100, 0).is_none() && s.area(0, 0, 100, 100).is_none());
        let mut s = scale(50);
        assert!(!s.usable());
        s.aspect = 1_000;
        s.length_um = 0;
        assert!(!s.usable());
        let mut huge = scale(1_000);
        huge.length_um = MAX_SCALE_LENGTH_UM;
        assert_eq!(huge.length(0, 0, 10_000, 10_000), None, "more than ten kilometres is refused");
    }

    #[test]
    fn typed_lengths_are_read_exactly() {
        let ft = LengthUnit::Feet;
        assert_eq!(ft.parse_length("10"), Some(3_048_000));
        assert_eq!(ft.parse_length(" 12.5 "), Some(3_810_000));
        assert_eq!(LengthUnit::Inches.parse_length("0.375"), Some(9_525));
        assert_eq!(LengthUnit::Meters.parse_length("3.2"), Some(3_200_000));
        assert_eq!(LengthUnit::Millimeters.parse_length("1"), Some(1_000));
        for bad in ["", ".", "-1", "1e3", "1.2345", "0", "0.000", "12 ft", "1,5", "99999999999", "40000"] {
            assert_eq!(ft.parse_length(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn measurements_read_in_the_scale_unit() {
        assert_eq!(LengthUnit::Feet.length_text(3_810_000), "12.50 ft");
        assert_eq!(LengthUnit::Meters.length_text(3_200_000), "3.20 m");
        assert_eq!(LengthUnit::Millimeters.length_text(1_500_000), "1500 mm");
        assert_eq!(LengthUnit::Feet.area_text(100 * 304_800 * 304_800), "100.0 sq ft");
        assert_eq!(LengthUnit::Meters.area_text(2_500_000_000_000), "2.50 sq m");
    }
}
