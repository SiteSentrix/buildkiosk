//! Reads the few fields of a JPEG's EXIF block the photo pages use: the day it was taken and the camera.
//! The bytes are untrusted, so every read is bounds-checked and anything odd gives "nothing found".
//! GPS positions and other personal details are never read.

use buildkiosk_core::Date;

#[derive(Debug, Default, PartialEq)]
pub struct Exif {
    pub taken: Option<Date>,
    /// "Make Model", or whichever of the two is present.
    pub camera: String,
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b: [u8; 2] = self.data.get(at..at.checked_add(2)?)?.try_into().ok()?;
        Some(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b: [u8; 4] = self.data.get(at..at.checked_add(4)?)?.try_into().ok()?;
        Some(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }

    /// The entries of the directory at `offset`: (tag, type, count, position of the 4-byte value field).
    fn entries(&self, offset: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(offset) else { return Vec::new() };
        (0..usize::from(n).min(500))
            .filter_map(|i| {
                let at = offset.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
                Some((self.u16(at)?, self.u16(at.checked_add(2)?)?, self.u32(at.checked_add(4)?)?, at.checked_add(8)?))
            })
            .collect()
    }

    /// An ASCII value (type 2): inline when it fits in four bytes, otherwise at the offset the field holds.
    fn text(&self, count: u32, field: usize) -> Option<String> {
        let len = usize::try_from(count).ok()?.min(200);
        let start = if len <= 4 { field } else { usize::try_from(self.u32(field)?).ok()? };
        let raw = self.data.get(start..start.checked_add(len)?)?;
        let s: String = String::from_utf8_lossy(raw).chars().take_while(|c| *c != '\0').filter(|c| !c.is_control()).collect();
        Some(s.trim().to_string())
    }
}

/// "2026:10:13 09:15:00" → 2026-10-13, for years that make sense.
fn exif_date(s: &str) -> Option<Date> {
    let mut it = s.get(..10)?.split(':');
    let (y, m, d) = (it.next()?.parse::<i32>().ok()?, it.next()?.parse::<u32>().ok()?, it.next()?.parse::<u32>().ok()?);
    if !(1990..=2100).contains(&y) {
        return None;
    }
    Date::from_ymd(y, m, d)
}

/// The TIFF block of a JPEG's first Exif segment.
fn tiff_block(jpeg: &[u8]) -> Option<&[u8]> {
    if jpeg.get(..2)? != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2usize;
    for _ in 0..64 {
        if jpeg.get(at)? != &0xFF {
            return None;
        }
        let marker = *jpeg.get(at.checked_add(1)?)?;
        if marker == 0xDA || marker == 0xD9 {
            return None;
        }
        let len = usize::from(u16::from_be_bytes(jpeg.get(at.checked_add(2)?..at.checked_add(4)?)?.try_into().ok()?));
        let body = jpeg.get(at.checked_add(4)?..at.checked_add(2)?.checked_add(len)?)?;
        if marker == 0xE1 && body.starts_with(b"Exif\0\0") {
            return body.get(6..);
        }
        at = at.checked_add(2)?.checked_add(len)?;
    }
    None
}

pub fn read(jpeg: &[u8]) -> Exif {
    let mut out = Exif::default();
    let Some(data) = tiff_block(jpeg) else { return out };
    let little = match data.get(..2) {
        Some(b"II") => true,
        Some(b"MM") => false,
        _ => return out,
    };
    let t = Tiff { data, little };
    if t.u16(2) != Some(42) {
        return out;
    }
    let Some(ifd0) = t.u32(4).and_then(|o| usize::try_from(o).ok()) else { return out };
    let (mut make, mut model) = (String::new(), String::new());
    let mut exif_ifd = None;
    for (tag, kind, count, field) in t.entries(ifd0) {
        match (tag, kind) {
            (0x010F, 2) => make = t.text(count, field).unwrap_or_default(),
            (0x0110, 2) => model = t.text(count, field).unwrap_or_default(),
            (0x8769, 4) => exif_ifd = t.u32(field).and_then(|o| usize::try_from(o).ok()),
            _ => {}
        }
    }
    if let Some(off) = exif_ifd {
        for (tag, kind, count, field) in t.entries(off) {
            if tag == 0x9003 && kind == 2 {
                out.taken = t.text(count, field).and_then(|s| exif_date(&s));
            }
        }
    }
    // Models often repeat the make ("Acme" and "Acme Phone 12"): don't say it twice.
    out.camera = if model.to_lowercase().starts_with(&make.to_lowercase()) { model } else { format!("{make} {model}").trim().to_string() };
    out.camera.truncate(100);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A JPEG header with an Exif block: make, model and DateTimeOriginal, little-endian.
    fn sample() -> Vec<u8> {
        let mut tiff: Vec<u8> = b"II*\0\x08\0\0\0".to_vec();
        let make = b"Acme\0";
        let model = b"Acme Phone 12\0";
        let date = b"2026:10:13 09:15:00\0";
        // IFD0 at 8: three entries (make, model, Exif pointer), then the next-IFD offset.
        let ifd0_len = 2 + 3 * 12 + 4;
        let make_off = 8 + ifd0_len;
        let model_off = make_off + make.len();
        let exif_ifd_off = model_off + model.len();
        let date_off = exif_ifd_off + 2 + 12 + 4;
        let entry = |tag: u16, kind: u16, count: u32, value: u32| {
            let mut e = Vec::new();
            e.extend(tag.to_le_bytes());
            e.extend(kind.to_le_bytes());
            e.extend(count.to_le_bytes());
            e.extend(value.to_le_bytes());
            e
        };
        tiff.extend(3u16.to_le_bytes());
        tiff.extend(entry(0x010F, 2, make.len() as u32, make_off as u32));
        tiff.extend(entry(0x0110, 2, model.len() as u32, model_off as u32));
        tiff.extend(entry(0x8769, 4, 1, exif_ifd_off as u32));
        tiff.extend(0u32.to_le_bytes());
        tiff.extend(make);
        tiff.extend(model);
        tiff.extend(1u16.to_le_bytes());
        tiff.extend(entry(0x9003, 2, date.len() as u32, date_off as u32));
        tiff.extend(0u32.to_le_bytes());
        tiff.extend(date);
        let mut seg = b"Exif\0\0".to_vec();
        seg.extend(tiff);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend(((seg.len() + 2) as u16).to_be_bytes());
        jpeg.extend(seg);
        jpeg.extend([0xFF, 0xDA, 0, 2]);
        jpeg
    }

    #[test]
    fn reads_the_day_and_the_camera() {
        let e = read(&sample());
        assert_eq!(e.taken.map(|d| d.to_string()).as_deref(), Some("2026-10-13"));
        assert_eq!(e.camera, "Acme Phone 12");
    }

    #[test]
    fn junk_and_truncated_input_give_nothing_and_never_panic() {
        assert_eq!(read(b""), Exif::default());
        assert_eq!(read(b"not a picture"), Exif::default());
        let good = sample();
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
        // Flip every byte in turn: still no panic.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] = bad[i].wrapping_add(0x55);
            let _ = read(&bad);
        }
        let mut huge = sample();
        huge[4..6].copy_from_slice(&[0xFF, 0xFF]);
        let _ = read(&huge);
    }

    #[test]
    fn dates_outside_a_sensible_range_are_ignored() {
        assert!(exif_date("1970:01:01 00:00:00").is_none());
        assert!(exif_date("2026:13:40 00:00:00").is_none());
        assert!(exif_date("garbage").is_none());
    }
}
