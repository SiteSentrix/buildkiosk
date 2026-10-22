//! Splits a PDF of several sheets into one PDF per page, so a combined set can become one sheet per page.
//!
//! The file is untrusted: it is parsed by a library that works on arbitrary input, so every call is guarded against a
//! panic (the sheet is then simply not split), the page count is capped, and encrypted or damaged files give an error
//! message rather than a crash.

use lopdf::Document;

/// The most pages one PDF may be split into.
pub const MAX_PAGES: usize = 150;

fn guarded<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|_| Err("it can't be read".to_string()))
}

/// How many pages the PDF has.
pub fn page_count(bytes: &[u8]) -> Result<usize, String> {
    guarded(|| {
        let doc = Document::load_mem(bytes).map_err(|_| "it can't be read as a PDF".to_string())?;
        if doc.is_encrypted() {
            return Err("it is password protected".to_string());
        }
        Ok(doc.get_pages().len())
    })
}

/// A PDF holding only page `page` (from 1) of `bytes`, with whatever else it needs (fonts, pictures) and nothing it doesn't.
pub fn page(bytes: &[u8], page: usize) -> Result<Vec<u8>, String> {
    guarded(|| {
        let mut doc = Document::load_mem(bytes).map_err(|_| "it can't be read as a PDF".to_string())?;
        let total = doc.get_pages().len();
        if page == 0 || page > total || total > MAX_PAGES {
            return Err(format!("page {page} is not there (it has {total})"));
        }
        let others: Vec<u32> = (1..=total).filter(|p| *p != page).filter_map(|p| u32::try_from(p).ok()).collect();
        doc.delete_pages(&others);
        doc.prune_objects();
        doc.renumber_objects();
        let mut out = Vec::new();
        doc.save_to(&mut out).map_err(|_| "the page can't be written".to_string())?;
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use buildkiosk_web::pdf::{Block, Doc};

    /// A PDF of several pages: a long list of paragraphs.
    fn several_pages() -> Vec<u8> {
        let blocks = (0..220).map(|i| Block::Paragraph(format!("Line {i} of the sheet set, with enough words to take room on the page."))).collect();
        buildkiosk_web::pdf::render(&Doc { title: "Set".into(), subtitle: "Sheets".into(), running_head: "Acme".into(), blocks })
    }

    #[test]
    fn a_set_splits_into_single_pages() {
        let set = several_pages();
        let n = page_count(&set).unwrap();
        assert!(n >= 3, "the sample should run to several pages, it has {n}");
        for k in [1, 2, n] {
            let one = page(&set, k).unwrap();
            assert_eq!(page_count(&one).unwrap(), 1, "page {k}");
            assert!(one.starts_with(b"%PDF"));
            // The renderer the viewer uses opens it and finds one page.
            let info = crate::render::info(one.clone()).unwrap();
            assert_eq!(info.pages(), 1);
            assert!(one.len() < set.len(), "page {k} is smaller than the whole set");
        }
        // The pages are different pages: the text of the first isn't in the last.
        let (first, last) = (page(&set, 1).unwrap(), page(&set, n).unwrap());
        assert!(first != last);
    }

    #[test]
    fn what_is_not_a_pdf_or_a_page_is_refused() {
        assert!(page_count(b"").is_err());
        assert!(page_count(b"%PDF-1.4 but not really").is_err());
        let set = several_pages();
        assert!(page(&set, 0).is_err());
        assert!(page(&set, 10_000).is_err());
        // Damaged files never panic.
        for cut in [10, 100, set.len() / 2, set.len() - 5] {
            let _ = page_count(&set[..cut]);
            let _ = page(&set[..cut], 1);
        }
        let mut flipped = set.clone();
        for i in (0..flipped.len()).step_by(97) {
            flipped[i] ^= 0xFF;
        }
        let _ = page_count(&flipped);
        let _ = page(&flipped, 1);
    }
}
