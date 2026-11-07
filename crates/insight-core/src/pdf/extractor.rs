//! PDF text extraction via MuPDF.
//!
//! Extracts per-page digital text (sorted page numbers), appends a
//! trailing `\n` per non-empty page so per-page boundaries are obvious
//! in the merged output, and tracks the byte offset where each page
//! ends.

use std::path::Path;

use anyhow::{Context, Result};
use mupdf::{Document, TextExtractOptions};

use crate::text::{ExtractedText, UnitKind};

/// Extract text from a PDF on disk.
pub fn extract_text(path: &Path) -> Result<ExtractedText> {
    let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;
    extract_text_from_bytes(pdf_bytes)
}

/// Extract text from PDF bytes already in memory.
pub fn extract_text_from_bytes(pdf_bytes: Vec<u8>) -> Result<ExtractedText> {
    let doc = Document::from_bytes(&pdf_bytes, "pdf").context("Failed to parse PDF (MuPDF)")?;

    let page_count = doc.page_count().context("Failed to read PDF page count")?;
    let page_count = usize::try_from(page_count)
        .with_context(|| format!("PDF reported an invalid page count ({page_count})"))?;

    let mut text = String::new();
    let mut units = Vec::with_capacity(page_count);
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
        units.push(text.len());
    }

    tracing::debug!(page_count, text_len = text.len(), "PDF extraction complete");

    Ok(ExtractedText {
        text,
        units,
        unit_kind: UnitKind::Page,
    })
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
        assert_eq!(result.unit_kind, UnitKind::Page);
        assert_eq!(result.units.len(), 1);
        assert!(result
            .text
            .contains("This is a test document with enough characters to count as digital"));
        assert_eq!(result.units[0], result.text.len());
    }

    #[test]
    fn extract_blank_pdf() {
        let result = extract_text_from_bytes(BLANK_PDF.to_vec()).unwrap();
        assert_eq!(result.unit_kind, UnitKind::Page);
        assert!(result.text.is_empty());
        assert_eq!(result.units, vec![0]);
    }

    #[test]
    fn extract_type3_custom_encoding_pdf() {
        let result = extract_text_from_bytes(TYPE3_PDF.to_vec()).unwrap();
        assert_eq!(result.text.trim(), "Search for evidence");
    }
}
