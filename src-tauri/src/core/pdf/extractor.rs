use std::path::Path;

use anyhow::{Context, Result};

/// Result of extracting text from a PDF
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// BLAKE3 hash of the PDF file
    pub pdf_hash: String,
    /// Extracted text content
    pub text: String,
    /// BLAKE3 hash of the extracted text
    pub text_hash: String,
    /// Number of pages in the PDF
    pub page_count: usize,
}

/// Extract text from a PDF file
pub fn extract_text(path: &Path) -> Result<ExtractedDocument> {
    let bytes = std::fs::read(path).context("Failed to read PDF file")?;
    let pdf_hash = blake3::hash(&bytes).to_hex().to_string();

    let doc = lopdf::Document::load_mem(&bytes).context("Failed to parse PDF")?;

    let pages: Vec<u32> = doc.get_pages().keys().cloned().collect();
    let page_count = pages.len();

    let text = doc
        .extract_text(&pages)
        .context("Failed to extract text from PDF")?;

    let text_hash = blake3::hash(text.as_bytes()).to_hex().to_string();

    tracing::debug!(
        "Extracted {} chars from {} pages, pdf_hash={}, text_hash={}",
        text.len(),
        page_count,
        &pdf_hash[..8],
        &text_hash[..8]
    );

    Ok(ExtractedDocument {
        pdf_hash,
        text,
        text_hash,
        page_count,
    })
}
