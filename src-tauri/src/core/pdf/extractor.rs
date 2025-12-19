use std::path::Path;

use anyhow::{Context, Result};

/// Result of extracting text from a PDF
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// Raw PDF bytes
    pub pdf_bytes: Vec<u8>,
    /// Extracted text content
    pub text: String,
    /// Number of pages in the PDF
    pub page_count: usize,
}

/// Extract text from a PDF file
pub fn extract_text(path: &Path) -> Result<ExtractedDocument> {
    let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;

    let doc = lopdf::Document::load_mem(&pdf_bytes).context("Failed to parse PDF")?;

    let pages: Vec<u32> = doc.get_pages().keys().cloned().collect();
    let page_count = pages.len();

    let text = doc
        .extract_text(&pages)
        .context("Failed to extract text from PDF")?;

    tracing::debug!("Extracted {} chars from {} pages", text.len(), page_count);

    Ok(ExtractedDocument {
        pdf_bytes,
        text,
        page_count,
    })
}
