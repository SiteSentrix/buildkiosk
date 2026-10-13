//! A PDF file opened in the browser: how big each page is, and a page drawn as pixels.
//!
//! The file is untrusted. Sizes are capped and every failure is an `Err`.

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings, render};

use crate::geom::{MAX_PIXELS, MAX_SIDE};

/// Files bigger than this aren't opened here (the picture on the server is shown instead).
pub const MAX_FILE: usize = 150 * 1024 * 1024;

pub struct Sheet {
    pdf: Pdf,
    /// Width and height of each page, in points.
    dims: Vec<(f32, f32)>,
}

/// A drawn page: `width` × `height` pixels of RGBA, the background white and opaque.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Sheet {
    pub fn open(bytes: Vec<u8>) -> Result<Sheet, String> {
        if bytes.len() > MAX_FILE {
            return Err("This file is too big to open here.".into());
        }
        if !bytes.starts_with(b"%PDF-") {
            return Err("This is not a PDF.".into());
        }
        let pdf = Pdf::new(bytes).map_err(|_| "This PDF can't be read.".to_string())?;
        let dims: Vec<(f32, f32)> = pdf.pages().iter().take(2_000).map(|p| p.render_dimensions()).collect();
        if dims.is_empty() || !dims.iter().all(|(w, h)| w.is_finite() && h.is_finite() && *w >= 1.0 && *h >= 1.0) {
            return Err("This PDF has no pages that can be drawn.".into());
        }
        Ok(Sheet { pdf, dims })
    }

    pub fn pages(&self) -> usize {
        self.dims.len()
    }

    /// The width over the height of page `page` (from 0).
    pub fn aspect(&self, page: usize) -> Option<f64> {
        self.dims.get(page).map(|(w, h)| f64::from(*w) / f64::from(*h))
    }

    /// Page `page` (from 0) drawn `width` pixels wide (less if the height or the total would be too big).
    pub fn draw(&self, page: usize, width: u32) -> Result<Raster, String> {
        let (pw, ph) = self.dims.get(page).copied().ok_or("There is no such page.")?;
        let (pw, ph) = (f64::from(pw), f64::from(ph));
        let mut scale = f64::from(width.clamp(16, MAX_SIDE)) / pw;
        scale = scale.min(f64::from(MAX_SIDE) / ph);
        let area = pw * scale * ph * scale;
        if area > MAX_PIXELS as f64 {
            scale *= (MAX_PIXELS as f64 / area).sqrt();
        }
        let scale = scale as f32;
        let pages = self.pdf.pages();
        let p = pages.get(page).ok_or("There is no such page.")?;
        let cache = RenderCache::new();
        let pixmap = render(
            p,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE },
        );
        let (width, height) = (u32::from(pixmap.width()), u32::from(pixmap.height()));
        let rgba = pixmap.data_as_u8_slice().to_vec();
        if width == 0 || height == 0 || rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err("The page could not be drawn.".into());
        }
        Ok(Raster { width, height, rgba })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-page PDF 200 × 100 points with a black box.
    fn tiny_pdf() -> Vec<u8> {
        let content = "0 0 0 rg 20 20 60 30 re f";
        let objs = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
        ];
        let mut out = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![];
        for (i, o) in objs.iter().enumerate() {
            offsets.push(out.len());
            out.extend(format!("{} 0 obj\n{o}\nendobj\n", i + 1).bytes());
        }
        let xref = out.len();
        out.extend(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).bytes());
        for o in offsets {
            out.extend(format!("{o:010} 00000 n \n").bytes());
        }
        out.extend(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).bytes());
        out
    }

    #[test]
    fn draws_a_page() {
        let s = Sheet::open(tiny_pdf()).unwrap();
        assert_eq!(s.pages(), 1);
        assert!((s.aspect(0).unwrap() - 2.0).abs() < 1e-9 && s.aspect(1).is_none());
        let r = s.draw(0, 400).unwrap();
        assert_eq!((r.width, r.height), (400, 200));
        let px = |x: usize, y: usize| r.rgba.get((y * r.width as usize + x) * 4..(y * r.width as usize + x) * 4 + 4).unwrap().to_vec();
        assert_eq!(px(10, 10), [255, 255, 255, 255], "paper is white");
        assert_eq!(px(100, 130), [0, 0, 0, 255], "the box is black");
    }

    #[test]
    fn refuses_what_is_not_a_pdf() {
        assert!(Sheet::open(vec![]).is_err());
        assert!(Sheet::open(b"hello".to_vec()).is_err());
        assert!(Sheet::open(b"%PDF-1.4 garbage".to_vec()).is_err());
        assert!(Sheet::open(tiny_pdf()).unwrap().draw(5, 100).is_err());
    }
}
