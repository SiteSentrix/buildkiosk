//! Drawings: the project's sheet log, each sheet with a history of revisions.

use buildkiosk_core::{Date, Id, Timestamp};
use serde::{Deserialize, Serialize};

use crate::choice::Choice;
use crate::common::{FileRef, Stamp};
use crate::directory::Person;
use crate::project::{CostCode, Project};
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
        /// A length measured along a path through `points` at the page's scale.
        Polyline = ("polyline", "Path length"),
        /// The area inside the outline through `points` at the page's scale.
        Polygon = ("polygon", "Outline area"),
        /// One item counted at (x, y); the note names what is counted.
        Count = ("count", "Count"),
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

    /// The real length along a path through `points`, in micrometres.
    pub fn path_length(&self, points: &[(u16, u16)]) -> Option<u64> {
        let mut total: u64 = 0;
        for w in points.windows(2) {
            let (a, b) = (w.first()?, w.get(1)?);
            total = total.checked_add(self.length(a.0, a.1, b.0, b.1)?)?;
        }
        (total as f64 <= MAX_SCALE_LENGTH_UM as f64).then_some(total)
    }

    /// The real area inside the outline through `points`, in square micrometres (the shoelace sum,
    /// worked out from the first corner so large coordinates don't swamp it).
    pub fn polygon_area(&self, points: &[(u16, u16)]) -> Option<u64> {
        if !self.usable() || points.len() < 3 {
            return None;
        }
        let (cx, cy) = Self::steps(self.x, self.y, self.x2, self.y2);
        let along = (self.span2(cx, cy) as f64).sqrt();
        let (fx, fy) = (f64::from(self.aspect) / along * self.length_um as f64, 1_000.0 / along * self.length_um as f64);
        let first = points.first()?;
        let real: Vec<(f64, f64)> =
            points.iter().map(|p| ((f64::from(p.0) - f64::from(first.0)) * fx, (f64::from(p.1) - f64::from(first.1)) * fy)).collect();
        let mut twice = 0.0f64;
        for (i, a) in real.iter().enumerate() {
            let b = real.get((i + 1) % real.len())?;
            twice += a.0 * b.1 - b.0 * a.1;
        }
        to_micrometres(twice.abs() / 2.0, 1e18)
    }

    /// What a markup of `kind` through the given corners measures at this scale, if it measures anything.
    pub fn measure_markup(&self, kind: MarkupKind, x: u16, y: u16, x2: u16, y2: u16, points: &[(u16, u16)]) -> Option<u64> {
        match kind {
            MarkupKind::Dimension => self.length(x, y, x2, y2),
            MarkupKind::Area => self.area(x, y, x2, y2),
            MarkupKind::Polyline => self.path_length(points),
            MarkupKind::Polygon => self.polygon_area(points),
            _ => None,
        }
    }
}

/// The most corners a path or outline has.
pub const MAX_POINTS: usize = 200;

impl MarkupKind {
    /// Whether the markup is drawn through a list of points rather than one or two.
    pub fn is_path(self) -> bool {
        matches!(self, MarkupKind::Polyline | MarkupKind::Polygon)
    }

    /// Whether it measures a length (a straight one or along a path).
    pub fn measures_length(self) -> bool {
        matches!(self, MarkupKind::Dimension | MarkupKind::Polyline)
    }

    /// Whether it measures an area (a box or an outline).
    pub fn measures_area(self) -> bool {
        matches!(self, MarkupKind::Area | MarkupKind::Polygon)
    }
}

/// Which side of the line from `a` to `b` the point `c` is on: positive left, negative right, 0 on it.
fn side(a: (u16, u16), b: (u16, u16), c: (u16, u16)) -> i64 {
    let v = |p: (u16, u16)| (i64::from(p.0), i64::from(p.1));
    let ((ax, ay), (bx, by), (cx, cy)) = (v(a), v(b), v(c));
    ((bx - ax) * (cy - ay) - (by - ay) * (cx - ax)).signum()
}

/// Whether `c` lies within the box of the segment `a`-`b` (used once the three are known to be in a line).
fn within(a: (u16, u16), b: (u16, u16), c: (u16, u16)) -> bool {
    c.0 >= a.0.min(b.0) && c.0 <= a.0.max(b.0) && c.1 >= a.1.min(b.1) && c.1 <= a.1.max(b.1)
}

/// Whether the segments `a`-`b` and `c`-`d` touch or cross.
fn segments_meet(a: (u16, u16), b: (u16, u16), c: (u16, u16), d: (u16, u16)) -> bool {
    let (s1, s2, s3, s4) = (side(a, b, c), side(a, b, d), side(c, d, a), side(c, d, b));
    if s1 != s2 && s3 != s4 {
        return true;
    }
    (s1 == 0 && within(a, b, c)) || (s2 == 0 && within(a, b, d)) || (s3 == 0 && within(c, d, a)) || (s4 == 0 && within(c, d, b))
}

/// Whether a path (or, when `closed`, an outline) goes through no corner twice in a row and no two of
/// its sides touch except where neighbours meet. An outline that crosses itself has no clear area.
pub fn outline_is_simple(points: &[(u16, u16)], closed: bool) -> bool {
    let n = points.len();
    if points.windows(2).any(|w| w.first() == w.get(1)) || (closed && n > 1 && points.first() == points.last()) {
        return false;
    }
    let sides = if closed { n } else { n.saturating_sub(1) };
    let at = |i: usize| (points.get(i % n).copied().unwrap_or((0, 0)), points.get((i + 1) % n).copied().unwrap_or((0, 0)));
    for i in 0..sides {
        for j in (i + 1)..sides {
            let neighbours = j == i + 1 || (closed && i == 0 && j == sides - 1);
            let ((a, b), (c, d)) = (at(i), at(j));
            if neighbours {
                // Neighbours share a corner; they must not also run back over each other.
                let (shared, p, q) = if j == i + 1 { (b, a, d) } else { (a, b, c) };
                if side(shared, p, q) == 0 && (within(shared, p, q) || within(shared, q, p)) {
                    return false;
                }
            } else if segments_meet(a, b, c, d) {
                return false;
            }
        }
    }
    true
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
    /// The corners of a path or outline, in order (x, y and x2, y2 are its first two).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<(u16, u16)>,
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
    /// The cost code a measurement or count is for, so quantities add up by code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_code: Option<Id<CostCode>>,
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
            k if k.measures_length() && self.measure > 0 => Some(unit.length_text(self.measure)),
            k if k.measures_area() && self.measure > 0 => Some(unit.area_text(self.measure)),
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use buildkiosk_core::Timestamp;

    /// A square page where the stretch (0,0)-(1000,0) is 10 ft long: 100 units a foot.
    fn scale() -> PageScale {
        PageScale {
            page: 1,
            x: 0,
            y: 0,
            x2: 1000,
            y2: 0,
            aspect: 1000,
            length_um: 10 * 304_800,
            unit: LengthUnit::Feet,
            by: None,
            at: Timestamp::default(),
        }
    }

    #[test]
    fn paths_and_outlines_are_measured() {
        let s = scale();
        // A path of 10 ft along x and then 5 ft along y.
        assert_eq!(s.path_length(&[(0, 0), (1000, 0), (1000, 500)]), Some(15 * 304_800));
        // A 10 ft by 5 ft rectangle drawn as an outline matches the box tool.
        let rect = [(0, 0), (1000, 0), (1000, 500), (0, 500)];
        assert_eq!(s.polygon_area(&rect), s.area(0, 0, 1000, 500));
        assert_eq!(s.polygon_area(&rect).map(|a| LengthUnit::Feet.area_text(a)).as_deref(), Some("50.0 sq ft"));
        // An L shape: 10 x 10 ft minus a 5 x 5 ft corner = 75 sq ft; the corner order doesn't matter.
        let l = [(0, 0), (1000, 0), (1000, 500), (500, 500), (500, 1000), (0, 1000)];
        let rev: Vec<_> = l.iter().rev().copied().collect();
        assert_eq!(s.polygon_area(&l), s.polygon_area(&rev));
        assert_eq!(s.polygon_area(&l).map(|a| LengthUnit::Feet.area_text(a)).as_deref(), Some("75.0 sq ft"));
        assert_eq!(s.polygon_area(&[(0, 0), (10, 10)]), None);
        assert_eq!(s.measure_markup(MarkupKind::Pin, 0, 0, 0, 0, &[]), None);
    }

    #[test]
    fn outlines_that_cross_themselves_are_refused() {
        let bow_tie = [(0, 0), (1000, 1000), (1000, 0), (0, 1000)];
        assert!(!outline_is_simple(&bow_tie, true));
        assert!(outline_is_simple(&[(0, 0), (1000, 0), (1000, 1000), (0, 1000)], true));
        // A path may not cross itself either, nor repeat a corner or double back.
        assert!(!outline_is_simple(&[(0, 0), (1000, 1000), (1000, 0), (0, 1000)], false));
        assert!(!outline_is_simple(&[(0, 0), (0, 0), (10, 10)], false));
        assert!(!outline_is_simple(&[(0, 0), (1000, 0), (500, 0)], false));
        assert!(outline_is_simple(&[(0, 0), (1000, 0), (1000, 1000)], false));
        // An open path whose last point touches an earlier side is refused; a closed one is not "open".
        assert!(!outline_is_simple(&[(0, 0), (1000, 0), (1000, 1000), (500, 0)], false));
    }
}
