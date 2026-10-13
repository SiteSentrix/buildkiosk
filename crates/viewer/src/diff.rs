//! Two revisions of a page laid over each other: ink only on the new one is red, ink only on the
//! old one is blue, ink on both is grey. The same picture the server draws for its own viewer.

use crate::geom::MAX_PIXELS;
use crate::sheet::Raster;

const RED: [u8; 3] = [220, 30, 30];
const BLUE: [u8; 3] = [30, 90, 220];
const GREY: [u8; 3] = [190, 190, 190];

/// Whether the pixel at (`x`, `y`) is dark enough to be ink; anything off the edge is paper.
fn ink(r: &Raster, x: u32, y: u32) -> bool {
    if x >= r.width || y >= r.height {
        return false;
    }
    let at = (y as usize * r.width as usize + x as usize) * 4;
    match r.rgba.get(at..at + 4) {
        Some([red, green, blue, _]) => (2126 * u32::from(*red) + 7152 * u32::from(*green) + 722 * u32::from(*blue)) / 10_000 < 160,
        _ => false,
    }
}

/// The differences between `old` and `new`, as big as the larger of the two.
pub fn overlay(old: &Raster, new: &Raster) -> Result<Raster, String> {
    let (width, height) = (old.width.max(new.width), old.height.max(new.height));
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err("These sheets are too big to compare here.".into());
    }
    let mut rgba = vec![255u8; width as usize * height as usize * 4];
    for y in 0..height {
        for x in 0..width {
            let rgb = match (ink(old, x, y), ink(new, x, y)) {
                (true, true) => GREY,
                (false, true) => RED,
                (true, false) => BLUE,
                (false, false) => continue,
            };
            let at = (y as usize * width as usize + x as usize) * 4;
            if let Some(px) = rgba.get_mut(at..at + 3) {
                px.copy_from_slice(&rgb);
            }
        }
    }
    Ok(Raster { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A raster of `w` × `h` pixels, white with the given pixels black.
    fn picture(w: u32, h: u32, black: &[(u32, u32)]) -> Raster {
        let mut rgba = vec![255u8; (w * h * 4) as usize];
        for (x, y) in black {
            let at = ((y * w + x) * 4) as usize;
            rgba[at..at + 3].copy_from_slice(&[0, 0, 0]);
        }
        Raster { width: w, height: h, rgba }
    }

    fn px(r: &Raster, x: u32, y: u32) -> [u8; 3] {
        let at = ((y * r.width + x) * 4) as usize;
        [r.rgba[at], r.rgba[at + 1], r.rgba[at + 2]]
    }

    #[test]
    fn shows_what_moved() {
        let old = picture(4, 4, &[(0, 0), (1, 1)]);
        let new = picture(4, 4, &[(1, 1), (3, 3)]);
        let d = overlay(&old, &new).unwrap();
        assert_eq!((px(&d, 0, 0), px(&d, 1, 1), px(&d, 3, 3), px(&d, 2, 2)), (BLUE, GREY, RED, [255, 255, 255]));
    }

    #[test]
    fn different_sizes_count_the_missing_part_as_paper() {
        let d = overlay(&picture(2, 2, &[(1, 1)]), &picture(4, 3, &[(3, 2)])).unwrap();
        assert_eq!((d.width, d.height), (4, 3));
        assert_eq!((px(&d, 1, 1), px(&d, 3, 2)), (BLUE, RED));
    }

    #[test]
    fn refuses_what_is_too_big_or_empty() {
        let huge = Raster { width: 5000, height: 5000, rgba: Vec::new() };
        assert!(overlay(&huge, &huge).is_err());
        assert!(overlay(&picture(0, 0, &[]), &picture(0, 0, &[])).is_err());
    }
}
