//! Reads the first sheet of an Excel workbook (`.xlsx`) as CSV text, so every importer that takes CSV also takes a
//! workbook. An `.xlsx` file is a ZIP archive of XML parts; only the parts needed for cell text are read. Nothing
//! here trusts the file: the archive is walked through its central directory, every part is read with a size cap,
//! and the number of rows and columns is capped.
//!
//! Dates in a workbook are stored as day numbers and come through as numbers; type them as text in the sheet.

use std::io::Read;

use flate2::read::DeflateDecoder;

/// Largest a part may be once unpacked.
const MAX_PART: u64 = 32 * 1024 * 1024;
const MAX_ROWS: usize = 20_000;
const MAX_COLUMNS: usize = 100;
const MAX_ENTRIES: usize = 2_000;

/// Whether the bytes start like a ZIP archive (which is what a workbook is).
pub fn looks_like_workbook(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

fn u16_at(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(2)?)?;
    Some(usize::from(u16::from_le_bytes([*s.first()?, *s.get(1)?])))
}

fn u32_at(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_le_bytes([*s.first()?, *s.get(1)?, *s.get(2)?, *s.get(3)?])).ok()
}

struct Entry {
    name: String,
    method: usize,
    compressed: usize,
    offset: usize,
}

/// The parts listed in the archive's central directory.
fn entries(b: &[u8]) -> Option<Vec<Entry>> {
    // The end-of-directory record is within the last 64 KB + 22 bytes.
    let from = b.len().saturating_sub(65_557);
    let tail = b.get(from..)?;
    let end = tail.windows(4).rposition(|w| w == b"PK\x05\x06")?.checked_add(from)?;
    let count = u16_at(b, end.checked_add(10)?)?;
    let mut at = u32_at(b, end.checked_add(16)?)?;
    if count > MAX_ENTRIES {
        return None;
    }
    let mut out = Vec::new();
    for _ in 0..count {
        if b.get(at..at.checked_add(4)?)? != b"PK\x01\x02" {
            return None;
        }
        let method = u16_at(b, at.checked_add(10)?)?;
        let compressed = u32_at(b, at.checked_add(20)?)?;
        let (name_len, extra_len, comment_len) = (u16_at(b, at.checked_add(28)?)?, u16_at(b, at.checked_add(30)?)?, u16_at(b, at.checked_add(32)?)?);
        let offset = u32_at(b, at.checked_add(42)?)?;
        let name_at = at.checked_add(46)?;
        let name = String::from_utf8_lossy(b.get(name_at..name_at.checked_add(name_len)?)?).into_owned();
        out.push(Entry { name, method, compressed, offset });
        at = name_at.checked_add(name_len)?.checked_add(extra_len)?.checked_add(comment_len)?;
    }
    Some(out)
}

/// One part's bytes, unpacked.
fn read_part(b: &[u8], e: &Entry) -> Option<Vec<u8>> {
    if b.get(e.offset..e.offset.checked_add(4)?)? != b"PK\x03\x04" {
        return None;
    }
    let (name_len, extra_len) = (u16_at(b, e.offset.checked_add(26)?)?, u16_at(b, e.offset.checked_add(28)?)?);
    let start = e.offset.checked_add(30)?.checked_add(name_len)?.checked_add(extra_len)?;
    let data = b.get(start..start.checked_add(e.compressed)?)?;
    let mut out = Vec::new();
    match e.method {
        0 => {
            if u64::try_from(data.len()).ok()? > MAX_PART {
                return None;
            }
            out.extend_from_slice(data);
        }
        8 => {
            DeflateDecoder::new(data).take(MAX_PART + 1).read_to_end(&mut out).ok()?;
            if u64::try_from(out.len()).ok()? > MAX_PART {
                return None;
            }
        }
        _ => return None,
    }
    Some(out)
}

fn part(b: &[u8], list: &[Entry], name: &str) -> Option<String> {
    let e = list.iter().find(|e| e.name == name)?;
    String::from_utf8(read_part(b, e)?).ok()
}

/// The 0-based column of a cell reference such as `C7`.
fn column_of(reference: &str) -> Option<usize> {
    let mut col = 0usize;
    let mut any = false;
    for c in reference.chars().take_while(char::is_ascii_alphabetic) {
        col = col.checked_mul(26)?.checked_add(usize::from(u8::try_from(c.to_ascii_uppercase()).ok()?.checked_sub(b'A')?) + 1)?;
        any = true;
    }
    if any { col.checked_sub(1) } else { None }
}

fn text_of(node: roxmltree::Node<'_, '_>) -> String {
    node.descendants().filter(|n| n.has_tag_name("t")).filter_map(|n| n.text()).collect()
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn parse(xml: &str) -> Option<roxmltree::Document<'_>> {
    roxmltree::Document::parse(xml).ok()
}

/// The first sheet as CSV, or why it can't be read.
pub fn to_csv(bytes: &[u8]) -> Result<String, String> {
    let bad = || "That isn't an Excel workbook this program can read. Save it as .xlsx (or as CSV) and try again.".to_string();
    let list = entries(bytes).ok_or_else(bad)?;
    // Which part is the first sheet: the workbook names it by relationship, the relationships give its path.
    let workbook = part(bytes, &list, "xl/workbook.xml").ok_or_else(bad)?;
    let wb = parse(&workbook).ok_or_else(bad)?;
    let rel_id = wb
        .descendants()
        .find(|n| n.has_tag_name("sheet"))
        .and_then(|n| n.attributes().find(|a| a.name() == "id").map(|a| a.value().to_string()))
        .ok_or_else(bad)?;
    let rels = part(bytes, &list, "xl/_rels/workbook.xml.rels").ok_or_else(bad)?;
    let rd = parse(&rels).ok_or_else(bad)?;
    let target = rd
        .descendants()
        .find(|n| n.has_tag_name("Relationship") && n.attribute("Id") == Some(rel_id.as_str()))
        .and_then(|n| n.attribute("Target"))
        .ok_or_else(bad)?;
    let sheet_path = match target.strip_prefix('/') {
        Some(abs) => abs.to_string(),
        None => format!("xl/{target}"),
    };
    let strings: Vec<String> = match part(bytes, &list, "xl/sharedStrings.xml") {
        Some(xml) => {
            let doc = parse(&xml).ok_or_else(bad)?;
            doc.descendants().filter(|n| n.has_tag_name("si")).take(1_000_000).map(text_of).collect()
        }
        None => Vec::new(),
    };
    let sheet_xml = part(bytes, &list, &sheet_path).ok_or_else(bad)?;
    let doc = parse(&sheet_xml).ok_or_else(bad)?;
    let mut out = String::new();
    let mut last_row = 0usize;
    for (n, row) in doc.descendants().filter(|n| n.has_tag_name("row")).enumerate() {
        if n >= MAX_ROWS {
            return Err(format!("The first sheet has more than {MAX_ROWS} rows."));
        }
        // Empty rows between two filled ones are kept so lines stay where they were.
        let number = row.attribute("r").and_then(|r| r.parse::<usize>().ok()).unwrap_or(last_row + 1).min(MAX_ROWS + 1);
        for _ in (last_row + 1)..number {
            out.push('\n');
        }
        last_row = number;
        let mut cells: Vec<String> = Vec::new();
        for (i, c) in row.children().filter(|n| n.has_tag_name("c")).enumerate() {
            let col = c.attribute("r").and_then(column_of).unwrap_or(i);
            if col >= MAX_COLUMNS {
                return Err(format!("The first sheet has more than {MAX_COLUMNS} columns."));
            }
            while cells.len() < col {
                cells.push(String::new());
            }
            let raw = c.children().find(|n| n.has_tag_name("v")).and_then(|n| n.text()).unwrap_or("");
            let value = match c.attribute("t") {
                Some("s") => raw.trim().parse::<usize>().ok().and_then(|i| strings.get(i)).cloned().unwrap_or_default(),
                Some("inlineStr") => c.children().find(|n| n.has_tag_name("is")).map(text_of).unwrap_or_default(),
                Some("b") => (if raw.trim() == "1" { "TRUE" } else { "FALSE" }).to_string(),
                _ => raw.to_string(),
            };
            cells.push(value);
        }
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A stored (uncompressed) ZIP archive of the given parts.
    pub(crate) fn zip(parts: &[(&str, &str)]) -> Vec<u8> {
        build(parts, false)
    }

    fn build(parts: &[(&str, &str)], deflate: bool) -> Vec<u8> {
        use std::io::Write;
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, text) in parts {
            let packed: Vec<u8> = if deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(text.as_bytes()).unwrap();
                e.finish().unwrap()
            } else {
                text.as_bytes().to_vec()
            };
            let data = &packed;
            let method: u8 = if deflate { 8 } else { 0 };
            let offset = u32::try_from(out.len()).unwrap();
            let (n, d) = (u16::try_from(name.len()).unwrap(), u32::try_from(data.len()).unwrap());
            out.extend_from_slice(b"PK\x03\x04");
            out.extend_from_slice(&[20, 0, 0, 0, method, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&d.to_le_bytes());
            out.extend_from_slice(&d.to_le_bytes());
            out.extend_from_slice(&n.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);
            central.extend_from_slice(b"PK\x01\x02");
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0, method, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&d.to_le_bytes());
            central.extend_from_slice(&d.to_le_bytes());
            central.extend_from_slice(&n.to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let at = u32::try_from(out.len()).unwrap();
        let (count, size) = (u16::try_from(parts.len()).unwrap(), u32::try_from(central.len()).unwrap());
        out.extend_from_slice(&central);
        out.extend_from_slice(b"PK\x05\x06\0\0\0\0");
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&at.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    pub(crate) const WORKBOOK: &str = r#"<workbook xmlns="x" xmlns:r="r"><sheets><sheet name="Budget" sheetId="1" r:id="rId1"/><sheet name="Other" sheetId="2" r:id="rId2"/></sheets></workbook>"#;
    pub(crate) const RELS: &str = r#"<Relationships><Relationship Id="rId1" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Target="worksheets/sheet2.xml"/></Relationships>"#;

    #[test]
    fn reads_the_first_sheet_as_csv() {
        let strings = r#"<sst><si><t>03-300</t></si><si><t>Concrete, footings</t></si><si><r><t>Say </t></r><r><t>"hi"</t></r></si></sst>"#;
        let sheet = r#"<worksheet><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="B1" t="s"><v>1</v></c><c r="C1"><v>125000.5</v></c></row>
            <row r="3"><c r="B3" t="inlineStr"><is><t>inline</t></is></c><c r="D3" t="s"><v>2</v></c><c r="E3" t="b"><v>1</v></c></row>
            </sheetData></worksheet>"#;
        let other = "<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>9</v></c></row></sheetData></worksheet>";
        let bytes = zip(&[
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", RELS),
            ("xl/sharedStrings.xml", strings),
            ("xl/worksheets/sheet1.xml", sheet),
            ("xl/worksheets/sheet2.xml", other),
        ]);
        assert!(looks_like_workbook(&bytes));
        assert_eq!(to_csv(&bytes).unwrap(), "03-300,\"Concrete, footings\",125000.5\n\n,inline,,\"Say \"\"hi\"\"\",TRUE\n");
    }

    #[test]
    fn compressed_parts_are_unpacked() {
        let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1"><v>7</v></c></row></sheetData></worksheet>"#;
        let bytes = build(&[("xl/workbook.xml", WORKBOOK), ("xl/_rels/workbook.xml.rels", RELS), ("xl/worksheets/sheet1.xml", sheet)], true);
        assert_eq!(to_csv(&bytes).unwrap(), "7\n");
    }

    #[test]
    fn junk_is_refused_not_a_crash() {
        assert!(to_csv(b"").is_err());
        assert!(to_csv(b"PK\x03\x04 not really").is_err());
        let mut bytes = zip(&[("xl/workbook.xml", WORKBOOK)]);
        assert!(to_csv(&bytes).is_err(), "no relationships or sheet");
        // A directory that points outside the file.
        let n = bytes.len();
        bytes[n - 6..n - 2].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(to_csv(&bytes).is_err());
        // Every prefix and a flipped byte of a real archive stays an error or an answer, never a panic.
        let good = zip(&[("xl/workbook.xml", WORKBOOK), ("xl/_rels/workbook.xml.rels", RELS)]);
        for cut in 0..good.len() {
            let _ = to_csv(&good[..cut]);
        }
        for i in 0..good.len() {
            let mut b = good.clone();
            b[i] ^= 0xFF;
            let _ = to_csv(&b);
        }
        assert!(!looks_like_workbook(b"name,amount"));
    }
}
