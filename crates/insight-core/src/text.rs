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

/// 1-indexed number of the unit containing `offset`, or `None` when the
/// source has no recorded units.
pub fn unit_number_at(units: &[usize], offset: usize) -> Option<usize> {
    if units.is_empty() {
        return None;
    }
    Some(
        units
            .iter()
            .position(|&end| offset < end)
            .map_or(units.len(), |index| index + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_number_at_maps_offsets_to_units() {
        let units = vec![10, 20, 30];
        assert_eq!(unit_number_at(&units, 0), Some(1));
        assert_eq!(unit_number_at(&units, 9), Some(1));
        // A boundary offset belongs to the unit that starts there.
        assert_eq!(unit_number_at(&units, 10), Some(2));
        assert_eq!(unit_number_at(&units, 29), Some(3));
        assert_eq!(unit_number_at(&units, 100), Some(3));
        assert_eq!(unit_number_at(&[], 0), None);
    }
}
