//! Extractions of a document's immutable source and the addressable units of
//! that source.

use serde::{Deserialize, Serialize};

/// Identifies the parser that produced an extraction.
///
/// `name` must not contain `/`; it is part of an iroh-docs key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParserId {
    pub name: String,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// Convert a zero-based character offset to a UTF-8 byte offset.
///
/// Returns `Some(text.len())` for an offset equal to the character count.
pub(crate) fn char_to_byte(text: &str, char_offset: usize) -> Option<usize> {
    if char_offset == 0 {
        return Some(0);
    }
    if let Some((byte, _)) = text.char_indices().nth(char_offset) {
        return Some(byte);
    }
    (char_offset == text.chars().count()).then_some(text.len())
}

/// An inclusive range of source unit numbers (pages) in an extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceRange {
    /// First source unit number.
    pub start: usize,
    /// Last source unit number.
    pub end: usize,
}

/// Inclusive source unit numbers (pages) covering a character range.
///
/// `text` is the text the character offsets index into; it is normally the
/// extraction's own text. Returns `None` when the range is empty or no unit
/// span overlaps it.
pub fn source_range(
    extraction: &Extraction,
    text: &str,
    start_char: usize,
    end_char: usize,
) -> Option<SourceRange> {
    let start_byte = char_to_byte(text, start_char)?;
    let end_byte = char_to_byte(text, end_char)?;
    if end_byte <= start_byte {
        return None;
    }

    let mut range: Option<SourceRange> = None;
    for unit in &extraction.units {
        let overlaps = unit
            .spans
            .iter()
            .any(|span| span.start < end_byte && span.end > start_byte);
        if overlaps {
            match &mut range {
                Some(range) => range.end = unit.number,
                None => {
                    range = Some(SourceRange {
                        start: unit.number,
                        end: unit.number,
                    })
                }
            }
        }
    }
    range
}
