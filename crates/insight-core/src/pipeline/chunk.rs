//! Char-based text chunking.
//!
//! Splits extracted document text into overlapping chunks for keyword
//! indexing. Uses `text_splitter` with a char-based config (~1500 chars
//! per chunk, ~200 char overlap).

use std::sync::LazyLock;

const CHUNK_SIZE: usize = 1500;
const CHUNK_OVERLAP: usize = 200;

static SPLITTER: LazyLock<text_splitter::TextSplitter<text_splitter::ChunkConfig>> =
    LazyLock::new(|| {
        text_splitter::TextSplitter::new(
            text_splitter::ChunkConfig::new(CHUNK_SIZE)
                .with_overlap(CHUNK_OVERLAP)
                .expect("overlap must be smaller than chunk size"),
        )
    });

/// Split text into overlapping chunks.
///
/// Trims the input first; empty (or whitespace-only) input yields no
/// chunks.
pub fn chunk_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    SPLITTER.chunks(trimmed).map(str::to_string).collect()
}

/// Find the starting byte offset of each chunk in the original text.
pub(crate) fn find_chunk_offsets(text: &str, chunks: &[String]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(chunks.len());
    let mut search_start_byte = 0;

    for chunk in chunks {
        if let Some(pos) = text[search_start_byte..].find(chunk.as_str()) {
            let absolute_byte = search_start_byte + pos;
            offsets.push(absolute_byte);
            search_start_byte = absolute_byte + chunk.len().saturating_sub(100);
            while search_start_byte < text.len() && !text.is_char_boundary(search_start_byte) {
                search_start_byte += 1;
            }
        } else {
            offsets.push(search_start_byte);
        }
    }

    offsets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_chunk_offsets() {
        let text = "Hello world. This is a test. Another sentence here.";
        let chunks = vec!["Hello world.".to_string(), "This is a test.".to_string()];
        let offsets = find_chunk_offsets(text, &chunks);
        assert_eq!(offsets, vec![0, 13]);
    }

    #[test]
    fn test_chunk_text_short() {
        let chunks = chunk_text("Hello world. This is a test.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Hello world"));
    }

    #[test]
    fn test_chunk_text_empty() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n\t  ").is_empty());
    }

    #[test]
    fn test_chunk_text_long() {
        let text = "word ".repeat(2000); // ~10k chars
        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            // Allow some tolerance around the configured size.
            assert!(chunk.len() <= CHUNK_SIZE + CHUNK_OVERLAP + 64);
        }
    }

    #[test]
    fn test_chunk_text_utf8_safety() {
        let text = "日本語のテキストです。".repeat(500);
        let chunks = chunk_text(&text);
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            // Every chunk must be valid UTF-8 (String guarantees this, but
            // also check it's not truncated mid-boundary lossily).
            assert!(!chunk.contains('\u{FFFD}'));
        }
    }
}
