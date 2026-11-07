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
use super::AppEvent;

/// Grouped job dispatch channels for pipeline stages.
pub struct JobSenders {
    pub extract: mpsc::UnboundedSender<ExtractJob>,
    pub index: mpsc::UnboundedSender<IndexJob>,
    pub events: mpsc::UnboundedSender<AppEvent>,
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
            if let Err(e) = run_watcher(
                namespace_id,
                storage,
                senders,
                progress,
                cancel_clone.clone(),
            )
            .await
            {
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
        LiveEvent::InsertLocal { entry } => (entry.key(), true),
        LiveEvent::InsertRemote { entry, .. } => (entry.key(), false),
        // Content landing or a peer sync can change what a collection lists
        // (a metadata blob becomes readable, a peer deletes entries), without
        // any insert event reaching us.
        LiveEvent::ContentReady { .. }
        | LiveEvent::PendingContentReady
        | LiveEvent::SyncFinished(_) => {
            invalidate(namespace_id, senders);
            return;
        }
        _ => return,
    };

    let key = String::from_utf8_lossy(key_bytes);
    if !key.starts_with("files/") {
        return;
    }

    // Any files/ entry can change the document listing.
    invalidate(namespace_id, senders);

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

/// Tell the application layer that this collection's entries changed.
fn invalidate(namespace_id: NamespaceId, senders: &JobSenders) {
    let _ = senders.events.send(AppEvent::CollectionInvalidated {
        namespace: namespace_id,
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use tokio::sync::mpsc;

    use super::*;
    use crate::storage::CollectionMetadata;

    /// Receive the next item, or `None` if nothing arrives within `budget`.
    async fn recv_within<T>(rx: &mut mpsc::UnboundedReceiver<T>, budget: Duration) -> Option<T> {
        tokio::time::timeout(budget, rx.recv()).await.ok().flatten()
    }

    /// Wait until the watcher reports this collection as changed.
    async fn await_invalidation(
        rx: &mut mpsc::UnboundedReceiver<AppEvent>,
        namespace: NamespaceId,
        budget: Duration,
    ) -> bool {
        let deadline = Instant::now() + budget;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match recv_within(rx, remaining).await {
                Some(AppEvent::CollectionInvalidated { namespace: ns }) if ns == namespace => {
                    return true;
                }
                Some(_) => continue,
                None => return false,
            }
        }
    }

    /// Assert the watcher reports no change for this collection within `window`.
    async fn assert_quiet(
        rx: &mut mpsc::UnboundedReceiver<AppEvent>,
        namespace: NamespaceId,
        window: Duration,
    ) {
        let deadline = Instant::now() + window;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return;
            }
            match recv_within(rx, remaining).await {
                Some(AppEvent::CollectionInvalidated { namespace: ns }) if ns == namespace => {
                    panic!("non-files/ entry invalidated the collection");
                }
                Some(_) => continue,
                None => return,
            }
        }
    }

    /// Write one entry through the docs API, the way collection metadata is written.
    async fn set_entry(storage: &RwLock<Storage>, namespace: NamespaceId, key: &str, value: &[u8]) {
        let storage = storage.read().await;
        let doc = storage
            .docs()
            .open(namespace)
            .await
            .unwrap()
            .expect("collection missing");
        doc.set_bytes(storage.author_id(), key.to_string(), value.to_vec())
            .await
            .unwrap();
        doc.close().await.unwrap();
    }

    /// A stored source routes to an extract job, stored text routes to an index
    /// job, and keys outside `files/` route nowhere.
    #[tokio::test]
    async fn routes_stored_entries_to_pipeline_jobs() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(RwLock::new(Storage::open(temp_dir.path()).await.unwrap()));
        let (namespace, _) = storage
            .read()
            .await
            .create_collection("Watcher Test")
            .await
            .unwrap();

        let (extract_tx, mut extract_rx) = mpsc::unbounded_channel();
        let (index_tx, mut index_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let progress = ProgressTracker::new(events_tx.clone());
        let watcher = CollectionWatcher::spawn(
            namespace,
            storage.clone(),
            JobSenders {
                extract: extract_tx,
                index: index_tx,
                events: events_tx,
            },
            progress,
            CancellationToken::new(),
        );

        // The watcher subscribes from its own task, so probe with a files/ entry
        // until its invalidation arrives; distinct values guarantee a fresh
        // InsertLocal event per attempt.
        let live = tokio::time::timeout(Duration::from_secs(30), async {
            for attempt in 0.. {
                set_entry(
                    &storage,
                    namespace,
                    "files/probe/meta",
                    format!("probe {attempt}").as_bytes(),
                )
                .await;
                if await_invalidation(&mut events_rx, namespace, Duration::from_millis(500)).await {
                    return;
                }
            }
        })
        .await;
        assert!(live.is_ok(), "watcher never subscribed to the collection");

        // A non-files/ key must not dispatch a job or invalidate the collection.
        let metadata = CollectionMetadata {
            name: "Watcher Test".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        };
        set_entry(
            &storage,
            namespace,
            "_collection",
            &serde_json::to_vec(&metadata).unwrap(),
        )
        .await;
        assert!(recv_within(&mut extract_rx, Duration::from_millis(500))
            .await
            .is_none());
        assert!(recv_within(&mut index_rx, Duration::from_millis(500))
            .await
            .is_none());
        assert_quiet(&mut events_rx, namespace, Duration::from_millis(500)).await;

        // Storing a source file must queue extraction for its document.
        let pdf_path = temp_dir.path().join("paper.pdf");
        tokio::fs::write(&pdf_path, b"%PDF-1.4 watcher test")
            .await
            .unwrap();
        let doc_id = storage
            .read()
            .await
            .store_pdf_source(&pdf_path, namespace)
            .await
            .unwrap();
        let job = recv_within(&mut extract_rx, Duration::from_secs(10))
            .await
            .expect("stored source was not queued for extraction");
        assert_eq!(job.doc_id, doc_id);
        assert_eq!(job.namespace_id, namespace);
        assert!(
            await_invalidation(&mut events_rx, namespace, Duration::from_secs(10)).await,
            "stored source did not invalidate the collection"
        );

        // Writing extracted text must queue indexing for the same document.
        let text = "page one text";
        storage
            .read()
            .await
            .write_text_and_meta(namespace, &doc_id, text, &[text.len()])
            .await
            .unwrap();
        let job = recv_within(&mut index_rx, Duration::from_secs(10))
            .await
            .expect("stored text was not queued for indexing");
        assert_eq!(job.doc_id, doc_id);
        assert_eq!(job.namespace_id, namespace);

        watcher.stop();
    }
}
