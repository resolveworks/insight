//! Sync watcher for handling documents arriving from peers.
//!
//! Watches a namespace for InsertRemote events and processes them.
//! Local imports are handled directly (not through events).

use std::sync::Arc;

use futures::StreamExt;
use iroh_docs::NamespaceId;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;
use crate::storage::{LiveEvent, Storage};

use super::index::IndexWorkerHandle;
use super::process_document;

/// Watches a namespace for documents arriving from peers.
///
/// Only handles `InsertRemote` events - local imports are processed directly
/// without going through the event system.
pub struct SyncWatcher {
    cancel: CancellationToken,
}

impl SyncWatcher {
    /// Spawn a sync watcher for a namespace.
    ///
    /// The watcher subscribes to namespace events and processes documents
    /// that arrive from peers (InsertRemote events).
    pub fn spawn(
        namespace_id: NamespaceId,
        storage: Arc<RwLock<Storage>>,
        embedder: Arc<RwLock<Option<Embedder>>>,
        model_id: Arc<RwLock<Option<String>>>,
        index_worker: IndexWorkerHandle,
        cancel: CancellationToken,
    ) -> Self {
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            if let Err(e) = run_watcher(
                namespace_id,
                storage,
                embedder,
                model_id,
                index_worker,
                cancel_clone.clone(),
            )
            .await
            {
                if !cancel_clone.is_cancelled() {
                    tracing::error!(
                        namespace = %namespace_id,
                        error = %e,
                        "SyncWatcher error"
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
    storage: Arc<RwLock<Storage>>,
    embedder: Arc<RwLock<Option<Embedder>>>,
    model_id: Arc<RwLock<Option<String>>>,
    index_worker: IndexWorkerHandle,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    // Subscribe to namespace events
    let stream = {
        let storage_guard = storage.read().await;
        storage_guard.subscribe(namespace_id).await?
    };

    tokio::pin!(stream);

    tracing::info!(namespace = %namespace_id, "SyncWatcher started");

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                tracing::debug!(namespace = %namespace_id, "SyncWatcher cancelled");
                break;
            }

            event = stream.next() => {
                match event {
                    Some(Ok(live_event)) => {
                        if let Err(e) = handle_event(
                            &live_event,
                            namespace_id,
                            &storage,
                            &embedder,
                            &model_id,
                            &index_worker,
                        ).await {
                            tracing::warn!(
                                namespace = %namespace_id,
                                error = %e,
                                "Failed to handle sync event"
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

    tracing::info!(namespace = %namespace_id, "SyncWatcher stopped");
    Ok(())
}

async fn handle_event(
    event: &LiveEvent,
    namespace_id: NamespaceId,
    storage: &Arc<RwLock<Storage>>,
    embedder: &Arc<RwLock<Option<Embedder>>>,
    model_id: &Arc<RwLock<Option<String>>>,
    index_worker: &IndexWorkerHandle,
) -> anyhow::Result<()> {
    // Only handle InsertRemote - local imports are processed directly
    let entry = match event {
        LiveEvent::InsertRemote { entry, .. } => entry,
        _ => return Ok(()),
    };

    let key = String::from_utf8_lossy(entry.key());

    // Only process files/* events (document metadata)
    // Ignore embeddings/*, _collection, _hash_index, etc.
    if !key.starts_with("files/") {
        return Ok(());
    }

    let doc_id = key.strip_prefix("files/").unwrap_or(&key);
    tracing::info!(doc_id = %doc_id, "Processing document from peer");

    // Get embedder and model ID
    let embedder_guard = embedder.read().await;
    let model_id_guard = model_id.read().await;

    let (emb, mid) = match (&*embedder_guard, &*model_id_guard) {
        (Some(e), Some(m)) => (e, m.clone()),
        _ => {
            tracing::warn!(doc_id = %doc_id, "No embedder configured, skipping sync document");
            return Ok(());
        }
    };

    // Fetch document metadata
    let storage_guard = storage.read().await;
    let metadata_bytes = storage_guard
        .get_blob(&entry.content_hash())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Metadata blob not found for {}", doc_id))?;
    let metadata: crate::storage::DocumentMetadata = serde_json::from_slice(&metadata_bytes)?;

    // Process document (shared function)
    process_document(
        &storage_guard,
        emb,
        &mid,
        namespace_id,
        index_worker,
        &metadata,
    )
    .await?;

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        "Synced document from peer"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_key_prefix_matching() {
        assert!("files/doc-123".starts_with("files/"));
        assert!(!"embeddings/doc-123/qwen3".starts_with("files/"));
        assert!(!"_collection".starts_with("files/"));
    }
}
