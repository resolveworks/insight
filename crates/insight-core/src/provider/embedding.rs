//! Embedding provider role trait.
//!
//! Embedding providers turn text into dense vectors used for hybrid search.
//! Implementations are free to chunk as they see fit (local models use a
//! tokenizer-aware splitter; a remote service could use a character budget).

use anyhow::Result;
use async_trait::async_trait;

use super::Provider;

/// Embedding role trait. Extends [`Provider`] with chunking + vector output.
#[async_trait]
pub trait EmbeddingProvider: Provider {
    /// Vector dimensions this model produces.
    fn dimensions(&self) -> usize;

    /// Split text into chunks suitable for this model.
    ///
    /// Async because the implementation may need to load a tokenizer to
    /// produce the split (see [`crate::provider::Provider::ensure_loaded`]).
    async fn chunk_text(&self, content: &str) -> Result<Vec<String>>;

    /// Embed a single short text (e.g. a query).
    async fn embed(&self, text: &str) -> Result<Vec<f32>>;

    /// Batch embed multiple texts. More efficient than calling [`embed`]
    /// in a loop — implementations may fan out to the model in one call.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
}
