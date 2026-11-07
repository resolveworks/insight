//! Extracted document text and the addressable units of its source.

use serde::{Deserialize, Serialize};

/// What one addressable unit of a document's source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Page,
}

/// Text extracted from a source, with the addressable units of that source
/// mapped onto it.
#[derive(Debug, Clone)]
pub struct ExtractedText {
    /// Full document text.
    pub text: String,
    /// UTF-8 byte offset in `text` where each unit ends, ascending.
    pub units: Vec<usize>,
    /// What one unit is in the source.
    pub unit_kind: UnitKind,
}
