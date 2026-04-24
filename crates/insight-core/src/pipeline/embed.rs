//! Document-level embedding generation.
//!
//! Pipeline glue that fetches text from storage, chunks + embeds it via an
//! [`EmbeddingProvider`], and returns the data for storage/indexing. The
//! per-model plumbing (tokenizer, weights, GPU/CPU dispatch) lives in
//! `provider::local::embedding`.

use iroh_docs::NamespaceId;

use crate::manager::ModelManager;
use crate::provider::EmbeddingProvider;
use crate::storage::{DocumentMetadata, EmbeddingChunk, EmbeddingData, Storage};

/// Maximum chunks to send to GPU in one call (to avoid OOM).
const MAX_CHUNKS_PER_GPU_CALL: usize = 256;

/// Generate embeddings for a document.
///
/// Fetches text from storage, chunks it, and embeds it. Returns the data
/// without storing it — callers write to storage when ready.
pub async fn generate_embeddings_data(
    storage: &Storage,
    embedder: &dyn EmbeddingProvider,
    model_id: &str,
    namespace_id: NamespaceId,
    metadata: &DocumentMetadata,
    models: &ModelManager,
) -> anyhow::Result<EmbeddingData> {
    let text_bytes = storage
        .get_document_text(namespace_id, &metadata.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Text not found for document {}", metadata.id))?;

    let text = String::from_utf8(text_bytes)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in document {}: {}", metadata.id, e))?;

    let chunks = embedder.chunk_text(&text).await?;
    if chunks.is_empty() {
        tracing::warn!(doc_id = %metadata.id, "Document has no text to embed");
        return Ok(EmbeddingData {
            model_id: model_id.to_string(),
            dimensions: embedder.dimensions(),
            chunks: vec![],
            created_at: chrono::Utc::now().to_rfc3339(),
        });
    }

    let chunk_offsets = find_chunk_offsets(&text, &chunks);

    tracing::debug!(
        doc_id = %metadata.id,
        chunk_count = chunks.len(),
        "Generating embeddings"
    );

    let chunk_refs: Vec<&str> = chunks.iter().map(|s| s.as_str()).collect();
    let vectors = embed_with_chunking(embedder, &chunk_refs, models).await?;

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

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        chunk_count = embedding_chunks.len(),
        "Generated embeddings"
    );

    Ok(EmbeddingData {
        model_id: model_id.to_string(),
        dimensions: embedder.dimensions(),
        chunks: embedding_chunks,
        created_at: chrono::Utc::now().to_rfc3339(),
    })
}

/// Find the starting byte offset of each chunk in the original text.
fn find_chunk_offsets(text: &str, chunks: &[String]) -> Vec<usize> {
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

/// Embed chunks, splitting into multiple GPU calls if needed to avoid OOM.
/// Touches embedding activity between batches so a long document doesn't
/// race the idle reaper.
async fn embed_with_chunking(
    emb: &dyn EmbeddingProvider,
    chunks: &[&str],
    models: &ModelManager,
) -> anyhow::Result<Vec<Vec<f32>>> {
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
        models.touch_embedding();
    }
    Ok(all_vectors)
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
}
