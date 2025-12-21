//! Embedding model wrapper using mistralrs
//!
//! Provides text embedding functionality for semantic search.
//! Documents are chunked and each chunk gets its own embedding vector.

use std::sync::Arc;

use anyhow::{Context, Result};
use mistralrs::{EmbeddingModelBuilder, EmbeddingRequest, Model};

/// Approximate tokens per chunk (leaving headroom for model's 512 limit)
const CHUNK_SIZE_CHARS: usize = 1600; // ~400 tokens at 4 chars/token average

/// Overlap between chunks to preserve context at boundaries
const CHUNK_OVERLAP_CHARS: usize = 200;

/// Wrapper around mistralrs embedding model
pub struct Embedder {
    model: Arc<Model>,
    /// Vector dimensions produced by this model
    pub dimensions: usize,
}

impl Embedder {
    /// Load an embedding model from HuggingFace
    ///
    /// # Arguments
    /// * `hf_repo_id` - HuggingFace repository ID (e.g., "BAAI/bge-base-en-v1.5")
    /// * `dimensions` - Expected output dimensions for this model
    pub async fn new(hf_repo_id: &str, dimensions: usize) -> Result<Self> {
        tracing::info!("Loading embedding model: {}", hf_repo_id);

        let model = EmbeddingModelBuilder::new(hf_repo_id)
            .with_logging()
            .build()
            .await
            .context("Failed to load embedding model")?;

        tracing::info!(
            "Embedding model loaded: {} ({}D)",
            hf_repo_id,
            dimensions
        );

        Ok(Self {
            model: Arc::new(model),
            dimensions,
        })
    }

    /// Embed a single text (for queries)
    ///
    /// Returns a single vector. Does not chunk - assumes query is short.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let text = text.trim();
        if text.is_empty() {
            // Return zero vector for empty input
            return Ok(vec![0.0; self.dimensions]);
        }

        self.model
            .generate_embedding(text)
            .await
            .context("Failed to generate embedding")
    }

    /// Embed a document, chunking if needed
    ///
    /// Returns multiple vectors, one per chunk. For short documents
    /// that fit in a single chunk, returns a single-element vector.
    pub async fn embed_document(&self, content: &str) -> Result<Vec<Vec<f32>>> {
        let content = content.trim();
        if content.is_empty() {
            // Return single zero vector for empty document
            return Ok(vec![vec![0.0; self.dimensions]]);
        }

        let chunks = chunk_text(content, CHUNK_SIZE_CHARS, CHUNK_OVERLAP_CHARS);

        if chunks.is_empty() {
            return Ok(vec![vec![0.0; self.dimensions]]);
        }

        if chunks.len() == 1 {
            // Single chunk - use simpler API
            let vector = self.embed(chunks[0]).await?;
            return Ok(vec![vector]);
        }

        // Multiple chunks - batch embed
        self.embed_batch(&chunks).await
    }

    /// Batch embed multiple texts
    ///
    /// More efficient than calling embed() multiple times.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        let request = EmbeddingRequest::builder()
            .add_prompts(texts.iter().map(|s| s.to_string()));

        self.model
            .generate_embeddings(request)
            .await
            .context("Failed to generate batch embeddings")
    }
}

/// Split text into overlapping chunks
///
/// Uses character-based chunking with overlap to preserve context.
/// Attempts to break at sentence boundaries when possible.
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<&str> {
    if text.len() <= chunk_size {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let mut end = (start + chunk_size).min(text.len());

        // Try to break at a sentence boundary if not at the end
        if end < text.len() {
            // Look for sentence-ending punctuation followed by space
            if let Some(break_pos) = text[start..end]
                .rfind(". ")
                .or_else(|| text[start..end].rfind("? "))
                .or_else(|| text[start..end].rfind("! "))
                .or_else(|| text[start..end].rfind("\n"))
            {
                // Only use this break if it's not too early in the chunk
                if break_pos > chunk_size / 2 {
                    end = start + break_pos + 1; // Include the punctuation
                }
            }
        }

        chunks.push(&text[start..end]);

        // Move start, accounting for overlap
        if end >= text.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text_short() {
        let text = "This is a short text.";
        let chunks = chunk_text(text, 100, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_text_long() {
        let text = "First sentence here. Second sentence follows. Third sentence comes after. Fourth sentence ends it.";
        let chunks = chunk_text(text, 50, 10);
        assert!(chunks.len() > 1);
        // All text should be covered
        assert!(chunks.iter().any(|c| c.contains("First")));
        assert!(chunks.iter().any(|c| c.contains("Fourth")));
    }

    #[test]
    fn test_chunk_text_preserves_sentence_boundary() {
        let text = "This is sentence one. This is sentence two. This is sentence three.";
        let chunks = chunk_text(text, 45, 5);
        // Should break at sentence boundaries
        for chunk in &chunks {
            // Each chunk should ideally end with punctuation (except possibly last)
            let trimmed = chunk.trim();
            if trimmed.len() >= 20 {
                // Long enough chunks should end at sentence boundary
                assert!(
                    trimmed.ends_with('.')
                        || trimmed.ends_with('?')
                        || trimmed.ends_with('!')
                        || chunk == chunks.last().unwrap(),
                    "Chunk '{}' doesn't end at sentence boundary",
                    chunk
                );
            }
        }
    }

    #[test]
    fn test_chunk_text_empty() {
        let chunks = chunk_text("", 100, 20);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }
}
