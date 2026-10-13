//! What the markups on a page look like: each one becomes a few shapes in window coordinates, which
//! the browser code draws. Kept apart from the drawing so it can be tested.

use buildkiosk_model::drawing::MarkupKind;
use serde::Deserialize;

use crate::geom::{Camera, from_units};

/// One markup, as the server describes it in the page.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Mark {
    pub id: u32,
    /// The page, from 1.
    #[serde(default)]
    pub page: u32,
    pub kind: MarkupKind,
    pub x: u16,
    pub y: u16,
    #[serde(default)]
    pub x2: u16,
    #[serde(default)]
    pub y2: u16,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub resolved: bool,
    /// What a length or area measures, as text ("12.50 ft").
    #[serde(default)]
    pub measure: Option<String>,
}

/// At most this many markups are read from a page.
pub const MAX_MARKS: usize = 5_000;

/// Reads the markups the server put in the page; anything unreadable gives none.
pub fn parse(json: &str) -> Vec<Mark> {
    let mut marks: Vec<Mark> = serde_json::from_str(json).unwrap_or_default();
    marks.truncate(MAX_MARKS);
    marks
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Pin {
        x: f64,
        y: f64,
        label: String,
    },
    Box {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        label: String,
    },
    Arrow {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        label: String,
    },
    /// A length: a line with a tick at each end.
    Line {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        label: String,
    },
    Text {
        x: f64,
        y: f64,
        label: String,
    },
}

/// A shape and whether it is resolved (drawn in grey).
pub type Drawn = (Shape, bool);

pub fn shapes(marks: &[Mark], camera: &Camera) -> Vec<Drawn> {
    marks
        .iter()
        .map(|m| {
            let (x, y) = camera.to_screen(from_units(m.x), from_units(m.y));
            let (x2, y2) = camera.to_screen(from_units(m.x2), from_units(m.y2));
            let id = m.id.to_string();
            let shape = match m.kind {
                MarkupKind::Pin => Shape::Pin { x, y, label: id },
                MarkupKind::Rectangle => Shape::Box { x: x.min(x2), y: y.min(y2), w: (x - x2).abs(), h: (y - y2).abs(), label: id },
                MarkupKind::Arrow => Shape::Arrow { x1: x, y1: y, x2, y2, label: id },
                MarkupKind::Text => Shape::Text { x, y, label: format!("{id}. {}", m.text) },
                MarkupKind::Dimension => Shape::Line { x1: x, y1: y, x2, y2, label: with_measure(&id, m) },
                MarkupKind::Area => Shape::Box { x: x.min(x2), y: y.min(y2), w: (x - x2).abs(), h: (y - y2).abs(), label: with_measure(&id, m) },
            };
            (shape, m.resolved)
        })
        .collect()
}

/// "3" or, for a measurement, "3 · 12.50 ft".
fn with_measure(id: &str, m: &Mark) -> String {
    match &m.measure {
        Some(t) if !t.is_empty() => format!("{id} · {t}"),
        _ => id.to_string(),
    }
}

/// The two corner points of an arrow head: the head is at (`x2`, `y2`), `size` long.
pub fn arrow_head(x1: f64, y1: f64, x2: f64, y2: f64, size: f64) -> [(f64, f64); 2] {
    let angle = (y2 - y1).atan2(x2 - x1);
    let spread = 0.45;
    [angle + std::f64::consts::PI - spread, angle + std::f64::consts::PI + spread].map(|a| (x2 + size * a.cos(), y2 + size * a.sin()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geom::Stage;

    #[test]
    fn reads_what_the_server_writes() {
        let marks = parse(
            r#"[{"id":1,"kind":"pin","x":2500,"y":5000},{"id":2,"kind":"rect","x":0,"y":0,"x2":10000,"y2":10000,"text":"hi","resolved":true}]"#,
        );
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].kind, MarkupKind::Pin);
        assert!(marks[1].resolved && marks[1].text == "hi");
    }

    #[test]
    fn junk_gives_no_markups() {
        for junk in ["", "null", "{}", "[1,2]", r#"[{"id":1,"kind":"circle","x":1,"y":1}]"#, "[{"] {
            assert!(parse(junk).is_empty(), "{junk}");
        }
    }

    #[test]
    fn markups_follow_the_page() {
        let cam = Camera::fit(Stage { w: 800.0, h: 600.0 }, 1.0);
        let marks = parse(r#"[{"id":7,"kind":"rect","x":8000,"y":6000,"x2":2000,"y2":2000}]"#);
        let drawn = shapes(&marks, &cam);
        let [(Shape::Box { x, y, w, h, label }, false)] = drawn.as_slice() else { panic!() };
        let (x, y, w, h) = (*x, *y, *w, *h);
        assert_eq!(label, "7");
        assert!(w > 0.0 && h > 0.0 && (x - (cam.x + 0.2 * cam.w)).abs() < 1e-9 && (y - (cam.y + 0.2 * cam.h)).abs() < 1e-9);
        // Zooming moves and scales them with the page.
        let z = cam.zoom_at(400.0, 300.0, 2.0);
        let zoomed = shapes(&marks, &z);
        let [(Shape::Box { w: w2, .. }, _)] = zoomed.as_slice() else { panic!() };
        assert!((w2 - 2.0 * w).abs() < 1e-9);
    }

    #[test]
    fn measurements_say_what_they_measure() {
        let cam = Camera::fit(Stage { w: 800.0, h: 600.0 }, 1.0);
        let marks = parse(
            r#"[{"id":4,"kind":"dim","x":0,"y":0,"x2":5000,"y2":0,"measure":"12.50 ft"},{"id":5,"kind":"area","x":0,"y":0,"x2":5000,"y2":5000,"measure":"100.0 sq ft"},{"id":6,"kind":"dim","x":0,"y":0,"x2":9,"y2":0}]"#,
        );
        let drawn = shapes(&marks, &cam);
        assert!(matches!(&drawn[0].0, Shape::Line { label, .. } if label == "4 · 12.50 ft"));
        assert!(matches!(&drawn[1].0, Shape::Box { label, .. } if label == "5 · 100.0 sq ft"));
        assert!(matches!(&drawn[2].0, Shape::Line { label, .. } if label == "6"), "a measurement without a scale shows only its number");
    }

    #[test]
    fn the_arrow_head_sits_at_the_tip() {
        let [a, b] = arrow_head(0.0, 0.0, 100.0, 0.0, 10.0);
        assert!(a.0 < 100.0 && b.0 < 100.0 && (a.1 + b.1).abs() < 1e-9 && a.1 != b.1);
    }
}
