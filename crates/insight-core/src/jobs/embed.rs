//! Embedding generation functions.
//!
//! Generates embeddings for document text. Can either return the data directly
//! (for local imports) or store to iroh (for sync scenarios).

use iroh_docs::NamespaceId;

use crate::embeddings::Embedder;
use crate::storage::{DocumentMetadata, EmbeddingChunk, EmbeddingData, Storage};

/// Maximum chunks to send to GPU in one call (to avoid OOM).
const MAX_CHUNKS_PER_GPU_CALL: usize = 256;

/// Generate embeddings for a document and return the data.
///
/// Fetches document text from storage, chunks it, and generates embeddings.
/// Returns the embedding data without storing it.
///
/// This is the core function used by both direct imports and sync handlers.
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
/// Use this when you need embeddings to persist in the distributed storage.
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

    #[test]
    fn test_find_chunk_offsets() {
        let text = "Hello world. This is a test. Another sentence here.";
        let chunks = vec!["Hello world.".to_string(), "This is a test.".to_string()];
        let offsets = find_chunk_offsets(text, &chunks);
        assert_eq!(offsets, vec![0, 13]);
    }
}
