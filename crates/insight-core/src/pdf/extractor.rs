//! PDF text extraction via MuPDF.
//!
//! Extracts per-page digital text (sorted page numbers), appends a
//! trailing `\n` per non-empty page so per-page boundaries are obvious
//! in the merged output, and tracks the character offset where each
//! page ends.

use std::path::Path;

use anyhow::{Context, Result};
use mupdf::{Document, TextExtractOptions};

/// Result of extracting a PDF.
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    pub page_count: usize,
    /// Full-document text (pages concatenated, each ending in `\n`).
    pub text: String,
    /// Character offset where each page ends (for chunk-to-page mapping).
    pub page_boundaries: Vec<usize>,
}

/// Extract text from a PDF on disk.
pub fn extract_text(path: &Path) -> Result<ExtractedDocument> {
    let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;
    extract_text_from_bytes(pdf_bytes)
}

/// Extract text from PDF bytes already in memory.
pub fn extract_text_from_bytes(pdf_bytes: Vec<u8>) -> Result<ExtractedDocument> {
    let doc = Document::from_bytes(&pdf_bytes, "pdf").context("Failed to parse PDF (MuPDF)")?;

    let page_count = doc.page_count().context("Failed to read PDF page count")?;
    let page_count = usize::try_from(page_count)
        .with_context(|| format!("PDF reported an invalid page count ({page_count})"))?;

    let mut text = String::new();
    let mut boundaries = Vec::with_capacity(page_count);
    for (index, page_number) in (0..page_count).enumerate() {
        let page_number = i32::try_from(page_number).context("PDF page index out of range")?;
        let page = doc
            .load_page(page_number)
            .with_context(|| format!("Failed to load page {}", index + 1))?;
        let page_text = page
            .text(TextExtractOptions::default())
            .with_context(|| format!("Failed to extract text from page {}", index + 1))?;

        // MuPDF ends a page's text with a blank line for block separation —
        // treat whitespace-only pages as blank (no text, no boundary advance).
        let page_text = page_text.trim_end_matches(['\r', '\n']);
        if !page_text.trim().is_empty() {
            text.push_str(page_text);
            if !text.ends_with('\n') {
                text.push('\n');
            }
        }
        boundaries.push(text.len());
    }

    tracing::debug!(page_count, text_len = text.len(), "PDF extraction complete");

    Ok(ExtractedDocument {
        page_count,
        text,
        page_boundaries: boundaries,
    })
}

/// Given a character offset in the merged full-document text, find which
/// page it falls on (1-indexed). Pages whose end offset is `> offset` are
/// matched; if `offset` is past the last boundary, the last page is
/// returned.
pub fn char_offset_to_page(offset: usize, page_boundaries: &[usize]) -> usize {
    for (i, &boundary) in page_boundaries.iter().enumerate() {
        if offset < boundary {
            return i + 1;
        }
    }
    page_boundaries.len().max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_PDF: &[u8] = include_bytes!("../../tests/fixtures/text.pdf");
    const BLANK_PDF: &[u8] = include_bytes!("../../tests/fixtures/blank.pdf");
    /// Type 3 font with custom `/Differences` glyph names: only the font's
    /// `/ToUnicode` CMap yields real text.
    const TYPE3_PDF: &[u8] = include_bytes!("../../tests/fixtures/type3.pdf");

    #[test]
    fn extract_digital_text_pdf() {
        let result = extract_text_from_bytes(TEXT_PDF.to_vec()).unwrap();
        assert_eq!(result.page_count, 1);
        assert_eq!(result.page_boundaries.len(), 1);
        assert!(result
            .text
            .contains("This is a test document with enough characters to count as digital"));
        assert_eq!(result.page_boundaries[0], result.text.len());
    }

    #[test]
    fn extract_blank_pdf() {
        let result = extract_text_from_bytes(BLANK_PDF.to_vec()).unwrap();
        assert_eq!(result.page_count, 1);
        assert!(result.text.is_empty());
        assert_eq!(result.page_boundaries, vec![0]);
    }

    #[test]
    fn extract_type3_custom_encoding_pdf() {
        let result = extract_text_from_bytes(TYPE3_PDF.to_vec()).unwrap();
        assert_eq!(result.text.trim(), "Search for evidence");
    }

    #[test]
    fn char_offset_to_page_basic() {
        let boundaries = vec![10, 20, 30];
        assert_eq!(char_offset_to_page(0, &boundaries), 1);
        assert_eq!(char_offset_to_page(9, &boundaries), 1);
        assert_eq!(char_offset_to_page(10, &boundaries), 2);
        assert_eq!(char_offset_to_page(29, &boundaries), 3);
        assert_eq!(char_offset_to_page(100, &boundaries), 3);
    }
}
