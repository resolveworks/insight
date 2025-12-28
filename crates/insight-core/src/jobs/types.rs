//! Job types and data structures for the import pipeline.

use serde::Serialize;

/// Event emitted when a document completes the pipeline (embedded and indexed).
#[derive(Debug, Clone, Serialize)]
pub struct DocumentCompleted {
    /// Document ID
    pub doc_id: String,
    /// Collection ID
    pub collection_id: String,
}

/// Event emitted when a document fails during processing.
#[derive(Debug, Clone, Serialize)]
pub struct DocumentFailed {
    /// Original file path or document ID
    pub path: String,
    /// Error message
    pub error: String,
}
