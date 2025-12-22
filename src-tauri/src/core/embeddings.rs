//! Embedding model wrapper using mistralrs
//!
//! Provides text embedding functionality for semantic search.
//! Documents are chunked and each chunk gets its own embedding vector.

use std::sync::Arc;

use anyhow::{Context, Result};
use hf_hub::api::tokio::Api;
use mistralrs::{EmbeddingModelBuilder, EmbeddingRequest, Model};
use text_splitter::{ChunkConfig, TextSplitter};
use tokenizers::Tokenizer;

/// Max tokens per chunk (most embedding models have 512 token limit)
const CHUNK_MAX_TOKENS: usize = 450;

/// Overlap between chunks in tokens
const CHUNK_OVERLAP_TOKENS: usize = 50;

/// Wrapper around mistralrs embedding model
pub struct Embedder {
    model: Arc<Model>,
    splitter: TextSplitter<Tokenizer>,
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

        // Download tokenizer for accurate token-based chunking
        let api = Api::new().context("Failed to create HuggingFace API")?;
        let repo = api.model(hf_repo_id.to_string());
        let tokenizer_path = repo
            .get("tokenizer.json")
            .await
            .context("Failed to download tokenizer.json")?;

        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("{}", e))?;

        let splitter = TextSplitter::new(
            ChunkConfig::new(CHUNK_MAX_TOKENS)
                .with_sizer(tokenizer)
                .with_overlap(CHUNK_OVERLAP_TOKENS)
                .context("Invalid chunk config")?,
        );

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
            splitter,
            dimensions,
        })
    }

    /// Split text into chunks (for display purposes)
    ///
    /// Returns the text chunks that would be used for embedding.
    pub fn chunk_text(&self, content: &str) -> Vec<String> {
        let content = content.trim();
        if content.is_empty() {
            return vec![];
        }
        self.splitter.chunks(content).map(|s| s.to_string()).collect()
    }

    /// Embed a single text (for queries)
    ///
    /// Returns a single vector. Does not chunk - assumes query is short.
    pub async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let text = text.trim();
        if text.is_empty() {
            tracing::debug!("Empty text, returning zero vector");
            return Ok(vec![0.0; self.dimensions]);
        }

        tracing::debug!(
            text_len = text.len(),
            "Embedding single text"
        );

        let start = std::time::Instant::now();
        let result = self
            .model
            .generate_embedding(text)
            .await
            .context("Failed to generate embedding");

        tracing::debug!(
            elapsed_ms = start.elapsed().as_millis(),
            "Embedding complete"
        );

        result
    }

    /// Embed a document, chunking if needed
    ///
    /// Returns multiple vectors, one per chunk. For short documents
    /// that fit in a single chunk, returns a single-element vector.
    pub async fn embed_document(&self, content: &str) -> Result<Vec<Vec<f32>>> {
        let content = content.trim();
        if content.is_empty() {
            tracing::debug!("Empty document, returning single zero vector");
            return Ok(vec![vec![0.0; self.dimensions]]);
        }

        let chunks: Vec<&str> = self.splitter.chunks(content).collect();

        tracing::info!(
            content_len = content.len(),
            chunk_count = chunks.len(),
            "Embedding document"
        );

        if chunks.is_empty() {
            return Ok(vec![vec![0.0; self.dimensions]]);
        }

        let start = std::time::Instant::now();

        let result = if chunks.len() == 1 {
            // Single chunk - use simpler API
            let vector = self.embed(chunks[0]).await?;
            Ok(vec![vector])
        } else {
            // Multiple chunks - batch embed
            self.embed_batch(&chunks).await
        };

        tracing::info!(
            chunk_count = chunks.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "Document embedding complete"
        );

        result
    }

    /// Batch embed multiple texts
    ///
    /// More efficient than calling embed() multiple times.
    pub async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            tracing::debug!("Empty batch, returning empty vec");
            return Ok(vec![]);
        }

        tracing::debug!(
            batch_size = texts.len(),
            "Embedding batch"
        );

        let start = std::time::Instant::now();

        let request = EmbeddingRequest::builder()
            .add_prompts(texts.iter().map(|s| s.to_string()));

        let result = self
            .model
            .generate_embeddings(request)
            .await
            .context("Failed to generate batch embeddings");

        tracing::debug!(
            batch_size = texts.len(),
            elapsed_ms = start.elapsed().as_millis(),
            "Batch embedding complete"
        );

        result
    }
}

#[cfg(test)]
mod tests {
    use text_splitter::TextSplitter;

    #[test]
    fn test_text_splitter_utf8_safe() {
        // This would panic with naive byte slicing
        let text = "Zabezpečenie štandardnej licenčnej podpory aplikačných ý test";
        let splitter = TextSplitter::new(20); // Character-based for test simplicity
        let chunks: Vec<&str> = splitter.chunks(text).collect();

        // Should not panic and all chunks should be valid UTF-8
        assert!(!chunks.is_empty());
        for chunk in &chunks {
            assert!(chunk.is_ascii() || chunk.chars().count() > 0);
        }
    }

    #[test]
    fn test_text_splitter_short_text() {
        let text = "This is a short text.";
        let splitter = TextSplitter::new(100);
        let chunks: Vec<&str> = splitter.chunks(text).collect();

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_text_splitter_long_text() {
        let text = "First sentence here. Second sentence follows. Third sentence comes after. Fourth sentence ends it.";
        let splitter = TextSplitter::new(50);
        let chunks: Vec<&str> = splitter.chunks(text).collect();

        assert!(chunks.len() > 1);
        // All text should be covered
        let joined: String = chunks.join("");
        assert!(joined.contains("First"));
        assert!(joined.contains("Fourth"));
    }
}
