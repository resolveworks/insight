//! Embedding worker.
//!
//! Generates embeddings and stores them to iroh, triggering re-indexing via events.
//! All chunks from multiple documents are embedded in a single GPU call.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;
use crate::storage::{EmbeddingChunk, EmbeddingData, Storage};

use super::types::EmbedRequest;
use super::worker::{BatchConfig, Batcher};

/// Maximum documents to batch together for embedding.
const BATCH_SIZE: usize = 16;

/// Maximum time to wait for a full batch before processing a partial one.
const BATCH_TIMEOUT: Duration = Duration::from_millis(200);

/// Maximum chunks to send to GPU in one call (to avoid OOM).
const MAX_CHUNKS_PER_GPU_CALL: usize = 256;

/// Spawns the embedding worker.
///
/// Receives EmbedRequest, generates embeddings, and stores them to iroh.
/// The DocWatcher will pick up the embeddings/* event and trigger re-indexing.
pub fn spawn(
    embedder: Arc<RwLock<Option<Embedder>>>,
    storage: Arc<RwLock<Storage>>,
    embedding_model_id: Arc<RwLock<Option<String>>>,
    cancel: CancellationToken,
) -> mpsc::Sender<EmbedRequest> {
    let (tx, rx) = mpsc::channel::<EmbedRequest>(64);

    let config = BatchConfig {
        max_size: BATCH_SIZE,
        max_wait: BATCH_TIMEOUT,
    };
    let mut batcher = Batcher::new(rx, config);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    tracing::debug!("Embedding worker cancelled");
                    break;
                }

                batch = batcher.next_batch() => {
                    let Some(requests) = batch else {
                        tracing::debug!("Embedding worker shutting down - channel closed");
                        break;
                    };

                    let doc_count = requests.len();
                    tracing::debug!("Processing embedding batch of {} documents", doc_count);

                    // Process batch
                    process_batch(
                        &embedder,
                        &storage,
                        &embedding_model_id,
                        requests,
                    ).await;

                    tracing::debug!("Completed embedding batch of {} documents", doc_count);
                }
            }
        }

        tracing::debug!("Embedding worker stopped");
    });

    tx
}

/// Process a batch of embedding requests.
async fn process_batch(
    embedder: &Arc<RwLock<Option<Embedder>>>,
    storage: &Arc<RwLock<Storage>>,
    embedding_model_id: &Arc<RwLock<Option<String>>>,
    requests: Vec<EmbedRequest>,
) {
    let embedder_guard = embedder.read().await;
    let model_id_guard = embedding_model_id.read().await;

    let (emb, model_id) = match (&*embedder_guard, &*model_id_guard) {
        (Some(emb), Some(model_id)) => (emb, model_id.clone()),
        _ => {
            tracing::debug!("No embedder configured, skipping batch");
            return;
        }
    };

    // Fetch document data for all requests
    let mut docs_to_embed: Vec<DocToEmbed> = Vec::new();

    for request in requests {
        let storage_guard = storage.read().await;

        // Fetch document metadata
        let metadata = match storage_guard
            .get_document(request.namespace_id, &request.doc_id)
            .await
        {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::warn!(doc_id = %request.doc_id, "Document not found for embedding");
                continue;
            }
            Err(e) => {
                tracing::warn!(doc_id = %request.doc_id, error = %e, "Failed to fetch document");
                continue;
            }
        };

        // Fetch text content
        let text_hash: iroh_blobs::Hash = match metadata.text_hash.parse() {
            Ok(h) => h,
            Err(_) => {
                tracing::warn!(doc_id = %request.doc_id, "Invalid text hash");
                continue;
            }
        };

        let text = match storage_guard.get_blob(&text_hash).await {
            Ok(Some(bytes)) => match String::from_utf8(bytes) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(doc_id = %request.doc_id, error = %e, "Invalid UTF-8 in text");
                    continue;
                }
            },
            Ok(None) => {
                tracing::warn!(doc_id = %request.doc_id, "Text blob not found");
                continue;
            }
            Err(e) => {
                tracing::warn!(doc_id = %request.doc_id, error = %e, "Failed to fetch text blob");
                continue;
            }
        };

        docs_to_embed.push(DocToEmbed {
            doc_id: request.doc_id,
            name: request.name,
            namespace_id: request.namespace_id,
            text,
            page_boundaries: metadata.page_boundaries,
        });
    }

    if docs_to_embed.is_empty() {
        return;
    }

    // Chunk all documents and collect metadata
    let mut all_chunks: Vec<String> = Vec::new();
    let mut doc_infos: Vec<DocChunkInfo> = Vec::new();

    for doc in &docs_to_embed {
        let chunks = emb.chunk_text(&doc.text);
        let start_idx = all_chunks.len();
        let chunk_count = chunks.len();
        let chunk_offsets = find_chunk_offsets(&doc.text, &chunks);

        doc_infos.push(DocChunkInfo {
            doc_id: doc.doc_id.clone(),
            name: doc.name.clone(),
            namespace_id: doc.namespace_id,
            page_boundaries: doc.page_boundaries.clone(),
            start_idx,
            chunk_count,
            chunk_offsets,
            chunk_contents: chunks.clone(),
        });

        all_chunks.extend(chunks);
    }

    let total_chunks = all_chunks.len();
    if total_chunks == 0 {
        tracing::debug!("No chunks to embed");
        return;
    }

    tracing::info!(
        doc_count = docs_to_embed.len(),
        total_chunks = total_chunks,
        "Embedding all chunks in single GPU call"
    );

    // Embed all chunks
    let chunk_refs: Vec<&str> = all_chunks.iter().map(|s| s.as_str()).collect();
    let vectors = match embed_with_chunking(emb, &chunk_refs).await {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "Failed to embed chunks");
            return;
        }
    };

    // Store embeddings for each document
    let dimensions = emb.dimensions;
    drop(embedder_guard);
    drop(model_id_guard);

    for info in doc_infos {
        let chunks: Vec<EmbeddingChunk> = (0..info.chunk_count)
            .map(|i| {
                let global_idx = info.start_idx + i;
                let content = info.chunk_contents.get(i).cloned().unwrap_or_default();
                let vector = vectors.get(global_idx).cloned().unwrap_or_default();

                let chunk_start_offset = info.chunk_offsets.get(i).copied().unwrap_or(0);
                let chunk_end_offset = chunk_start_offset + content.len();
                let start_page =
                    crate::pdf::char_offset_to_page(chunk_start_offset, &info.page_boundaries);
                let end_page =
                    crate::pdf::char_offset_to_page(chunk_end_offset, &info.page_boundaries);

                EmbeddingChunk {
                    index: i,
                    content,
                    vector,
                    start_page,
                    end_page,
                }
            })
            .collect();

        let embedding_data = EmbeddingData {
            model_id: model_id.clone(),
            dimensions,
            chunks,
            created_at: chrono::Utc::now().to_rfc3339(),
        };

        // Store embeddings to iroh (triggers embeddings/* event)
        let storage_guard = storage.read().await;
        if let Err(e) = storage_guard
            .store_embeddings(info.namespace_id, &info.doc_id, embedding_data)
            .await
        {
            tracing::error!(
                doc_id = %info.doc_id,
                error = %e,
                "Failed to store embeddings"
            );
        } else {
            tracing::debug!(
                doc_id = %info.doc_id,
                name = %info.name,
                chunk_count = info.chunk_count,
                "Stored embeddings to iroh"
            );
        }
    }
}

/// Document data fetched from storage for embedding.
struct DocToEmbed {
    doc_id: String,
    name: String,
    namespace_id: iroh_docs::NamespaceId,
    text: String,
    page_boundaries: Vec<usize>,
}

/// Metadata about a document's chunks for reassembly after batched embedding.
struct DocChunkInfo {
    doc_id: String,
    name: String,
    namespace_id: iroh_docs::NamespaceId,
    page_boundaries: Vec<usize>,
    start_idx: usize,
    chunk_count: usize,
    chunk_offsets: Vec<usize>,
    chunk_contents: Vec<String>,
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
        match emb.embed_batch(chunk_batch).await {
            Ok(vectors) => all_vectors.extend(vectors),
            Err(e) => {
                tracing::error!("Embedding batch failed: {}", e);
                return Err(e);
            }
        }
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
