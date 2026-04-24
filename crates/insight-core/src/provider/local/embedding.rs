//! Local embedding provider using mistralrs.
//!
//! Construction is cheap: the HuggingFace repo id and dimensions are
//! recorded. Tokenizer + weights load on [`Provider::ensure_loaded`].

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use hf_hub::api::tokio::Api;
use mistralrs::{EmbeddingModelBuilder, EmbeddingRequest, Model};
use text_splitter::{ChunkConfig, TextSplitter};
use tokenizers::Tokenizer;

use crate::provider::{EmbeddingProvider, MemoryKind, Provider};

use super::LocalModelState;

/// Max tokens per chunk (most embedding models have 512 token limit).
const CHUNK_MAX_TOKENS: usize = 450;

/// Overlap between chunks in tokens.
const CHUNK_OVERLAP_TOKENS: usize = 50;

/// Weights + tokenizer-aware splitter. Loaded together on first use.
struct LoadedState {
    model: Arc<Model>,
    splitter: TextSplitter<Tokenizer>,
}

pub struct LocalEmbeddingProvider {
    hf_repo_id: String,
    dimensions: usize,
    state: LocalModelState<LoadedState>,
}

impl LocalEmbeddingProvider {
    pub fn new(model_id: &str, hf_repo_id: &str, dimensions: usize) -> Self {
        Self {
            hf_repo_id: hf_repo_id.to_string(),
            dimensions,
            state: LocalModelState::new(model_id),
        }
    }

    async fn loaded(&self) -> Result<Arc<LoadedState>> {
        self.state
            .current()
            .await
            .ok_or_else(|| anyhow::anyhow!("Local embedding model not loaded"))
    }
}

#[async_trait]
impl Provider for LocalEmbeddingProvider {
    fn provider_name(&self) -> &'static str {
        "local"
    }

    fn model_id(&self) -> &str {
        self.state.model_id()
    }

    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Local
    }

    fn coexist(&self) -> bool {
        self.state.coexist()
    }

    fn set_coexist(&self, coexist: bool) {
        self.state.set_coexist(coexist);
    }

    async fn is_loaded(&self) -> bool {
        self.state.is_loaded().await
    }

    async fn ensure_loaded(&self) -> Result<()> {
        let hf_repo_id = self.hf_repo_id.clone();
        let dimensions = self.dimensions;

        self.state
            .get_or_load(|| async move {
                tracing::info!("Loading embedding model: {}", hf_repo_id);

                // Download tokenizer for accurate token-based chunking.
                let api = Api::new().context("Failed to create HuggingFace API")?;
                let tokenizer_path = api
                    .model(hf_repo_id.clone())
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

                let model = EmbeddingModelBuilder::new(&hf_repo_id)
                    .with_logging()
                    .build()
                    .await
                    .context("Failed to load embedding model")?;

                tracing::info!("Embedding model loaded: {} ({}D)", hf_repo_id, dimensions);

                Ok(LoadedState {
                    model: Arc::new(model),
                    splitter,
                })
            })
            .await
            .map(|_| ())
    }

    async fn unload(&self) -> Result<()> {
        if self.state.unload().await {
            tracing::info!("Unloaded embedding model '{}'", self.state.model_id());
        }
        Ok(())
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbeddingProvider {
    fn dimensions(&self) -> usize {
        self.dimensions
    }

    async fn chunk_text(&self, content: &str) -> Result<Vec<String>> {
        let content = content.trim();
        if content.is_empty() {
            return Ok(vec![]);
        }
        self.ensure_loaded().await?;
        let state = self.loaded().await?;
        Ok(state.splitter.chunks(content).map(String::from).collect())
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(vec![0.0; self.dimensions]);
        }

        self.ensure_loaded().await?;
        let state = self.loaded().await?;

        let start = std::time::Instant::now();
        let result = state
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

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        self.ensure_loaded().await?;
        let state = self.loaded().await?;

        let start = std::time::Instant::now();
        let request = EmbeddingRequest::builder().add_prompts(texts.iter().map(|s| s.to_string()));
        let result = state
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
        let text = "Zabezpečenie štandardnej licenčnej podpory aplikačných ý test";
        let splitter = TextSplitter::new(20);
        let chunks: Vec<&str> = splitter.chunks(text).collect();

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
        let joined: String = chunks.join("");
        assert!(joined.contains("First"));
        assert!(joined.contains("Fourth"));
    }
}
