//! Job types and data structures for the import pipeline.

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
}

/// Document stored in iroh, ready for embedding
pub struct Stored {
    /// Document ID (generated UUID)
    pub doc_id: String,
    /// File name (for display)
    pub name: String,
    /// Extracted text content
    pub text: String,
    /// Collection this document belongs to
    pub collection_id: String,
}

/// Chunks with embeddings, ready for indexing
pub struct Embedded {
    /// Document ID
    pub doc_id: String,
    /// File name (for display)
    pub name: String,
    /// Collection this document belongs to
    pub collection_id: String,
    /// Text chunks with their embeddings
    pub chunks: Vec<ChunkWithVector>,
}

/// A text chunk with its optional embedding vector
pub struct ChunkWithVector {
    /// Position in the document (0-indexed)
    pub index: usize,
    /// The chunk text content
    pub content: String,
    /// Pre-computed embedding vector (None if embedder unavailable)
    pub vector: Option<Vec<f32>>,
}

/// Progress update emitted to frontend during import
#[derive(Debug, Clone, Serialize)]
pub struct PipelineProgress {
    /// Number of documents waiting to be processed
    pub pending: usize,
    /// Number of documents currently being extracted
    pub extracting: usize,
    /// Number of documents currently being embedded
    pub embedding: usize,
    /// Number of documents currently being indexed
    pub indexing: usize,
    /// Number of documents successfully completed
    pub completed: usize,
    /// Number of documents that failed
    pub failed: usize,
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
