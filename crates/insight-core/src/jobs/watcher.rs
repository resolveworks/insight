//! Document watcher that subscribes to iroh events and triggers indexing.
//!
//! Watches for two event types:
//! - `files/*` events: Generate embeddings and store them (triggers embeddings/*)
//! - `embeddings/*` events: Index document with embeddings, notify completion

use std::sync::Arc;

use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;
use crate::search::ChunkToIndex;
use crate::storage::{EmbeddingData, LiveEvent, Storage};

use super::embed::generate_embeddings;
use super::index::IndexWorkerHandle;
use super::types::{DocumentCompleted, DocumentFailed};

/// Shared context for document watchers.
#[derive(Clone)]
pub struct WatcherContext {
    pub storage: Arc<RwLock<Storage>>,
    pub embedder: Arc<RwLock<Option<Embedder>>>,
    pub index_worker: IndexWorkerHandle,
    pub completed_tx: mpsc::Sender<DocumentCompleted>,
    pub failed_tx: mpsc::Sender<DocumentFailed>,
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
    /// - `files/*`: Generate embeddings, store them (triggers embeddings/*)
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
/// Generate embeddings directly - this triggers embeddings/* event.
async fn handle_document_event(
    key_str: &str,
    content_hash: Hash,
    namespace_id: NamespaceId,
    ctx: &WatcherContext,
) -> anyhow::Result<()> {
    let doc_id = key_str.strip_prefix("files/").unwrap_or(key_str);
    tracing::debug!(doc_id = %doc_id, "Processing document event");

    // Get embedder and model ID - fail if not configured
    let embedder_guard = ctx.embedder.read().await;
    let model_id_guard = ctx.current_model_id.read().await;

    let (embedder, model_id) = match (&*embedder_guard, &*model_id_guard) {
        (Some(emb), Some(mid)) => (emb, mid.clone()),
        _ => {
            let error = "No embedder configured - cannot process document";
            tracing::error!(doc_id = %doc_id, error);
            // Send failure notification
            let _ = ctx
                .failed_tx
                .send(DocumentFailed {
                    path: doc_id.to_string(),
                    error: error.to_string(),
                })
                .await;
            return Err(anyhow::anyhow!(error));
        }
    };

    // Fetch document metadata
    let storage_guard = ctx.storage.read().await;
    let metadata_bytes = storage_guard
        .get_blob(&content_hash)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Metadata blob not found for {}", doc_id))?;
    let metadata: crate::storage::DocumentMetadata = serde_json::from_slice(&metadata_bytes)?;

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        "Generating embeddings for document"
    );

    // Generate and store embeddings (this triggers embeddings/* event)
    if let Err(e) =
        generate_embeddings(&storage_guard, embedder, &model_id, namespace_id, &metadata).await
    {
        tracing::error!(
            doc_id = %metadata.id,
            error = %e,
            "Failed to generate embeddings"
        );
        // Send failure notification
        let _ = ctx
            .failed_tx
            .send(DocumentFailed {
                path: metadata.name.clone(),
                error: e.to_string(),
            })
            .await;
        return Err(e);
    }

    // Note: completion is sent when embeddings/* event is handled
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

    // Fetch embedding data and metadata from storage
    let (embedding_data, metadata) = {
        let storage_guard = ctx.storage.read().await;
        let embedding_bytes = storage_guard
            .get_blob(&content_hash)
            .await?
            .ok_or_else(|| {
                anyhow::anyhow!("Embedding blob not found for {}/{}", doc_id, event_model_id)
            })?;
        let embedding_data: EmbeddingData = serde_json::from_slice(&embedding_bytes)?;

        let metadata = storage_guard
            .get_document(namespace_id, doc_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Document not found: {}", doc_id))?;

        (embedding_data, metadata)
    }; // Storage lock released here

    // Delete old chunks before indexing (in case of re-embedding)
    let deleted = ctx
        .index_worker
        .delete_document_chunks(doc_id.to_string())
        .await?;
    if deleted > 0 {
        tracing::debug!(doc_id = %doc_id, deleted = deleted, "Deleted old chunks");
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

    // Send to index worker (runs in dedicated thread, doesn't block async runtime)
    if !chunks_to_index.is_empty() {
        ctx.index_worker.index_chunks(chunks_to_index).await?;
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
