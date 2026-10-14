//! Drawing pages as pictures: PDF pages are drawn with a pure-Rust renderer into PNG images the
//! viewer page shows, and two revisions can be laid over each other to show what changed.
//!
//! Everything here treats the file as untrusted: sizes are capped and any failure is a `None`.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

/// The widest picture made.
pub const MAX_WIDTH: u32 = 2400;
/// The narrowest.
pub const MIN_WIDTH: u32 = 300;
/// The tallest picture made, in pixels.
const MAX_HEIGHT: u32 = 6000;

/// What a sheet file is: a PDF (with the size of each page, in points) or a single picture.
#[derive(Clone, Debug, PartialEq)]
pub struct Info {
    pub image: bool,
    /// Width and height of each page (of the picture, for an image), at most 2,000 pages.
    pub dims: Vec<(f32, f32)>,
}

impl Info {
    pub fn pages(&self) -> usize {
        self.dims.len()
    }
}

/// Reads what a sheet file is, or `None` when it is neither a PDF nor a picture that can be shown.
pub fn info(bytes: Vec<u8>) -> Option<Info> {
    if bytes.starts_with(b"%PDF-") {
        let pdf = Pdf::new(bytes).ok()?;
        let dims: Vec<(f32, f32)> = pdf.pages().iter().take(2_000).map(|p| p.render_dimensions()).collect();
        return Some(Info { image: false, dims })
            .filter(|i| !i.dims.is_empty() && i.dims.iter().all(|(w, h)| w.is_finite() && h.is_finite() && *w >= 1.0 && *h >= 1.0));
    }
    let format = image::guess_format(&bytes).ok()?;
    if !matches!(format, image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Gif | image::ImageFormat::WebP) {
        return None;
    }
    let (w, h) = image::ImageReader::with_format(std::io::Cursor::new(&bytes), format).into_dimensions().ok()?;
    Some(Info { image: true, dims: vec![(w as f32, h as f32)] }).filter(|i| i.dims.iter().all(|(w, h)| *w >= 1.0 && *h >= 1.0))
}

/// Page `page` (from 0) as a PNG `width` pixels wide, or `None`.
pub fn render_page(bytes: Vec<u8>, page: usize, width: u32) -> Option<Vec<u8>> {
    let width = width.clamp(MIN_WIDTH, MAX_WIDTH);
    let pdf = Pdf::new(bytes).ok()?;
    let pages = pdf.pages();
    let p = pages.get(page)?;
    let (w, h) = p.render_dimensions();
    if !(w.is_finite() && h.is_finite()) || w < 1.0 || h < 1.0 {
        return None;
    }
    let mut scale = width as f32 / w;
    if h * scale > MAX_HEIGHT as f32 {
        scale = MAX_HEIGHT as f32 / h;
    }
    let cache = RenderCache::new();
    let pixmap = render(
        p,
        &cache,
        &InterpreterSettings::default(),
        &RenderSettings::default(),
        &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
    );
    pixmap.into_png().ok()
}

/// What changed between two pictures of the same page: ink only in `b` is red, ink only in `a`
/// is blue, ink in both is light gray. The result is as big as the larger picture.
pub fn compare(a_png: &[u8], b_png: &[u8]) -> Option<Vec<u8>> {
    let a = image::load_from_memory(a_png).ok()?.to_luma8();
    let b = image::load_from_memory(b_png).ok()?.to_luma8();
    let (w, h) = (a.width().max(b.width()), a.height().max(b.height()));
    if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 30_000_000 {
        return None;
    }
    let mut out = image::RgbImage::from_pixel(w, h, image::Rgb([255, 255, 255]));
    for y in 0..h {
        for x in 0..w {
            // Anything off the edge of the smaller picture counts as paper.
            let ink = |img: &image::GrayImage| img.get_pixel_checked(x, y).is_some_and(|p| p.0.first().copied().unwrap_or(255) < 160);
            let pixel = match (ink(&a), ink(&b)) {
                (true, true) => image::Rgb([190, 190, 190]),
                (false, true) => image::Rgb([220, 30, 30]),
                (true, false) => image::Rgb([30, 90, 220]),
                (false, false) => continue,
            };
            out.put_pixel(x, y, pixel);
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(out).write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

/// Pictures already made, kept in memory so panning back and forth doesn't redraw: the newest
/// ones up to a total size.
#[derive(Default)]
pub struct Pictures {
    map: HashMap<(String, usize, u32), Arc<Vec<u8>>>,
    order: VecDeque<(String, usize, u32)>,
    bytes: usize,
    infos: HashMap<String, Option<Arc<Info>>>,
}

const KEEP_BYTES: usize = 64 * 1024 * 1024;

impl Pictures {
    pub fn get(&self, sha: &str, page: usize, width: u32) -> Option<Arc<Vec<u8>>> {
        self.map.get(&(sha.to_string(), page, width)).cloned()
    }

    pub fn put(&mut self, sha: &str, page: usize, width: u32, png: Vec<u8>) -> Arc<Vec<u8>> {
        let key = (sha.to_string(), page, width);
        let png = Arc::new(png);
        if png.len() > KEEP_BYTES / 4 {
            return png;
        }
        self.bytes += png.len();
        self.order.push_back(key.clone());
        if let Some(old) = self.map.insert(key, png.clone()) {
            self.bytes = self.bytes.saturating_sub(old.len());
        }
        while self.bytes > KEEP_BYTES {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(gone) = self.map.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(gone.len());
            }
        }
        png
    }

    pub fn info(&self, sha: &str) -> Option<Option<Arc<Info>>> {
        self.infos.get(sha).cloned()
    }

    pub fn set_info(&mut self, sha: &str, info: Option<Info>) {
        if self.infos.len() > 2_000 {
            self.infos.clear();
        }
        self.infos.insert(sha.to_string(), info.map(Arc::new));
    }
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    /// A one-page PDF with a filled box, written by hand.
    pub fn tiny_pdf(box_x: u32) -> Vec<u8> {
        pdf_page(&format!("0 0 0 rg {box_x} 100 80 60 re f"), "<< >>")
    }

    /// A one-page PDF with the given page content and resources dictionary.
    fn pdf_page(content: &str, resources: &str) -> Vec<u8> {
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 300] /Contents 4 0 R /Resources {resources} >>"),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, o) in objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
        out
    }

    #[test]
    fn renders_a_page_and_counts_pages() {
        let i = info(tiny_pdf(50)).unwrap();
        assert_eq!((i.image, i.pages(), i.dims[0]), (false, 1, (400.0, 300.0)));
        let png = render_page(tiny_pdf(50), 0, 800).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_luma8();
        assert_eq!((img.width(), img.height()), (800, 600));
        assert!(img.get_pixel(150, 500).0[0] > 200, "paper is white");
        assert!(img.get_pixel(140, 340).0[0] < 60, "the box is black (PDF y runs upward)");
        assert!(render_page(tiny_pdf(50), 1, 800).is_none(), "no second page");
        assert!(render_page(b"not a pdf".to_vec(), 0, 800).is_none());
        assert!(info(Vec::new()).is_none() && info(b"%PDF-1.4 junk".to_vec()).is_none() && info(b"<svg/>".to_vec()).is_none());
        let png = render_page(tiny_pdf(50), 0, 300).unwrap();
        let i = info(png).unwrap();
        assert_eq!((i.image, i.pages(), i.dims[0]), (true, 1, (300.0, 225.0)));
    }

    /// Text in fonts the file does not embed (the standard names and common look-alikes) is drawn
    /// with the bundled set rather than left blank.
    #[test]
    fn text_in_fonts_the_file_does_not_embed_is_drawn() {
        for base in
            ["Helvetica", "Times-Roman", "Courier", "ArialMT", "TimesNewRomanPSMT", "Arial-BoldMT", "Calibri", "Verdana-Bold", "CourierNewPSMT"]
        {
            let resources = format!("<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /{base} >> >> >>");
            let pdf = pdf_page("BT /F1 48 Tf 20 120 Td (Footing F-2) Tj ET", &resources);
            let img = image::load_from_memory(&render_page(pdf, 0, 800).unwrap()).unwrap().to_luma8();
            let dark = img.pixels().filter(|p| p.0[0] < 100).count();
            assert!(dark > 400, "{base} drew {dark} dark pixels");
        }
    }

    #[test]
    fn compare_marks_what_moved() {
        let a = render_page(tiny_pdf(50), 0, 800).unwrap();
        let b = render_page(tiny_pdf(250), 0, 800).unwrap();
        let png = compare(&a, &b).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgb8();
        assert_eq!(img.get_pixel(140, 340).0, [30, 90, 220], "the old position is blue");
        assert_eq!(img.get_pixel(540, 340).0, [220, 30, 30], "the new position is red");
        assert_eq!(img.get_pixel(5, 5).0, [255, 255, 255]);
        let same = image::load_from_memory(&compare(&a, &a).unwrap()).unwrap().to_rgb8();
        assert_eq!(same.get_pixel(140, 340).0, [190, 190, 190]);
        assert!(compare(b"x", &a).is_none());
    }

    #[test]
    fn the_cache_forgets_the_oldest() {
        let mut c = Pictures::default();
        for n in 0..10 {
            c.put("s", n, 800, vec![0u8; 10 * 1024 * 1024]);
        }
        assert!(c.get("s", 9, 800).is_some() && c.get("s", 0, 800).is_none());
        c.set_info("s", Some(Info { image: false, dims: vec![(1.0, 1.0)] }));
        assert_eq!(c.info("s").unwrap().unwrap().pages(), 1);
        c.set_info("t", None);
        assert_eq!(c.info("t"), Some(None));
    }
}
