//! Embedding worker.
//!
//! Chunks text and generates embeddings, batching for GPU efficiency.
//! All chunks from multiple documents are embedded in a single GPU call.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;

use super::types::{ChunkWithVector, Embedded, Stored};
use super::worker::{BatchConfig, Batcher};

/// Maximum documents to batch together for embedding.
/// Increased since we now batch all chunks in one GPU call.
const BATCH_SIZE: usize = 16;

/// Maximum time to wait for a full batch before processing a partial one.
const BATCH_TIMEOUT: Duration = Duration::from_millis(200);

/// Maximum chunks to send to GPU in one call (to avoid OOM).
/// Most embedding models have diminishing returns beyond ~256 chunks.
const MAX_CHUNKS_PER_GPU_CALL: usize = 256;

/// Spawns the embedding worker.
///
/// Returns a sender to submit stored documents for embedding.
/// Embedded documents are sent to `output_tx`.
pub fn spawn(
    embedder: Arc<RwLock<Option<Embedder>>>,
    cancel: CancellationToken,
    output_tx: mpsc::Sender<Embedded>,
) -> mpsc::Sender<Stored> {
    let (tx, rx) = mpsc::channel::<Stored>(64);

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
                    let Some(docs) = batch else {
                        tracing::debug!("Embedding worker shutting down - channel closed");
                        break;
                    };

                    let doc_count = docs.len();
                    tracing::debug!("Processing embedding batch of {} documents", doc_count);

                    // Process batch with single GPU call
                    let results = process_batch(&embedder, docs).await;

                    // Send results downstream
                    for embedded in results {
                        if output_tx.send(embedded).await.is_err() {
                            tracing::warn!("Failed to send embedded document - channel closed");
                            return;
                        }
                    }

                    tracing::debug!("Completed embedding batch of {} documents", doc_count);
                }
            }
        }

        tracing::debug!("Embedding worker stopped");
    });

    tx
}

/// Metadata about a document's chunks for reassembly after batched embedding.
struct DocChunkInfo {
    doc_id: String,
    name: String,
    collection_id: String,
    page_count: usize,
    /// Starting index in the flattened chunk array
    start_idx: usize,
    /// Number of chunks for this document
    chunk_count: usize,
}

/// Process a batch of documents with a single GPU call for all chunks.
async fn process_batch(
    embedder: &Arc<RwLock<Option<Embedder>>>,
    docs: Vec<Stored>,
) -> Vec<Embedded> {
    let embedder_guard = embedder.read().await;

    match &*embedder_guard {
        Some(emb) => process_batch_with_embedder(emb, docs).await,
        None => process_batch_without_embedder(docs),
    }
}

/// Process batch with embedder - single GPU call for all chunks.
async fn process_batch_with_embedder(emb: &Embedder, docs: Vec<Stored>) -> Vec<Embedded> {
    // Step 1: Chunk all documents and track their positions
    let mut all_chunks: Vec<String> = Vec::new();
    let mut doc_infos: Vec<DocChunkInfo> = Vec::new();

    for doc in &docs {
        let chunks = emb.chunk_text(&doc.text);
        let start_idx = all_chunks.len();
        let chunk_count = chunks.len();

        doc_infos.push(DocChunkInfo {
            doc_id: doc.doc_id.clone(),
            name: doc.name.clone(),
            collection_id: doc.collection_id.clone(),
            page_count: doc.page_count,
            start_idx,
            chunk_count,
        });

        all_chunks.extend(chunks);
    }

    let total_chunks = all_chunks.len();
    if total_chunks == 0 {
        // All documents were empty, return empty Embedded structs
        return doc_infos
            .into_iter()
            .map(|info| Embedded {
                doc_id: info.doc_id,
                name: info.name,
                collection_id: info.collection_id,
                page_count: info.page_count,
                chunks: vec![],
            })
            .collect();
    }

    tracing::info!(
        doc_count = docs.len(),
        total_chunks = total_chunks,
        "Embedding all chunks in single GPU call"
    );

    // Step 2: Embed all chunks in batches (respecting GPU memory limits)
    let chunk_refs: Vec<&str> = all_chunks.iter().map(|s| s.as_str()).collect();
    let all_vectors = embed_with_chunking(emb, &chunk_refs).await;

    // Step 3: Reassemble vectors back to their documents
    doc_infos
        .into_iter()
        .map(|info| {
            let chunks: Vec<ChunkWithVector> = (0..info.chunk_count)
                .map(|i| {
                    let global_idx = info.start_idx + i;
                    ChunkWithVector {
                        index: i,
                        content: all_chunks[global_idx].clone(),
                        vector: all_vectors
                            .as_ref()
                            .ok()
                            .and_then(|vecs| vecs.get(global_idx).cloned()),
                    }
                })
                .collect();

            Embedded {
                doc_id: info.doc_id,
                name: info.name,
                collection_id: info.collection_id,
                page_count: info.page_count,
                chunks,
            }
        })
        .collect()
}

/// Embed chunks, splitting into multiple GPU calls if needed to avoid OOM.
async fn embed_with_chunking(
    emb: &Embedder,
    chunks: &[&str],
) -> Result<Vec<Vec<f32>>, anyhow::Error> {
    if chunks.len() <= MAX_CHUNKS_PER_GPU_CALL {
        // Small enough for single call
        return emb.embed_batch(chunks).await;
    }

    // Split into multiple GPU calls
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

/// Process batch without embedder - just chunk text for keyword search.
fn process_batch_without_embedder(docs: Vec<Stored>) -> Vec<Embedded> {
    docs.into_iter()
        .map(|doc| {
            let chunks = chunk_text_simple(&doc.text)
                .into_iter()
                .enumerate()
                .map(|(i, content)| ChunkWithVector {
                    index: i,
                    content,
                    vector: None,
                })
                .collect();

            Embedded {
                doc_id: doc.doc_id,
                name: doc.name,
                collection_id: doc.collection_id,
                page_count: doc.page_count,
                chunks,
            }
        })
        .collect()
}

/// Simple character-based chunking fallback when no embedder is available.
fn chunk_text_simple(text: &str) -> Vec<String> {
    const CHUNK_SIZE: usize = 2000;
    const OVERLAP: usize = 200;

    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }

    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= CHUNK_SIZE {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < chars.len() {
        let end = (start + CHUNK_SIZE).min(chars.len());
        let chunk: String = chars[start..end].iter().collect();
        chunks.push(chunk);

        if end >= chars.len() {
            break;
        }

        // Move forward with overlap
        start = end.saturating_sub(OVERLAP);
        if start + CHUNK_SIZE >= chars.len() && end < chars.len() {
            // Last chunk - just go to the end
            start = end;
        }
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_text_simple_short() {
        let text = "This is a short text.";
        let chunks = chunk_text_simple(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }

    #[test]
    fn test_chunk_text_simple_empty() {
        let chunks = chunk_text_simple("");
        assert!(chunks.is_empty());

        let chunks = chunk_text_simple("   ");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_text_simple_long() {
        let text = "a".repeat(5000);
        let chunks = chunk_text_simple(&text);

        // Should have multiple chunks
        assert!(chunks.len() > 1);

        // Each chunk should be at most CHUNK_SIZE
        for chunk in &chunks {
            assert!(chunk.len() <= 2000);
        }

        // Combined length should cover the original (accounting for overlap)
        let total_chars: usize = chunks.iter().map(|c| c.len()).sum();
        assert!(total_chars >= text.len());
    }
}
