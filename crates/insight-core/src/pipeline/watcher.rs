//! Collection watcher for event-driven pipeline.
//!
//! Watches iroh-docs events and dispatches to worker pools based on key patterns.

use std::sync::Arc;

use futures::StreamExt;
use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::storage::{LiveEvent, Storage};

use super::progress::ProgressTracker;
use super::types::{ExtractJob, IndexJob, Stage};

/// Grouped job dispatch channels for pipeline stages.
pub struct JobSenders {
    pub extract: mpsc::UnboundedSender<ExtractJob>,
    pub index: mpsc::UnboundedSender<IndexJob>,
}

/// Check if key is a source entry: files/{doc_id}/source
fn is_source_key(key: &str) -> bool {
    key.starts_with("files/") && key.ends_with("/source")
}

/// Check if key is a text entry: files/{doc_id}/text
fn is_text_key(key: &str) -> bool {
    key.starts_with("files/") && key.ends_with("/text")
}

/// Extract doc_id from a files/{doc_id}/... key
fn extract_doc_id(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("files/")?;
    rest.split('/').next()
}

/// Watches a collection for iroh events and dispatches to worker pools.
pub struct CollectionWatcher {
    cancel: CancellationToken,
}

impl CollectionWatcher {
    /// Spawn a watcher for a collection.
    ///
    /// The watcher subscribes to iroh-docs events and dispatches jobs
    /// to the appropriate worker pools based on key patterns:
    /// - files/*/source (InsertLocal only) → Extract
    /// - files/*/text (local or remote) → Index
    pub fn spawn(
        namespace_id: NamespaceId,
        storage: Arc<RwLock<Storage>>,
        senders: JobSenders,
        progress: ProgressTracker,
        cancel: CancellationToken,
    ) -> Self {
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            if let Err(e) = run_watcher(namespace_id, storage, senders, progress, cancel_clone.clone()).await {
                if !cancel_clone.is_cancelled() {
                    tracing::error!(
                        namespace = %namespace_id,
                        error = %e,
                        "CollectionWatcher error"
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
    senders: JobSenders,
    progress: ProgressTracker,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    // Subscribe to namespace events (need storage just for subscription)
    let stream = {
        let storage_guard = storage.read().await;
        storage_guard.subscribe(namespace_id).await?
    };
    // Drop storage reference - we don't need it after subscribing
    drop(storage);

    tokio::pin!(stream);

    let collection_id = namespace_id.to_string();
    tracing::info!(namespace = %namespace_id, "CollectionWatcher started");

    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                tracing::debug!(namespace = %namespace_id, "CollectionWatcher cancelled");
                break;
            }

            event = stream.next() => {
                match event {
                    Some(Ok(live_event)) => {
                        handle_event(&live_event, namespace_id, &collection_id, &senders, &progress).await;
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

    tracing::info!(namespace = %namespace_id, "CollectionWatcher stopped");
    Ok(())
}

async fn handle_event(
    event: &LiveEvent,
    namespace_id: NamespaceId,
    collection_id: &str,
    senders: &JobSenders,
    progress: &ProgressTracker,
) {
    let (key_bytes, is_local) = match event {
        LiveEvent::InsertLocal { entry, .. } => (entry.key(), true),
        LiveEvent::InsertRemote { entry, .. } => (entry.key(), false),
        _ => return,
    };

    let key = String::from_utf8_lossy(key_bytes);
    let doc_id = match extract_doc_id(&key) {
        Some(id) => id.to_string(),
        None => return,
    };

    if is_source_key(&key) && is_local {
        // Local source stored → queue extract
        tracing::debug!(doc_id = %doc_id, "Source stored, queuing extract");
        progress.queue(collection_id, Stage::Extract).await;
        let _ = senders.extract.send(ExtractJob {
            namespace_id,
            doc_id,
        });
    } else if is_text_key(&key) {
        // Text ready (local extract or remote sync) → queue index
        tracing::debug!(doc_id = %doc_id, is_local, "Text ready, queuing index");
        progress.queue(collection_id, Stage::Index).await;
        let _ = senders.index.send(IndexJob {
            namespace_id,
            doc_id,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_patterns() {
        assert!(is_source_key("files/doc-123/source"));
        assert!(!is_source_key("files/doc-123/text"));
        assert!(!is_source_key("files/doc-123/meta"));

        assert!(is_text_key("files/doc-123/text"));
        assert!(!is_text_key("files/doc-123/source"));
        assert!(!is_text_key("files/doc-123/meta"));
    }

    #[test]
    fn test_extract_doc_id() {
        assert_eq!(extract_doc_id("files/doc-123/source"), Some("doc-123"));
        assert_eq!(extract_doc_id("files/abc/text"), Some("abc"));
        assert_eq!(extract_doc_id("_collection"), None);
    }
}
