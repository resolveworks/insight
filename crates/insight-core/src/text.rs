//! Extractions of a document's immutable source and the addressable units of
//! that source.

use serde::{Deserialize, Serialize};

/// Identifies the parser that produced an extraction.
///
/// `name` must not contain `/` or `@`; it is part of an iroh-docs key.
/// `version` is maintained by the parser and must be bumped when its output
/// changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserId {
    pub name: String,
    pub version: String,
}

impl ParserId {
    /// The parser's identity as used in extraction keys: `{name}@{version}`.
    pub fn key(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

/// A byte range in an extraction's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
}

/// One addressable unit of a document's source, with the text spans
/// attributed to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// The 1-indexed source unit (page).
    pub number: usize,
    /// Byte ranges in the extraction text attributed to this unit, ascending
    /// and non-overlapping. Empty when the unit produced no text.
    pub spans: Vec<TextSpan>,
}

/// What one addressable unit of a document's source is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnitKind {
    Page,
}

/// Text produced from a document's source by one parser, with the source's
/// addressable units mapped onto it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extraction {
    /// The parser that produced this extraction.
    pub parser: ParserId,
    /// Full extracted text.
    pub text: String,
    /// What one unit is. `None` when the parser records no unit structure.
    pub unit_kind: Option<UnitKind>,
    /// Units ordered by number.
    pub units: Vec<Unit>,
}

impl Extraction {
    /// Number of pages in the source, when its units are pages.
    pub fn page_count(&self) -> Option<usize> {
        (self.unit_kind == Some(UnitKind::Page)).then_some(self.units.len())
    }
}
