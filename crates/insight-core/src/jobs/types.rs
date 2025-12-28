//! Job types and data structures for the import pipeline.

use iroh_docs::NamespaceId;
use serde::Serialize;

/// Extracted PDF ready for storage
pub struct Extracted {
    /// File name (for display)
    pub name: String,
    /// Collection to add this document to
    pub collection_id: String,
    /// Raw PDF bytes
    pub pdf_bytes: Vec<u8>,
    /// Extracted text content
    pub text: String,
    /// Number of pages
    pub page_count: usize,
    /// Character offset where each page ends (for chunk-to-page mapping)
    pub page_boundaries: Vec<usize>,
}

/// Request to generate embeddings for a document.
/// The embed worker fetches text from storage and stores embeddings back to iroh.
#[derive(Debug, Clone)]
pub struct EmbedRequest {
    /// Document ID
    pub doc_id: String,
    /// Document name (for logging)
    pub name: String,
    /// Namespace containing the document
    pub namespace_id: NamespaceId,
}

/// Event emitted when a document completes the pipeline
#[derive(Debug, Clone, Serialize)]
pub struct DocumentCompleted {
    /// Document ID
    pub doc_id: String,
    /// Collection ID
    pub collection_id: String,
}

/// Event emitted when a document fails
#[derive(Debug, Clone, Serialize)]
pub struct DocumentFailed {
    /// Original file path
    pub path: String,
    /// Error message
    pub error: String,
}
