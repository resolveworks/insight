//! Collection watcher for remote pipeline changes.
//!
//! Local imports hand work to the workers explicitly. The watcher only reacts
//! to remote `files/` entries, queuing index reconciliation for their document.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use iroh_blobs::Hash;
use iroh_docs::{ContentStatus, NamespaceId};
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::storage::{LiveEvent, Storage};

use super::progress::ProgressTracker;
use super::types::{IndexJob, Stage};
use super::AppEvent;

/// Dispatch channels for watcher-originated work.
pub struct JobSenders {
    pub index: mpsc::UnboundedSender<IndexJob>,
    pub events: mpsc::UnboundedSender<AppEvent>,
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
    /// The watcher subscribes to iroh-docs events. Local inserts are ignored;
    /// remote `files/` entries queue index reconciliation for their document.
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
    let mut pending_content: HashMap<Hash, String> = HashMap::new();
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
                        handle_event(
                            &live_event,
                            namespace_id,
                            &collection_id,
                            &senders,
                            &progress,
                            &mut pending_content,
                        )
                        .await;
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
    pending_content: &mut HashMap<Hash, String>,
) {
    let (key_bytes, content_status, content_hash) = match event {
        // A local import hands its work off explicitly; local entries never
        // drive the pipeline through the watcher.
        LiveEvent::InsertLocal { .. } => return,
        LiveEvent::InsertRemote {
            entry,
            content_status,
            ..
        } => (entry.key(), *content_status, entry.content_hash()),
        LiveEvent::ContentReady { hash } => {
            if let Some(doc_id) = pending_content.remove(hash) {
                queue_index(namespace_id, collection_id, &doc_id, senders, progress).await;
            }
            return;
        }
        // A peer sync can add, change or delete entries with no local command.
        LiveEvent::SyncFinished(_) => {
            invalidate(namespace_id, senders);
            return;
        }
        _ => return,
    };

    let key = String::from_utf8_lossy(key_bytes);
    let Some(doc_id) = extract_doc_id(&key) else {
        return;
    };
    let doc_id = doc_id.to_string();

    if content_status != ContentStatus::Complete {
        // The entry arrived before its content blob. Wait for the matching
        // ContentReady before queueing the index job.
        tracing::debug!(
            doc_id = %doc_id,
            "Remote entry pending content; waiting for ContentReady"
        );
        pending_content.insert(content_hash, doc_id);
    } else {
        queue_index(namespace_id, collection_id, &doc_id, senders, progress).await;
    }
}

async fn queue_index(
    namespace_id: NamespaceId,
    collection_id: &str,
    doc_id: &str,
    senders: &JobSenders,
    progress: &ProgressTracker,
) {
    tracing::debug!(doc_id = %doc_id, "Queuing index");
    progress.queue(collection_id, Stage::Index).await;
    let _ = senders.index.send(IndexJob {
        namespace_id,
        doc_id: doc_id.to_string(),
    });
}

/// Tell the application layer that a peer changed this collection's entries.
///
/// The signal exists because a peer's changes can alter a collection's listing
/// with no local command; it is not a general change feed.
fn invalidate(namespace_id: NamespaceId, senders: &JobSenders) {
    let _ = senders.events.send(AppEvent::CollectionInvalidated {
        namespace: namespace_id,
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iroh::SecretKey;
    use iroh_docs::{AuthorId, Entry, Record, RecordIdentifier};
    use tokio::sync::mpsc;

    use super::*;

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
                    panic!("a local change invalidated the collection");
                }
                Some(_) => continue,
                None => return,
            }
        }
    }

    fn entry(namespace: NamespaceId, key: &str) -> Entry {
        Entry::new(
            RecordIdentifier::new(namespace, AuthorId::default(), key.as_bytes()),
            Record::new(Hash::new(key.as_bytes()), key.len() as u64, 0),
        )
    }

    fn insert_local(namespace: NamespaceId, key: &str) -> LiveEvent {
        LiveEvent::InsertLocal {
            entry: entry(namespace, key),
        }
    }

    fn insert_remote(
        namespace: NamespaceId,
        key: &str,
        content_status: ContentStatus,
    ) -> LiveEvent {
        LiveEvent::InsertRemote {
            from: SecretKey::generate().public(),
            entry: entry(namespace, key),
            content_status,
        }
    }

    /// Local entries are handed off explicitly by the import and extract
    /// stages; the watcher must not queue work or invalidate for them.
    #[tokio::test]
    async fn ignores_local_pipeline_entries() {
        let namespace = NamespaceId::default();
        let (index_tx, mut index_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let senders = JobSenders {
            index: index_tx,
            events: events_tx.clone(),
        };
        let progress = ProgressTracker::new(events_tx);
        let mut pending = HashMap::new();

        for key in [
            "files/doc1/source",
            "files/doc1/meta",
            "files/doc1/extractions/test@1",
            "_collection",
        ] {
            handle_event(
                &insert_local(namespace, key),
                namespace,
                &namespace.to_string(),
                &senders,
                &progress,
                &mut pending,
            )
            .await;
        }

        assert!(recv_within(&mut index_rx, Duration::from_millis(200))
            .await
            .is_none());
        assert_quiet(&mut events_rx, namespace, Duration::from_millis(200)).await;
        assert!(pending.is_empty());
        assert!(progress.get(&namespace.to_string()).await.is_none());
    }

    /// Remote `files/` entries queue index reconciliation for their document;
    /// keys outside a document prefix route nowhere.
    #[tokio::test]
    async fn queues_remote_files_entries_for_indexing() {
        let namespace = NamespaceId::default();
        let (index_tx, mut index_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let senders = JobSenders {
            index: index_tx,
            events: events_tx.clone(),
        };
        let progress = ProgressTracker::new(events_tx);
        let mut pending = HashMap::new();

        for key in [
            "files/doc1/meta",
            "files/doc1/source",
            "files/doc1/extractions/test@1",
        ] {
            handle_event(
                &insert_remote(namespace, key, ContentStatus::Complete),
                namespace,
                &namespace.to_string(),
                &senders,
                &progress,
                &mut pending,
            )
            .await;
            let job = recv_within(&mut index_rx, Duration::from_secs(1))
                .await
                .expect("remote files entry was not queued for indexing");
            assert_eq!(job.doc_id, "doc1");
            assert_eq!(job.namespace_id, namespace);
        }

        handle_event(
            &insert_remote(namespace, "_collection", ContentStatus::Complete),
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;
        assert!(recv_within(&mut index_rx, Duration::from_millis(200))
            .await
            .is_none());
        assert_quiet(&mut events_rx, namespace, Duration::from_millis(200)).await;
        assert!(pending.is_empty());
    }

    /// A finished peer sync changes a collection's listing with no local
    /// command, so it invalidates the collection; content landing on its own
    /// does not.
    #[tokio::test]
    async fn reports_collection_changed_only_for_peer_sync() {
        use std::time::SystemTime;

        use iroh_docs::engine::{Origin, SyncEvent};

        let namespace = NamespaceId::default();
        let (index_tx, _index_rx) = mpsc::unbounded_channel();
        let (events_tx, mut events_rx) = mpsc::unbounded_channel();
        let senders = JobSenders {
            index: index_tx,
            events: events_tx.clone(),
        };
        let progress = ProgressTracker::new(events_tx);
        let mut pending = HashMap::new();

        let sync = LiveEvent::SyncFinished(SyncEvent {
            peer: SecretKey::generate().public(),
            origin: Origin::Accept,
            started: SystemTime::now(),
            finished: SystemTime::now(),
            result: Err("test sync".to_string()),
        });
        handle_event(
            &sync,
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;
        assert!(
            await_invalidation(&mut events_rx, namespace, Duration::from_secs(1)).await,
            "peer sync did not invalidate the collection"
        );

        handle_event(
            &LiveEvent::PendingContentReady,
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;
        handle_event(
            &LiveEvent::ContentReady {
                hash: Hash::new(b"landed content"),
            },
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;
        assert_quiet(&mut events_rx, namespace, Duration::from_millis(300)).await;
    }

    /// A remote entry whose content is not local yet waits for the matching
    /// ContentReady event before queueing an index job.
    #[tokio::test]
    async fn waits_for_content_ready_before_indexing_remote_entry() {
        let namespace = NamespaceId::default();
        let content_hash = Hash::new(b"remote record");
        let event = LiveEvent::InsertRemote {
            from: SecretKey::generate().public(),
            entry: Entry::new(
                RecordIdentifier::new(
                    namespace,
                    AuthorId::default(),
                    b"files/doc1/extractions/p@1",
                ),
                Record::new(content_hash, 17, 0),
            ),
            content_status: ContentStatus::Missing,
        };

        let (index_tx, mut index_rx) = mpsc::unbounded_channel();
        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let senders = JobSenders {
            index: index_tx,
            events: events_tx.clone(),
        };
        let progress = ProgressTracker::new(events_tx);
        let mut pending = HashMap::new();

        handle_event(
            &event,
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;

        assert_eq!(pending.get(&content_hash), Some(&"doc1".to_string()));
        assert!(recv_within(&mut index_rx, Duration::from_millis(200))
            .await
            .is_none());

        handle_event(
            &LiveEvent::ContentReady { hash: content_hash },
            namespace,
            &namespace.to_string(),
            &senders,
            &progress,
            &mut pending,
        )
        .await;

        let job = recv_within(&mut index_rx, Duration::from_secs(1))
            .await
            .expect("content-ready entry was not queued for indexing");
        assert_eq!(job.doc_id, "doc1");
        assert_eq!(job.namespace_id, namespace);
        assert!(pending.is_empty());
    }
}
