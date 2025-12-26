//! Document watcher that subscribes to iroh events and triggers indexing.
//!
//! Watches for InsertLocal/InsertRemote events on document entries (files/*)
//! and triggers the embed → index pipeline for each new document.

use std::sync::Arc;

use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::NamespaceId;
use serde::Serialize;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::storage::{DocumentMetadata, LiveEvent, Storage};

use super::types::Stored;

/// Event emitted when a document is successfully indexed
#[derive(Debug, Clone, Serialize)]
pub struct DocumentIndexed {
    pub doc_id: String,
    pub name: String,
    pub collection_id: String,
}

/// Event emitted when document indexing fails
#[derive(Debug, Clone, Serialize)]
pub struct DocumentIndexFailed {
    pub doc_id: String,
    pub collection_id: String,
    pub error: String,
}

/// Watches a namespace for document events and triggers indexing.
pub struct DocWatcher {
    cancel: CancellationToken,
}

impl DocWatcher {
    /// Spawn a watcher for a specific namespace.
    ///
    /// Subscribes to the namespace's event stream and forwards new documents
    /// to the embedding pipeline.
    pub fn spawn(
        namespace_id: NamespaceId,
        storage: Arc<RwLock<Storage>>,
        embed_tx: mpsc::Sender<Stored>,
        cancel: CancellationToken,
    ) -> Self {
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            if let Err(e) = run_watcher(namespace_id, storage, embed_tx, cancel_clone.clone()).await
            {
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
    storage: Arc<RwLock<Storage>>,
    embed_tx: mpsc::Sender<Stored>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let collection_id = namespace_id.to_string();

    // Subscribe to namespace events
    let stream = {
        let storage = storage.read().await;
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
                            &storage,
                            &embed_tx,
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
    storage: &Arc<RwLock<Storage>>,
    embed_tx: &mpsc::Sender<Stored>,
) -> anyhow::Result<()> {
    // We care about InsertLocal and InsertRemote for files/* keys
    let entry = match event {
        LiveEvent::InsertLocal { entry } => entry,
        LiveEvent::InsertRemote { entry, .. } => entry,
        // ContentReady could be used for on-demand fetching, but we process immediately
        _ => return Ok(()),
    };

    let key = entry.key();
    let key_str = String::from_utf8_lossy(key);

    // Only process document entries (files/*)
    if !key_str.starts_with("files/") {
        return Ok(());
    }

    let doc_id = key_str.strip_prefix("files/").unwrap_or(&key_str);
    tracing::debug!(doc_id = %doc_id, collection_id = %collection_id, "Processing document event");

    // Fetch the document metadata from the entry's content
    let content_hash = entry.content_hash();
    let storage_guard = storage.read().await;

    let metadata_bytes = storage_guard
        .get_blob(&content_hash)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Metadata blob not found for {}", doc_id))?;

    let metadata: DocumentMetadata = serde_json::from_slice(&metadata_bytes)?;

    // Fetch the text content using text_hash
    let text_hash: Hash = metadata
        .text_hash
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid text hash for {}", doc_id))?;

    let text_bytes = storage_guard
        .get_blob(&text_hash)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Text blob not found for {}", doc_id))?;

    let text = String::from_utf8(text_bytes)?;

    drop(storage_guard);

    // Create Stored and send to embedding pipeline
    let stored = Stored {
        doc_id: metadata.id,
        name: metadata.name,
        text,
        collection_id: collection_id.to_string(),
        page_count: metadata.page_count,
        page_boundaries: metadata.page_boundaries,
    };

    embed_tx
        .send(stored)
        .await
        .map_err(|_| anyhow::anyhow!("Embed channel closed"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_key_prefix_matching() {
        assert!("files/doc-123".starts_with("files/"));
        assert!(!"_collection".starts_with("files/"));
    }
}
