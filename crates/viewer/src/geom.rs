//! The maths of the viewer: where the page sits in the window, zooming around a point, and turning
//! a click into a position on the page. Pure functions, tested without a browser.

use buildkiosk_model::drawing::MARKUP_SCALE;

/// How far the page may be zoomed, as a multiple of "fit".
pub const MIN_ZOOM: f64 = 0.5;
pub const MAX_ZOOM: f64 = 40.0;
/// Breathing room around the page when it is fitted, in pixels.
const PAD: f64 = 8.0;
/// At least this much of the page stays inside the window when panning.
const KEEP: f64 = 48.0;

/// The size of the window the sheet is shown in, in CSS pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub w: f64,
    pub h: f64,
}

/// Where the page is: its size at zoom 1 (`w`, `h`) and the zoom and offset of its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub w: f64,
    pub h: f64,
    pub zoom: f64,
    pub x: f64,
    pub y: f64,
}

fn finite(v: f64, fallback: f64) -> f64 {
    if v.is_finite() { v } else { fallback }
}

impl Camera {
    /// The whole page, centred in the window, for a page `aspect` (width over height) wide.
    pub fn fit(stage: Stage, aspect: f64) -> Camera {
        let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
        let (aw, ah) = ((stage.w - 2.0 * PAD).max(1.0), (stage.h - 2.0 * PAD).max(1.0));
        let (w, h) = if aw / ah > aspect { (ah * aspect, ah) } else { (aw, aw / aspect) };
        Camera { w, h, zoom: 1.0, x: (stage.w - w) / 2.0, y: (stage.h - h) / 2.0 }
    }

    /// The page as drawn: width and height in pixels.
    pub fn size(&self) -> (f64, f64) {
        (self.w * self.zoom, self.h * self.zoom)
    }

    /// Zooms by `factor` keeping the page point under (`sx`, `sy`) where it is.
    pub fn zoom_at(self, sx: f64, sy: f64, factor: f64) -> Camera {
        let zoom = (self.zoom * finite(factor, 1.0)).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.zoom;
        Camera { zoom, x: sx - (sx - self.x) * ratio, y: sy - (sy - self.y) * ratio, ..self }
    }

    pub fn pan(self, dx: f64, dy: f64) -> Camera {
        Camera { x: self.x + finite(dx, 0.0), y: self.y + finite(dy, 0.0), ..self }
    }

    /// Keeps a part of the page in the window so it can't be lost.
    pub fn keep_in_view(self, stage: Stage) -> Camera {
        let (pw, ph) = self.size();
        let keep_x = KEEP.min(pw).min(stage.w);
        let keep_y = KEEP.min(ph).min(stage.h);
        Camera {
            x: self.x.clamp(keep_x - pw, (stage.w - keep_x).max(keep_x - pw)),
            y: self.y.clamp(keep_y - ph, (stage.h - keep_y).max(keep_y - ph)),
            ..self
        }
    }

    /// A point on the page, as fractions (0 to 1, from the top-left corner) of its size, for a
    /// point in the window; `None` outside the page.
    pub fn to_page(&self, sx: f64, sy: f64) -> Option<(f64, f64)> {
        let (pw, ph) = self.size();
        if pw <= 0.0 || ph <= 0.0 {
            return None;
        }
        let (fx, fy) = ((sx - self.x) / pw, (sy - self.y) / ph);
        ((0.0..=1.0).contains(&fx) && (0.0..=1.0).contains(&fy)).then_some((fx, fy))
    }

    /// The window position of a point on the page given as fractions.
    pub fn to_screen(&self, fx: f64, fy: f64) -> (f64, f64) {
        let (pw, ph) = self.size();
        (self.x + fx * pw, self.y + fy * ph)
    }
}

/// A fraction of the page as the whole number the markups are stored as.
pub fn to_units(fraction: f64) -> u16 {
    let scale = f64::from(MARKUP_SCALE);
    (finite(fraction, 0.0) * scale).round().clamp(0.0, scale) as u16
}

/// A stored position as a fraction of the page.
pub fn from_units(units: u16) -> f64 {
    f64::from(units.min(MARKUP_SCALE)) / f64::from(MARKUP_SCALE)
}

/// The most pixels on one side of a drawn page, and the most in all (a 4096 × 4096 picture takes
/// 64 MB).
pub const MAX_SIDE: u32 = 4096;
pub const MAX_PIXELS: u64 = 16_000_000;

/// How many pixels wide to draw the page so it is sharp at the current zoom (`pixel_ratio` is the
/// screen's pixels per CSS pixel), within the limits above. At least 64.
pub fn render_width(camera: &Camera, pixel_ratio: f64) -> u32 {
    let ratio = finite(pixel_ratio, 1.0).clamp(1.0, 4.0);
    let aspect = if camera.h > 0.0 { camera.w / camera.h } else { 1.0 };
    let mut width = (camera.w * camera.zoom * ratio).clamp(64.0, f64::from(MAX_SIDE));
    if width / aspect > f64::from(MAX_SIDE) {
        width = f64::from(MAX_SIDE) * aspect;
    }
    let area = width * (width / aspect);
    if area > MAX_PIXELS as f64 {
        width *= (MAX_PIXELS as f64 / area).sqrt();
    }
    width.floor().max(64.0) as u32
}

/// Whether a picture `have` pixels wide is too blurry or far too big for one `want` wide.
pub fn needs_redraw(have: u32, want: u32) -> bool {
    f64::from(want) > f64::from(have) * 1.15 || f64::from(want) * 2.0 < f64::from(have)
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGE: Stage = Stage { w: 1000.0, h: 600.0 };

    #[test]
    fn fit_centres_and_keeps_the_aspect() {
        let c = Camera::fit(STAGE, 1.5);
        assert!((c.w / c.h - 1.5).abs() < 1e-9);
        assert!(c.w <= STAGE.w - 16.0 + 1e-9 && c.h <= STAGE.h - 16.0 + 1e-9);
        assert!((c.x - (STAGE.w - c.w) / 2.0).abs() < 1e-9);
        // A wide page is limited by the width, a tall one by the height.
        assert!((Camera::fit(STAGE, 4.0).w - 984.0).abs() < 1e-9);
        assert!((Camera::fit(STAGE, 0.5).h - 584.0).abs() < 1e-9);
    }

    #[test]
    fn fit_survives_nonsense() {
        for aspect in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let c = Camera::fit(Stage { w: 0.0, h: -5.0 }, aspect);
            assert!(c.w.is_finite() && c.h.is_finite() && c.w > 0.0 && c.h > 0.0);
        }
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let c = Camera::fit(STAGE, 1.5);
        let (sx, sy) = (400.0, 300.0);
        let before = c.to_page(sx, sy).unwrap();
        let z = c.zoom_at(sx, sy, 3.0);
        let after = z.to_page(sx, sy).unwrap();
        assert!((before.0 - after.0).abs() < 1e-9 && (before.1 - after.1).abs() < 1e-9);
        assert!((z.zoom - 3.0).abs() < 1e-9);
    }

    #[test]
    fn zoom_is_limited_and_ignores_nonsense() {
        let c = Camera::fit(STAGE, 1.5);
        assert_eq!(c.zoom_at(0.0, 0.0, 1e9).zoom, MAX_ZOOM);
        assert_eq!(c.zoom_at(0.0, 0.0, 1e-9).zoom, MIN_ZOOM);
        assert_eq!(c.zoom_at(0.0, 0.0, f64::NAN), c);
        assert_eq!(c.pan(f64::NAN, f64::INFINITY), c);
    }

    #[test]
    fn the_page_cannot_be_panned_out_of_sight() {
        let c = Camera::fit(STAGE, 1.5).zoom_at(500.0, 300.0, 4.0);
        let far = c.pan(1e6, -1e6).keep_in_view(STAGE);
        let (pw, ph) = far.size();
        assert!(far.x < STAGE.w && far.x + pw > 0.0 && far.y + ph > 0.0 && far.y < STAGE.h);
        assert!(STAGE.w - far.x >= 47.9 && far.y + ph >= 47.9);
        // A page that is already in view isn't moved.
        let ok = Camera::fit(STAGE, 1.5);
        assert_eq!(ok.keep_in_view(STAGE), ok);
    }

    #[test]
    fn clicks_become_positions_on_the_page() {
        let c = Camera::fit(STAGE, 1.5);
        let (cx, cy) = c.to_screen(0.25, 0.75);
        let (fx, fy) = c.to_page(cx, cy).unwrap();
        assert!((fx - 0.25).abs() < 1e-9 && (fy - 0.75).abs() < 1e-9);
        assert_eq!(c.to_page(c.x - 1.0, c.y + 5.0), None);
        assert_eq!(c.to_page(c.x + 5.0, c.y + c.h + 1.0), None);
        assert_eq!(to_units(0.25), 2500);
        assert_eq!(to_units(1.5), 10_000);
        assert_eq!(to_units(f64::NAN), 0);
        assert!((from_units(2500) - 0.25).abs() < 1e-12);
        assert_eq!(from_units(u16::MAX), 1.0);
    }

    #[test]
    fn the_picture_is_sharp_but_bounded() {
        let c = Camera::fit(STAGE, 1.5);
        let fit = render_width(&c, 1.0);
        assert!((64..=MAX_SIDE).contains(&fit));
        assert!(render_width(&c.zoom_at(0.0, 0.0, 40.0), 2.0) <= MAX_SIDE);
        // A tall sheet is limited by its height and by the total.
        let tall = Camera { w: 100.0, h: 1000.0, zoom: 40.0, x: 0.0, y: 0.0 };
        let w = render_width(&tall, 1.0);
        assert!(u64::from(w) * u64::from(w) * 10 <= MAX_PIXELS + 10_000_000 && w * 10 <= MAX_SIDE + 10);
        assert!(render_width(&c, f64::NAN) >= 64);
        assert!(!needs_redraw(1000, 1000) && needs_redraw(1000, 1200) && needs_redraw(3000, 1000) && !needs_redraw(1500, 1000));
    }
}
