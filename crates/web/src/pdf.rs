//! A small PDF writer for printable documents: a title, headings, paragraphs, key facts and tables
//! on US Letter pages, set in the standard Helvetica fonts (so nothing is embedded and there are no
//! dependencies). Text is wrapped and tables break across pages with their header repeated.

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;
const MARGIN: f32 = 54.0;
const CONTENT_W: f32 = PAGE_W - 2.0 * MARGIN;
const BODY: f32 = 10.0;
const LEAD: f32 = 13.0;

/// Helvetica glyph widths for the printable ASCII range (thousandths of the font size).
const WIDTHS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space .. /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0-9
    278, 278, 584, 584, 584, 556, 1015, // : ; < = > ? @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // A-Z
    278, 278, 278, 469, 556, 333, // [ \ ] ^ _ `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // a-z
    334, 260, 334, 584, // { | } ~
];

/// How a column's text is aligned.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub enum Block {
    Heading(String),
    Paragraph(String),
    /// Label and value pairs, labels in bold.
    Facts(Vec<(String, String)>),
    Table(Table),
    Space,
}

pub struct Table {
    pub headers: Vec<String>,
    /// Relative column widths.
    pub widths: Vec<f32>,
    pub aligns: Vec<Align>,
    pub rows: Vec<Vec<String>>,
    /// A last row set in bold (totals).
    pub footer: Option<Vec<String>>,
}

pub struct Doc {
    pub title: String,
    pub subtitle: String,
    /// Printed small at the top of every page (the company's name).
    pub running_head: String,
    pub blocks: Vec<Block>,
}

/// What one character becomes in the PDF text (WinAnsi), or `?`.
fn encode(c: char) -> u8 {
    match c {
        ' '..='~' => c as u8,
        '\u{a0}' => b' ',
        '\u{a1}'..='\u{ff}' => c as u32 as u8,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201c}' => 0x93,
        '\u{201d}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2026}' => 0x85,
        '\u{20ac}' => 0x80,
        '\u{2212}' => b'-',
        '\t' => b' ',
        _ => b'?',
    }
}

fn glyph_width(b: u8, bold: bool) -> f32 {
    let w = match b {
        0x20..=0x7e => WIDTHS.get(usize::from(b) - 0x20).copied().unwrap_or(556),
        0x91..=0x96 => 556,
        0x97 => 1000,
        _ => 556,
    };
    // Bold is a little wider; this keeps wrapping safe without carrying a second table.
    let w = f32::from(w);
    if bold { w * 1.06 } else { w }
}

fn text_width(s: &str, size: f32, bold: bool) -> f32 {
    s.chars().map(|c| glyph_width(encode(c), bold)).sum::<f32>() * size / 1000.0
}

/// Wraps `s` to `width` points; very long words are cut.
fn wrap(s: &str, width: f32, size: f32, bold: bool) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for para in s.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            let mut word = word.to_string();
            while text_width(&word, size, bold) > width && word.chars().count() > 1 {
                // Cut the word at the longest prefix that fits.
                let mut cut = String::new();
                for ch in word.chars() {
                    let mut next = cut.clone();
                    next.push(ch);
                    if text_width(&next, size, bold) > width && !cut.is_empty() {
                        break;
                    }
                    cut = next;
                }
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                let rest: String = word.chars().skip(cut.chars().count()).collect();
                lines.push(cut);
                word = rest;
            }
            let candidate = if line.is_empty() { word.clone() } else { format!("{line} {word}") };
            if text_width(&candidate, size, bold) <= width || line.is_empty() {
                line = candidate;
            } else {
                lines.push(std::mem::take(&mut line));
                line = word;
            }
        }
        lines.push(line);
    }
    lines
}

fn escape(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        match encode(c) {
            b'(' => out.extend_from_slice(b"\\("),
            b')' => out.extend_from_slice(b"\\)"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b => out.push(b),
        }
    }
    out
}

/// One page's drawing commands.
#[derive(Default)]
struct Page {
    ops: Vec<u8>,
}

impl Page {
    fn text(&mut self, x: f32, y: f32, size: f32, bold: bool, s: &str) {
        if s.is_empty() {
            return;
        }
        self.ops.extend_from_slice(format!("BT /{} {size:.1} Tf {x:.2} {y:.2} Td (", if bold { "F2" } else { "F1" }).as_bytes());
        self.ops.extend_from_slice(&escape(s));
        self.ops.extend_from_slice(b") Tj ET\n");
    }

    fn text_right(&mut self, right: f32, y: f32, size: f32, bold: bool, s: &str) {
        let x = right - text_width(s, size, bold);
        self.text(x, y, size, bold, s);
    }

    fn line(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, grey: f32) {
        self.ops.extend_from_slice(format!("{grey:.2} G 0.5 w {x1:.2} {y1:.2} m {x2:.2} {y2:.2} l S\n").as_bytes());
    }

    fn fill(&mut self, x: f32, y: f32, w: f32, h: f32, grey: f32) {
        self.ops.extend_from_slice(format!("{grey:.2} g {x:.2} {y:.2} {w:.2} {h:.2} re f 0 g\n").as_bytes());
    }
}

struct Layout {
    /// Finished pages.
    done: Vec<Page>,
    /// The page being drawn.
    cur: Page,
    y: f32,
}

impl Layout {
    fn new() -> Layout {
        Layout { done: Vec::new(), cur: Page::default(), y: PAGE_H - MARGIN - 18.0 }
    }

    fn page(&mut self) -> &mut Page {
        &mut self.cur
    }

    /// Writes text at the current line.
    fn put(&mut self, x: f32, size: f32, bold: bool, s: &str) {
        let y = self.y;
        self.cur.text(x, y, size, bold, s);
    }

    /// Starts a new page when `h` more points don't fit.
    fn need(&mut self, h: f32) -> bool {
        if self.y - h < MARGIN + 14.0 {
            let full = std::mem::take(&mut self.cur);
            self.done.push(full);
            self.y = PAGE_H - MARGIN - 18.0;
            true
        } else {
            false
        }
    }

    fn into_pages(mut self) -> Vec<Page> {
        self.done.push(self.cur);
        self.done
    }
}

fn table(l: &mut Layout, t: &Table) {
    let total: f32 = t.widths.iter().sum::<f32>().max(0.001);
    let widths: Vec<f32> = t.widths.iter().map(|w| w / total * CONTENT_W).collect();
    let pad = 3.0;
    let cell_lines = |cells: &[String], bold: bool| -> (Vec<Vec<String>>, usize) {
        let wrapped: Vec<Vec<String>> =
            widths.iter().enumerate().map(|(i, w)| wrap(cells.get(i).map(String::as_str).unwrap_or(""), w - 2.0 * pad, BODY, bold)).collect();
        let n = wrapped.iter().map(Vec::len).max().unwrap_or(1).max(1);
        (wrapped, n)
    };
    let draw = |l: &mut Layout, cells: &[String], bold: bool, shade: bool| {
        let (wrapped, n) = cell_lines(cells, bold);
        let h = n as f32 * LEAD + 2.0 * pad - 2.0;
        if l.need(h + 2.0) {
            // A new page repeats the header.
            header_row(l, t, &widths);
        }
        let top = l.y;
        if shade {
            l.page().fill(MARGIN, top - h, CONTENT_W, h, 0.94);
        }
        let mut x = MARGIN;
        for (i, col) in wrapped.iter().enumerate() {
            let w = widths.get(i).copied().unwrap_or(0.0);
            let right = t.aligns.get(i).copied().unwrap_or(Align::Left) == Align::Right;
            for (k, text) in col.iter().enumerate() {
                let y = top - pad - BODY - k as f32 * LEAD + 2.0;
                if right {
                    l.page().text_right(x + w - pad, y, BODY, bold, text);
                } else {
                    l.page().text(x + pad, y, BODY, bold, text);
                }
            }
            x += w;
        }
        l.page().line(MARGIN, top - h, MARGIN + CONTENT_W, top - h, 0.8);
        l.y = top - h;
    };
    // Header.
    {
        let (_, n) = cell_lines(&t.headers, true);
        let h = n as f32 * LEAD + 2.0 * pad;
        l.need(h + LEAD * 3.0);
    }
    header_row(l, t, &widths);
    for r in &t.rows {
        draw(l, r, false, false);
    }
    if let Some(f) = &t.footer {
        draw(l, f, true, true);
    }
    l.y -= 8.0;
}

fn header_row(l: &mut Layout, t: &Table, widths: &[f32]) {
    let pad = 3.0;
    let n = widths
        .iter()
        .enumerate()
        .map(|(i, w)| wrap(t.headers.get(i).map(String::as_str).unwrap_or(""), w - 2.0 * pad, BODY, true).len())
        .max()
        .unwrap_or(1)
        .max(1);
    let h = n as f32 * LEAD + 2.0 * pad - 2.0;
    let top = l.y;
    l.page().fill(MARGIN, top - h, CONTENT_W, h, 0.88);
    let mut x = MARGIN;
    for (i, w) in widths.iter().enumerate() {
        let right = t.aligns.get(i).copied().unwrap_or(Align::Left) == Align::Right;
        for (k, text) in wrap(t.headers.get(i).map(String::as_str).unwrap_or(""), w - 2.0 * pad, BODY, true).iter().enumerate() {
            let y = top - pad - BODY - k as f32 * LEAD + 2.0;
            if right {
                l.page().text_right(x + w - pad, y, BODY, true, text);
            } else {
                l.page().text(x + pad, y, BODY, true, text);
            }
        }
        x += w;
    }
    l.page().line(MARGIN, top - h, MARGIN + CONTENT_W, top - h, 0.5);
    l.y = top - h;
}

thread_local! {
    /// The language documents are written in on this thread (set around the code that builds one).
    static LANGUAGE: std::cell::Cell<crate::i18n::Lang> = const { std::cell::Cell::new(crate::i18n::Lang::En) };
}

/// Runs `build` so that every document rendered on this thread is written in `lang`.
pub fn with_language<R>(lang: crate::i18n::Lang, build: impl FnOnce() -> R) -> R {
    let before = LANGUAGE.with(|l| l.replace(lang));
    let out = build();
    LANGUAGE.with(|l| l.set(before));
    out
}

/// The document with its wording (and dates) in the thread's language.
fn localized(doc: &Doc) -> Option<Doc> {
    let lang = LANGUAGE.with(std::cell::Cell::get);
    if lang == crate::i18n::Lang::En {
        return None;
    }
    let t = |s: &str| crate::i18n::translate_plain(s, lang);
    let row = |r: &[String]| r.iter().map(|c| t(c)).collect::<Vec<_>>();
    Some(Doc {
        title: t(&doc.title),
        subtitle: t(&doc.subtitle),
        running_head: t(&doc.running_head),
        blocks: doc
            .blocks
            .iter()
            .map(|b| match b {
                Block::Space => Block::Space,
                Block::Heading(h) => Block::Heading(t(h)),
                Block::Paragraph(p) => Block::Paragraph(t(p)),
                Block::Facts(f) => Block::Facts(f.iter().map(|(k, v)| (t(k), t(v))).collect()),
                Block::Table(x) => Block::Table(Table {
                    headers: row(&x.headers),
                    widths: x.widths.clone(),
                    aligns: x.aligns.clone(),
                    rows: x.rows.iter().map(|r| row(r)).collect(),
                    footer: x.footer.as_deref().map(row),
                }),
            })
            .collect(),
    })
}

/// Lays the document out and returns the PDF file's bytes.
pub fn render(doc: &Doc) -> Vec<u8> {
    if let Some(translated) = localized(doc) {
        return render_in_place(&translated);
    }
    render_in_place(doc)
}

fn render_in_place(doc: &Doc) -> Vec<u8> {
    let mut l = Layout::new();
    // Title block.
    for (k, line) in wrap(&doc.title, CONTENT_W, 18.0, true).iter().enumerate() {
        let y = l.y - k as f32 * 22.0;
        l.page().text(MARGIN, y, 18.0, true, line);
        if k > 0 {
            l.y -= 22.0;
        }
    }
    l.y -= 20.0;
    if !doc.subtitle.is_empty() {
        for line in wrap(&doc.subtitle, CONTENT_W, 10.5, false) {
            l.put(MARGIN, 10.5, false, &line);
            l.y -= LEAD;
        }
    }
    l.y -= 6.0;
    for b in &doc.blocks {
        match b {
            Block::Space => l.y -= 8.0,
            Block::Heading(h) => {
                l.need(40.0);
                l.y -= 6.0;
                l.put(MARGIN, 12.0, true, h);
                l.y -= 16.0;
            }
            Block::Paragraph(p) => {
                for line in wrap(p, CONTENT_W, BODY, false) {
                    l.need(LEAD);
                    l.put(MARGIN, BODY, false, &line);
                    l.y -= LEAD;
                }
                l.y -= 4.0;
            }
            Block::Facts(rows) => {
                let label_w = rows.iter().map(|(k, _)| text_width(k, BODY, true)).fold(0.0, f32::max).min(CONTENT_W * 0.4) + 10.0;
                for (k, v) in rows {
                    let vl = wrap(v, CONTENT_W - label_w, BODY, false);
                    l.need(LEAD * vl.len() as f32);
                    l.put(MARGIN, BODY, true, k);
                    for line in &vl {
                        l.put(MARGIN + label_w, BODY, false, line);
                        l.y -= LEAD;
                    }
                }
                l.y -= 4.0;
            }
            Block::Table(t) => table(&mut l, t),
        }
    }
    finish(doc, l.into_pages())
}

fn finish(doc: &Doc, mut pages: Vec<Page>) -> Vec<u8> {
    let total = pages.len().max(1);
    if pages.is_empty() {
        pages.push(Page::default());
    }
    for (i, p) in pages.iter_mut().enumerate() {
        p.text(MARGIN, PAGE_H - 36.0, 8.0, false, &doc.running_head);
        p.line(MARGIN, PAGE_H - 40.0, PAGE_W - MARGIN, PAGE_H - 40.0, 0.7);
        p.text_right(PAGE_W - MARGIN, 30.0, 8.0, false, &format!("Page {} of {total}", i + 1));
    }
    let n = pages.len();
    // Objects: 1 catalog, 2 pages, 3 and 4 fonts, 5 info, then a page and its content per page.
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets: Vec<usize> = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: Vec<u8>| {
        offsets.push(out.len());
        let id = offsets.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let kids: String = (0..n).map(|i| format!("{} 0 R", 6 + 2 * i)).collect::<Vec<_>>().join(" ");
    obj(&mut out, format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>").into_bytes());
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    obj(&mut out, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
    let mut info = b"<< /Producer (BuildKiosk) /Title (".to_vec();
    info.extend_from_slice(&escape(&doc.title));
    info.extend_from_slice(b") >>");
    obj(&mut out, info);
    for (i, p) in pages.iter().enumerate() {
        let content_id = 7 + 2 * i;
        obj(
            &mut out,
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>").into_bytes(),
        );
        let mut stream = format!("<< /Length {} >>\nstream\n", p.ops.len()).into_bytes();
        stream.extend_from_slice(&p.ops);
        stream.extend_from_slice(b"\nendstream");
        obj(&mut out, stream);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Doc {
        Doc {
            title: "Other party".into(),
            subtitle: "Status: Signed".into(),
            running_head: "Summit Ridge Builders".into(),
            blocks: vec![
                Block::Heading("Signature".into()),
                Block::Facts(vec![("Signed on".into(), "Oct 31, 2026".into()), ("Signed by".into(), "Sam Sub".into())]),
                Block::Table(Table {
                    headers: vec!["Cost code".into(), "Amount".into()],
                    widths: vec![1.0, 1.0],
                    aligns: vec![Align::Left, Align::Right],
                    rows: vec![vec!["03-300".into(), "1.00".into()]],
                    footer: Some(vec!["Total".into(), "1.00".into()]),
                }),
            ],
        }
    }

    fn has(bytes: &[u8], needle: &str) -> bool {
        bytes.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn a_document_is_written_in_the_language_it_is_asked_for() {
        use crate::i18n::Lang;
        let english = render(&small());
        assert!(has(&english, "Other party") && has(&english, "Signed on") && !has(&english, "Otra parte"));
        // The people's own text and the company name stay as they are; the wording and dates follow the language.
        let spanish = with_language(Lang::Es, || render(&small()));
        assert!(has(&spanish, "Otra parte") && has(&spanish, "Sam Sub") && has(&spanish, "Summit Ridge Builders") && !has(&spanish, "Other party"));
        let french = with_language(Lang::Fr, || render(&small()));
        assert!(has(&french, "Autre partie") && !has(&french, "Other party"));
        // The language is only for the closure: afterwards it is English again.
        assert!(has(&render(&small()), "Other party"));
        // Text with no translation is left alone.
        let odd = with_language(Lang::Nl, || {
            render(&Doc { title: "Zzz nothing like this".into(), subtitle: String::new(), running_head: String::new(), blocks: Vec::new() })
        });
        assert!(has(&odd, "Zzz nothing like this"));
    }

    pub(super) fn sample() -> Doc {
        let rows: Vec<Vec<String>> = (1..=120)
            .map(|i| {
                vec![
                    format!("Item {i} with a fairly long description that has to wrap onto a second line because it is so long"),
                    format!("{}.00", i * 100),
                ]
            })
            .collect();
        Doc {
            title: "Invoice #1 (draft) – October".into(),
            subtitle: "Riverside Medical Office · period ending Oct 31, 2026".into(),
            running_head: "Summit Ridge Builders".into(),
            blocks: vec![
                Block::Heading("Summary".into()),
                Block::Facts(vec![("Total".into(), "$1,234.00".into()), ("Notes".into(), "A very long note ".repeat(30))]),
                Block::Table(Table {
                    headers: vec!["Item".into(), "Amount".into()],
                    widths: vec![4.0, 1.0],
                    aligns: vec![Align::Left, Align::Right],
                    rows,
                    footer: Some(vec!["Total".into(), "726,000.00".into()]),
                }),
                Block::Paragraph("Signed (by hand) \\ café “quoted” — end".into()),
            ],
        }
    }

    #[test]
    fn structure_is_valid() {
        let pdf = render(&sample());
        // One char per byte, so offsets match.
        let text: String = pdf.iter().map(|&b| if b < 128 { char::from(b) } else { '?' }).collect();
        assert!(text.starts_with("%PDF-1.4") && text.trim_end().ends_with("%%EOF"));
        // The cross-reference table points at every object.
        let xref = text.rfind("startxref\n").unwrap();
        let offset: usize = text[xref + 10..].lines().next().unwrap().trim().parse().unwrap();
        assert!(text[offset..].starts_with("xref\n"));
        let count: usize = text[offset..].lines().nth(1).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        for (i, line) in text[offset..].lines().skip(3).take(count - 1).enumerate() {
            let at: usize = line[..10].parse().unwrap();
            assert!(text[at..].starts_with(&format!("{} 0 obj", i + 1)), "object {} at {at}", i + 1);
        }
        assert!(text.matches("/Type /Page ").count() > 3, "a long table spans pages");
        assert!(text.contains("Page 1 of"));
    }

    #[test]
    fn wrapping_respects_width_and_long_words() {
        let lines = wrap("one two three four five six seven eight nine ten", 60.0, 10.0, false);
        assert!(lines.len() > 3 && lines.iter().all(|l| text_width(l, 10.0, false) <= 60.5));
        let cut = wrap(&"x".repeat(200), 100.0, 10.0, false);
        assert!(cut.len() > 5 && cut.iter().all(|l| text_width(l, 10.0, false) <= 100.5));
        assert_eq!(wrap("", 100.0, 10.0, false), vec![String::new()]);
    }

    #[test]
    fn text_is_encoded_and_escaped() {
        assert_eq!(encode('é'), 0xe9);
        assert_eq!(encode('—'), 0x97);
        assert_eq!(encode('日'), b'?');
        assert_eq!(escape("a(b)c\\"), b"a\\(b\\)c\\\\".to_vec());
    }
}

#[cfg(test)]
pub(crate) fn tests_sample() -> Doc {
    tests::sample()
}

#[cfg(test)]
mod dump {
    #[test]
    fn write_sample_for_viewing() {
        if let Ok(dir) = std::env::var("BK_PDF_DUMP") {
            let pdf = super::render(&super::tests_sample());
            let _ = std::fs::write(format!("{dir}/sample.pdf"), pdf);
        }
    }
}
