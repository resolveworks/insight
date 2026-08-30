//! PDF text extraction via lopdf.
//!
//! Extracts per-page digital text (sorted page numbers), appends a
//! trailing `\n` per non-empty page so per-page boundaries are obvious
//! in the merged output, and tracks the character offset where each
//! page ends.

use std::path::Path;

use anyhow::{Context, Result};

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
    let doc = lopdf::Document::load_mem(&pdf_bytes).context("Failed to parse PDF (lopdf)")?;

    let mut pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    pages.sort();
    let page_count = pages.len();

    let mut text = String::new();
    let mut boundaries = Vec::with_capacity(page_count);
    for page_num in &pages {
        let page_text = doc.extract_text(&[*page_num]).unwrap_or_default();
        if !page_text.is_empty() {
            text.push_str(&page_text);
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
    use lopdf::{dictionary, Document, Object, Stream};

    fn create_test_pdf(text: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        let content = format!(
            "BT /F1 12 Tf 100 700 Td ({}) Tj ET",
            text.replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)")
        );
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });

        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        });

        if let Ok(page) = doc.get_object_mut(page_id) {
            if let Object::Dictionary(ref mut dict) = page {
                dict.set("Parent", pages_id);
            }
        }

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);

        let mut buffer = Vec::new();
        doc.save_to(&mut buffer).unwrap();
        buffer
    }

    #[test]
    fn extract_digital_text_pdf() {
        let pdf = create_test_pdf("This is a test document with enough characters to count as digital");
        let result = extract_text_from_bytes(pdf).unwrap();
        assert_eq!(result.page_count, 1);
        assert_eq!(result.page_boundaries.len(), 1);
        assert!(!result.text.is_empty());
        assert!(result.text.contains("test") || result.text.contains("document"));
        assert_eq!(result.page_boundaries[0], result.text.len());
    }

    #[test]
    fn extract_blank_pdf() {
        let pdf = create_test_pdf("");
        let result = extract_text_from_bytes(pdf).unwrap();
        assert_eq!(result.page_count, 1);
        assert!(result.text.is_empty());
        assert_eq!(result.page_boundaries, vec![0]);
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
