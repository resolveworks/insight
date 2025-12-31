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
    model: Option<Arc<Model>>,
    splitter: Box<dyn ChunkSplitter + Send + Sync>,
    /// Vector dimensions produced by this model
    pub dimensions: usize,
}

/// Trait for text chunking to allow mocking
trait ChunkSplitter {
    fn chunk<'a>(&self, text: &'a str) -> Vec<&'a str>;
}

struct TokenizerSplitter(TextSplitter<Tokenizer>);

impl ChunkSplitter for TokenizerSplitter {
    fn chunk<'a>(&self, text: &'a str) -> Vec<&'a str> {
        self.0.chunks(text).collect()
    }
}

struct SimpleSplitter(usize);

impl ChunkSplitter for SimpleSplitter {
    fn chunk<'a>(&self, text: &'a str) -> Vec<&'a str> {
        // Simple character-based chunking for tests
        let text = text.trim();
        if text.is_empty() {
            return vec![];
        }
        if text.len() <= self.0 {
            return vec![text];
        }
        text.as_bytes()
            .chunks(self.0)
            .filter_map(|chunk| {
                let s = std::str::from_utf8(chunk).ok()?;
                if s.trim().is_empty() {
                    None
                } else {
                    Some(s)
                }
            })
            .collect()
    }
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

        tracing::info!("Embedding model loaded: {} ({}D)", hf_repo_id, dimensions);

        Ok(Self {
            model: Some(Arc::new(model)),
            splitter: Box::new(TokenizerSplitter(splitter)),
            dimensions,
        })
    }

    /// Create a mock embedder for testing.
    ///
    /// Returns dummy vectors instead of calling a real model.
    pub fn mock(dimensions: usize) -> Self {
        Self {
            model: None,
            splitter: Box::new(SimpleSplitter(500)),
            dimensions,
        }
    }

    /// Split text into chunks (for display purposes)
    ///
    /// Returns the text chunks that would be used for embedding.
    pub fn chunk_text(&self, content: &str) -> Vec<String> {
        let content = content.trim();
        if content.is_empty() {
            return vec![];
        }
        self.splitter
            .chunk(content)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
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

        // Mock mode: return dummy vector
        let Some(ref model) = self.model else {
            return Ok(vec![0.1; self.dimensions]);
        };

        tracing::debug!(text_len = text.len(), "Embedding single text");

        let start = std::time::Instant::now();
        let result = model
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

        let chunks = self.splitter.chunk(content);

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

        // Mock mode: return dummy vectors
        let Some(ref model) = self.model else {
            return Ok(texts.iter().map(|_| vec![0.1; self.dimensions]).collect());
        };

        tracing::debug!(batch_size = texts.len(), "Embedding batch");

        let start = std::time::Instant::now();

        let request = EmbeddingRequest::builder().add_prompts(texts.iter().map(|s| s.to_string()));

        let result = model
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

// ============================================================================
// Document Embedding Generation
// ============================================================================

use iroh_docs::NamespaceId;

use crate::storage::{DocumentMetadata, EmbeddingChunk, EmbeddingData, Storage};

/// Maximum chunks to send to GPU in one call (to avoid OOM).
const MAX_CHUNKS_PER_GPU_CALL: usize = 256;

/// Generate embeddings for a document and return the data.
///
/// Fetches document text from storage, chunks it, and generates embeddings.
/// Returns the embedding data without storing it.
pub async fn generate_embeddings_data(
    storage: &Storage,
    embedder: &Embedder,
    model_id: &str,
    namespace_id: NamespaceId,
    metadata: &DocumentMetadata,
) -> anyhow::Result<EmbeddingData> {
    // Fetch text content from the files/{id}/text entry
    let text_bytes = storage
        .get_document_text(namespace_id, &metadata.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Text not found for document {}", metadata.id))?;

    let text = String::from_utf8(text_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in document {}: {}", metadata.id, e))?;

    // Chunk the text
    let chunks = embedder.chunk_text(&text);
    if chunks.is_empty() {
        tracing::warn!(doc_id = %metadata.id, "Document has no text to embed");
        return Ok(EmbeddingData {
            model_id: model_id.to_string(),
            dimensions: embedder.dimensions,
            chunks: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    // Find chunk offsets for page mapping
    let chunk_offsets = find_chunk_offsets(&text, &chunks);

    tracing::debug!(
        doc_id = %metadata.id,
        chunk_count = chunks.len(),
        "Generating embeddings"
    );

    // Embed all chunks
    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let vectors = embed_with_chunking(embedder, &chunk_refs).await?;

    // Build embedding chunks with page info
    let embedding_chunks: Vec<EmbeddingChunk> = chunks
        .iter()
        .enumerate()
        .map(|(i, content)| {
            let chunk_start_offset = chunk_offsets.get(i).copied().unwrap_or(0);
            let chunk_end_offset = chunk_start_offset + content.len();
            let start_page =
                crate::pdf::char_offset_to_page(chunk_start_offset, &metadata.page_boundaries);
            let end_page =
                crate::pdf::char_offset_to_page(chunk_end_offset, &metadata.page_boundaries);

            EmbeddingChunk {
                index: i,
                content: content.clone(),
                vector: vectors.get(i).cloned().unwrap_or_default(),
                start_page,
                end_page,
            }
        })
        .collect();

    let chunk_count = embedding_chunks.len();

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        chunk_count,
        "Generated embeddings"
    );

    Ok(EmbeddingData {
        model_id: model_id.to_string(),
        dimensions: embedder.dimensions,
        chunks: embedding_chunks,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Generate embeddings and store them to iroh.
///
/// This stores the embeddings as a blob in iroh, which can be synced to peers.
pub async fn generate_and_store_embeddings(
    storage: &Storage,
    embedder: &Embedder,
    model_id: &str,
    namespace_id: NamespaceId,
    metadata: &DocumentMetadata,
) -> anyhow::Result<EmbeddingData> {
    let embedding_data =
        generate_embeddings_data(storage, embedder, model_id, namespace_id, metadata).await?;

    storage
        .store_embeddings(namespace_id, &metadata.id, embedding_data.clone())
        .await?;

    tracing::debug!(
        doc_id = %metadata.id,
        "Stored embeddings to iroh"
    );

    Ok(embedding_data)
}

/// Find the starting byte offset of each chunk in the original text.
fn find_chunk_offsets(text: &str, chunks: &[String]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(chunks.len());
    let mut search_start_byte = 0;

    for chunk in chunks {
        if let Some(pos) = text[search_start_byte..].find(chunk.as_str()) {
            let absolute_byte = search_start_byte + pos;
            offsets.push(absolute_byte);
            // Advance past most of this chunk for next search (overlap handling)
            search_start_byte = absolute_byte + chunk.len().saturating_sub(100);
            // Ensure we're at a char boundary
            while search_start_byte < text.len() && !text.is_char_boundary(search_start_byte) {
                search_start_byte += 1;
            }
        } else {
            offsets.push(search_start_byte);
        }
    }

    offsets
}

/// Embed chunks, splitting into multiple GPU calls if needed to avoid OOM.
async fn embed_with_chunking(
    emb: &Embedder,
    chunks: &[&str],
) -> Result<Vec<Vec<f32>>, anyhow::Error> {
    if chunks.len() <= MAX_CHUNKS_PER_GPU_CALL {
        return emb.embed_batch(chunks).await;
    }

    tracing::debug!(
        total_chunks = chunks.len(),
        max_per_call = MAX_CHUNKS_PER_GPU_CALL,
        "Splitting embedding into multiple GPU calls"
    );

    let mut all_vectors = Vec::with_capacity(chunks.len());

    for chunk_batch in chunks.chunks(MAX_CHUNKS_PER_GPU_CALL) {
        let vectors = emb.embed_batch(chunk_batch).await?;
        all_vectors.extend(vectors);
    }

    Ok(all_vectors)
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn test_find_chunk_offsets() {
        let text = "Hello world. This is a test. Another sentence here.";
        let chunks = vec!["Hello world.".to_string(), "This is a test.".to_string()];
        let offsets = find_chunk_offsets(text, &chunks);
        assert_eq!(offsets, vec![0, 13]);
    }
}
