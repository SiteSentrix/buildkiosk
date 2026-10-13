//! A minimal ZIP writer: files are stored without compression, which keeps it small and safe
//! (no decompression is ever done here). Names are UTF-8; sizes stay under 4 GB.

/// One file to put in the archive.
pub struct Entry {
    /// The path inside the archive, with `/` separators.
    pub name: String,
    pub data: Vec<u8>,
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = u32::try_from(i).unwrap_or(0);
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = 0xFFFF_FFFFu32;
    for b in data {
        let idx = usize::from((crc ^ u32::from(*b)).to_le_bytes().first().copied().unwrap_or(0));
        crc = table.get(idx).copied().unwrap_or(0) ^ (crc >> 8);
    }
    !crc
}

fn le16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn le32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// The archive, or `None` when it would be too big for the plain ZIP format.
pub fn build(entries: &[Entry]) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let count = u16::try_from(entries.len()).ok()?;
    for e in entries {
        let offset = u32::try_from(out.len()).ok()?;
        let size = u32::try_from(e.data.len()).ok()?;
        let crc = crc32(&e.data);
        let name = e.name.as_bytes();
        let name_len = u16::try_from(name.len()).ok()?;
        // Local file header: version 20, UTF-8 names (bit 11), stored, no time (1980-01-01).
        le32(&mut out, 0x0403_4B50);
        le16(&mut out, 20);
        le16(&mut out, 0x0800);
        le16(&mut out, 0);
        le16(&mut out, 0);
        le16(&mut out, 0x0021);
        le32(&mut out, crc);
        le32(&mut out, size);
        le32(&mut out, size);
        le16(&mut out, name_len);
        le16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(&e.data);
        // Central directory entry.
        le32(&mut central, 0x0201_4B50);
        le16(&mut central, 20);
        le16(&mut central, 20);
        le16(&mut central, 0x0800);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0x0021);
        le32(&mut central, crc);
        le32(&mut central, size);
        le32(&mut central, size);
        le16(&mut central, name_len);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le16(&mut central, 0);
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(name);
    }
    let dir_offset = u32::try_from(out.len()).ok()?;
    let dir_size = u32::try_from(central.len()).ok()?;
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4B50);
    le16(&mut out, 0);
    le16(&mut out, 0);
    le16(&mut out, count);
    le16(&mut out, count);
    le32(&mut out, dir_size);
    le32(&mut out, dir_offset);
    le16(&mut out, 0);
    Some(out)
}

/// A file name made safe for an archive: no directory parts, no control characters.
pub fn clean_segment(s: &str) -> String {
    let cleaned: String = s.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':') { '_' } else { c }).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').to_string();
    if cleaned.is_empty() { "file".into() } else { cleaned.chars().take(120).collect() }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;

    #[test]
    fn crc_matches_the_known_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn archive_has_the_right_shape() {
        let zip = build(&[Entry { name: "a/b.txt".into(), data: b"hello".to_vec() }, Entry { name: "c.txt".into(), data: Vec::new() }]).unwrap();
        assert_eq!(&zip[..4], b"PK\x03\x04");
        let end = zip.len() - 22;
        assert_eq!(&zip[end..end + 4], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([zip[end + 10], zip[end + 11]]), 2, "two entries");
        let dir_offset = u32::from_le_bytes(zip[end + 16..end + 20].try_into().unwrap()) as usize;
        assert_eq!(&zip[dir_offset..dir_offset + 4], b"PK\x01\x02");
        assert!(zip.windows(5).any(|w| w == b"hello"));
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean_segment("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(clean_segment("   "), "file");
        assert_eq!(clean_segment("a\u{0}b"), "a_b");
    }
}
