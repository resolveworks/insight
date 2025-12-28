//! Document watcher that subscribes to iroh events and triggers indexing.
//!
//! Watches for two event types:
//! - `files/*` events: Queue document for embedding generation
//! - `embeddings/*` events: Index document with embeddings (both keyword and semantic search)

use std::sync::Arc;

use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::NamespaceId;
use milli::update::IndexerConfig;
use milli::Index;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::search::{self, ChunkToIndex};
use crate::storage::{EmbeddingData, LiveEvent, Storage};

use super::types::{DocumentCompleted, EmbedRequest};

/// Shared context for document watchers.
///
/// Groups resources that are shared across all watcher instances.
#[derive(Clone)]
pub struct WatcherContext {
    pub storage: Arc<RwLock<Storage>>,
    pub index: Arc<RwLock<Index>>,
    pub indexer_config: Arc<Mutex<IndexerConfig>>,
    pub embed_tx: mpsc::Sender<EmbedRequest>,
    pub completed_tx: mpsc::Sender<DocumentCompleted>,
    pub current_model_id: Arc<RwLock<Option<String>>>,
}

/// Watches a namespace for document events and triggers indexing.
pub struct DocWatcher {
    cancel: CancellationToken,
}

impl DocWatcher {
    /// Spawn a watcher for a specific namespace.
    ///
    /// Handles two event types:
    /// - `files/*`: Queue for embedding generation
    /// - `embeddings/*`: Index document with embeddings
    pub fn spawn(
        namespace_id: NamespaceId,
        ctx: WatcherContext,
        cancel: CancellationToken,
    ) -> Self {
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            if let Err(e) = run_watcher(namespace_id, ctx, cancel_clone.clone()).await {
                if !cancel_clone.is_cancelled() {
                    tracing::error!(
                        namespace = %namespace_id,
                        error = %e,
                        "DocWatcher error"
                    );
                }
            }
        });

        Self { cancel }
    }

    /// Stop the watcher.
    pub fn stop(&self) {
        self.cancel.cancel();
    }
}

async fn run_watcher(
    namespace_id: NamespaceId,
    ctx: WatcherContext,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let collection_id = namespace_id.to_string();

    // Subscribe to namespace events
    let stream = {
        let storage = ctx.storage.read().await;
        storage.subscribe(namespace_id).await?
    };
    tokio::pin!(stream);

    tracing::info!(namespace = %namespace_id, "DocWatcher started");

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                tracing::debug!(namespace = %namespace_id, "DocWatcher cancelled");
                break;
            }

            event = stream.next() => {
                match event {
                    Some(Ok(live_event)) => {
                        if let Err(e) = handle_event(
                            &live_event,
                            &collection_id,
                            namespace_id,
                            &ctx,
                        ).await {
                            tracing::warn!(
                                namespace = %namespace_id,
                                error = %e,
                                "Failed to handle event"
                            );
                        }
                    }
                    Some(Err(e)) => {
                        tracing::warn!(
                            namespace = %namespace_id,
                            error = %e,
                            "Event stream error"
                        );
                    }
                    None => {
                        tracing::debug!(namespace = %namespace_id, "Event stream ended");
                        break;
                    }
                }
            }
        }
    }

    tracing::info!(namespace = %namespace_id, "DocWatcher stopped");
    Ok(())
}

async fn handle_event(
    event: &LiveEvent,
    collection_id: &str,
    namespace_id: NamespaceId,
    ctx: &WatcherContext,
) -> anyhow::Result<()> {
    // We care about InsertLocal and InsertRemote
    let entry = match event {
        LiveEvent::InsertLocal { entry } => entry,
        LiveEvent::InsertRemote { entry, .. } => entry,
        _ => return Ok(()),
    };

    let key = entry.key();
    let key_str = String::from_utf8_lossy(key);

    // Route to appropriate handler based on key prefix
    if key_str.starts_with("files/") {
        handle_document_event(&key_str, entry.content_hash(), namespace_id, ctx).await
    } else if key_str.starts_with("embeddings/") {
        handle_embedding_event(
            &key_str,
            entry.content_hash(),
            collection_id,
            namespace_id,
            ctx,
        )
        .await
    } else {
        // Ignore other keys (_collection, _hash_index, etc.)
        Ok(())
    }
}

/// Handle a document event (files/{doc_id}).
/// Queue for embedding generation - indexing happens when embeddings arrive.
async fn handle_document_event(
    key_str: &str,
    content_hash: Hash,
    namespace_id: NamespaceId,
    ctx: &WatcherContext,
) -> anyhow::Result<()> {
    let doc_id = key_str.strip_prefix("files/").unwrap_or(key_str);
    tracing::debug!(doc_id = %doc_id, "Processing document event");

    // Fetch document metadata to get the name
    let storage_guard = ctx.storage.read().await;
    let metadata_bytes = storage_guard
        .get_blob(&content_hash)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Metadata blob not found for {}", doc_id))?;
    let metadata: crate::storage::DocumentMetadata = serde_json::from_slice(&metadata_bytes)?;
    drop(storage_guard);

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        "Document stored, queuing for embedding"
    );

    // Queue for embedding generation
    let embed_request = EmbedRequest {
        doc_id: metadata.id,
        name: metadata.name,
        namespace_id,
    };
    ctx.embed_tx
        .send(embed_request)
        .await
        .map_err(|_| anyhow::anyhow!("Embed channel closed"))?;

    Ok(())
}

/// Handle an embedding event (embeddings/{doc_id}/{model_id}).
/// Index document with embeddings if model matches.
async fn handle_embedding_event(
    key_str: &str,
    content_hash: Hash,
    collection_id: &str,
    namespace_id: NamespaceId,
    ctx: &WatcherContext,
) -> anyhow::Result<()> {
    // Parse key: embeddings/{doc_id}/{model_id}
    let parts: Vec<&str> = key_str.split('/').collect();
    if parts.len() != 3 {
        tracing::warn!(key = %key_str, "Invalid embeddings key format");
        return Ok(());
    }
    let doc_id = parts[1];
    let event_model_id = parts[2];

    // Check if model matches our current model
    let model_guard = ctx.current_model_id.read().await;
    let should_index = match &*model_guard {
        Some(current) => current == event_model_id,
        None => false, // No embedder configured, skip
    };
    drop(model_guard);

    if !should_index {
        tracing::debug!(
            doc_id = %doc_id,
            event_model = %event_model_id,
            "Ignoring embeddings for different model"
        );
        return Ok(());
    }

    tracing::debug!(doc_id = %doc_id, model_id = %event_model_id, "Processing embedding event");

    // Fetch embedding data
    let storage_guard = ctx.storage.read().await;
    let embedding_bytes = storage_guard
        .get_blob(&content_hash)
        .await?
        .ok_or_else(|| {
            anyhow::anyhow!("Embedding blob not found for {}/{}", doc_id, event_model_id)
        })?;
    let embedding_data: EmbeddingData = serde_json::from_slice(&embedding_bytes)?;

    // Fetch document metadata for page_count and name
    let metadata = storage_guard
        .get_document(namespace_id, doc_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Document not found: {}", doc_id))?;
    drop(storage_guard);

    // Delete old chunks before indexing (in case of re-embedding)
    {
        let index_guard = ctx.index.read().await;
        let config_guard = ctx.indexer_config.lock().await;
        let deleted = search::delete_document_chunks(&index_guard, &config_guard, doc_id)?;
        if deleted > 0 {
            tracing::debug!(doc_id = %doc_id, deleted = deleted, "Deleted old chunks");
        }
    }

    // Convert EmbeddingData to chunks and index
    let chunks_to_index: Vec<ChunkToIndex> = embedding_data
        .chunks
        .into_iter()
        .map(|chunk| {
            let enriched_content = format!("[{}]\n\n{}", metadata.name, chunk.content);
            ChunkToIndex {
                id: format!("{}_chunk_{}", doc_id, chunk.index),
                parent_id: doc_id.to_string(),
                parent_name: metadata.name.clone(),
                chunk_index: chunk.index,
                content: enriched_content,
                collection_id: collection_id.to_string(),
                page_count: metadata.page_count,
                start_page: chunk.start_page,
                end_page: chunk.end_page,
                vector: Some(chunk.vector),
            }
        })
        .collect();

    let chunk_count = chunks_to_index.len();

    if !chunks_to_index.is_empty() {
        let index_guard = ctx.index.read().await;
        let config_guard = ctx.indexer_config.lock().await;
        search::index_chunks_batch(&index_guard, &config_guard, chunks_to_index)?;
    }

    tracing::info!(
        doc_id = %doc_id,
        name = %metadata.name,
        chunk_count = chunk_count,
        "Indexed document with embeddings"
    );

    // Notify completion
    let _ = ctx
        .completed_tx
        .send(DocumentCompleted {
            doc_id: doc_id.to_string(),
            collection_id: collection_id.to_string(),
        })
        .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_key_prefix_matching() {
        assert!("files/doc-123".starts_with("files/"));
        assert!("embeddings/doc-123/qwen3".starts_with("embeddings/"));
        assert!(!"_collection".starts_with("files/"));
    }

    #[test]
    fn test_embeddings_key_parsing() {
        let key = "embeddings/abc123/qwen3-embedding";
        let parts: Vec<&str> = key.split('/').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], "abc123");
        assert_eq!(parts[2], "qwen3-embedding");
    }
}
