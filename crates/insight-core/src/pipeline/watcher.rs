//! Collection watcher for remote pipeline changes.
//!
//! Local imports hand work to the workers explicitly. The watcher only reacts
//! to remote `files/` entries, queuing index reconciliation for their document.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::Context;
use futures::{Stream, StreamExt};
use iroh_blobs::Hash;
use iroh_docs::{ContentStatus, NamespaceId};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::storage::{LastSyncSummary, LiveEvent, SharingTracker, Storage};

use super::index_queue::IndexQueue;
use super::AppEvent;

/// Dispatch channels for watcher-originated work.
pub struct JobSenders {
    pub index: IndexQueue,
    pub events: mpsc::UnboundedSender<AppEvent>,
    pub sharing: SharingTracker,
}

/// Extract doc_id from a files/{doc_id}/... key
fn extract_doc_id(key: &str) -> Option<&str> {
    let rest = key.strip_prefix("files/")?;
    rest.split('/').next()
}

/// Watches a collection for iroh events and dispatches to worker pools.
pub struct CollectionWatcher {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl CollectionWatcher {
    /// Spawn a watcher for a collection.
    ///
    /// Subscribes to iroh-docs events before returning, so a caller that
    /// starts sync afterwards cannot miss the first neighbor or sync events.
    /// Local inserts are ignored; remote `files/` entries queue index
    /// reconciliation for their document.
    pub async fn spawn(
        namespace_id: NamespaceId,
        storage: Arc<RwLock<Storage>>,
        senders: JobSenders,
        cancel: CancellationToken,
    ) -> anyhow::Result<Self> {
        let (doc, stream) = {
            let storage_guard = storage.read().await;
            storage_guard.subscribe(namespace_id).await?
        };
        let cancel_clone = cancel.clone();

        let task = tokio::spawn(async move {
            let result = run_watcher(namespace_id, stream, senders, cancel_clone.clone()).await;
            if let Err(ref e) = result {
                if !cancel_clone.is_cancelled() {
                    tracing::error!(
                        namespace = %namespace_id,
                        error = %e,
                        "CollectionWatcher error"
                    );
                }
            }
            // The watcher holds the replica open for as long as it consumes
            // the document's events.
            if let Err(e) = doc.close().await {
                tracing::warn!(
                    namespace = %namespace_id,
                    error = %e,
                    "Failed to close watched collection replica"
                );
            }
        });

        Ok(Self { cancel, task })
    }

    /// Stop the watcher and wait for its task to finish.
    ///
    /// Returns once the task has closed the document replica it holds open.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        self.cancel.cancel();
        self.task.await.context("CollectionWatcher task failed")?;
        Ok(())
    }
}

async fn run_watcher(
    namespace_id: NamespaceId,
    stream: impl Stream<Item = anyhow::Result<LiveEvent>> + Send + Unpin + 'static,
    senders: JobSenders,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    tokio::pin!(stream);

    let mut pending_content: HashMap<Hash, HashSet<String>> = HashMap::new();
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
                            &senders,
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
    senders: &JobSenders,
    pending_content: &mut HashMap<Hash, HashSet<String>>,
) {
    match event {
        LiveEvent::NeighborUp(peer) => {
            senders.sharing.peer_connected(namespace_id, *peer).await;
            sharing_changed(namespace_id, senders);
            return;
        }
        LiveEvent::NeighborDown(peer) => {
            senders.sharing.peer_disconnected(namespace_id, *peer).await;
            sharing_changed(namespace_id, senders);
            return;
        }
        LiveEvent::SyncFinished(event) => {
            let summary = match &event.result {
                Ok(details) => LastSyncSummary {
                    peer: event.peer.fmt_short().to_string(),
                    finished_at: chrono::DateTime::<chrono::Utc>::from(event.finished),
                    entries_received: details.entries_received,
                    entries_sent: details.entries_sent,
                    error: None,
                },
                Err(message) => LastSyncSummary {
                    peer: event.peer.fmt_short().to_string(),
                    finished_at: chrono::DateTime::<chrono::Utc>::from(event.finished),
                    entries_received: 0,
                    entries_sent: 0,
                    error: Some(message.clone()),
                },
            };
            senders.sharing.sync_finished(namespace_id, summary).await;
            sharing_changed(namespace_id, senders);
            invalidate(namespace_id, senders);
            return;
        }
        _ => {}
    }

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
            if let Some(doc_ids) = pending_content.remove(hash) {
                for doc_id in doc_ids {
                    queue_index(namespace_id, &doc_id, senders).await;
                }
            }
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
        pending_content
            .entry(content_hash)
            .or_default()
            .insert(doc_id);
    } else {
        queue_index(namespace_id, &doc_id, senders).await;
    }
}

async fn queue_index(namespace_id: NamespaceId, doc_id: &str, senders: &JobSenders) {
    tracing::debug!(doc_id = %doc_id, "Queuing index");
    senders
        .index
        .enqueue(namespace_id, doc_id.to_string())
        .await;
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

/// Tell the application layer that a collection's live sharing state changed.
fn sharing_changed(namespace_id: NamespaceId, senders: &JobSenders) {
    let _ = senders.events.send(AppEvent::SharingStatusChanged {
        namespace: Some(namespace_id),
    });
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use iroh::SecretKey;
    use iroh_docs::{AuthorId, Entry, Record, RecordIdentifier};
    use tokio::sync::mpsc;

    use super::*;
    use crate::pipeline::{IndexJob, ProgressTracker};

    /// Receive the next item on a channel, or `None` if nothing arrives within `budget`.
    async fn recv_within<T>(rx: &mut mpsc::UnboundedReceiver<T>, budget: Duration) -> Option<T> {
        tokio::time::timeout(budget, rx.recv()).await.ok().flatten()
    }

    /// Receive the next index job, or `None` if none arrives within `budget`.
    async fn recv_job(queue: &IndexQueue, budget: Duration) -> Option<IndexJob> {
        tokio::time::timeout(budget, queue.recv())
            .await
            .ok()
            .flatten()
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
                    panic!("unexpected collection invalidation");
                }
                Some(_) => continue,
                None => return,
            }
        }
    }

    /// Build the watcher's dispatch inputs, returning the queue so tests can
    /// observe the jobs the watcher actually sent.
    fn dispatch() -> (
        JobSenders,
        IndexQueue,
        ProgressTracker,
        mpsc::UnboundedReceiver<AppEvent>,
    ) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        let progress = ProgressTracker::new(events_tx.clone());
        let queue = IndexQueue::new(progress.clone());
        let senders = JobSenders {
            index: queue.clone(),
            events: events_tx,
            sharing: SharingTracker::new(),
        };
        (senders, queue, progress, events_rx)
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

    fn insert_remote_with_hash(namespace: NamespaceId, key: &str, hash: Hash) -> LiveEvent {
        LiveEvent::InsertRemote {
            from: SecretKey::generate().public(),
            entry: Entry::new(
                RecordIdentifier::new(namespace, AuthorId::default(), key.as_bytes()),
                Record::new(hash, key.len() as u64, 0),
            ),
            content_status: ContentStatus::Missing,
        }
    }

    /// Local entries are handed off explicitly by the import and extract
    /// stages; the watcher must not queue work or invalidate for them.
    #[tokio::test]
    async fn ignores_local_pipeline_entries() {
        let namespace = NamespaceId::default();
        let (senders, queue, progress, mut events_rx) = dispatch();
        let mut pending = HashMap::new();

        for key in [
            "files/doc1/source",
            "files/doc1/meta",
            "files/doc1/extractions/test",
            "_collection",
        ] {
            handle_event(
                &insert_local(namespace, key),
                namespace,
                &senders,
                &mut pending,
            )
            .await;
        }

        assert!(recv_job(&queue, Duration::from_millis(200)).await.is_none());
        assert_quiet(&mut events_rx, namespace, Duration::from_millis(200)).await;
        assert!(pending.is_empty());
        assert!(progress.get(&namespace.to_string()).await.is_none());
    }

    /// Remote `files/` entries queue index reconciliation for their document;
    /// keys outside a document prefix route nowhere.
    #[tokio::test]
    async fn queues_remote_files_entries_for_indexing() {
        let namespace = NamespaceId::default();
        let (senders, queue, _progress, mut events_rx) = dispatch();
        let mut pending = HashMap::new();

        for key in [
            "files/doc1/meta",
            "files/doc1/source",
            "files/doc1/extractions/test",
        ] {
            handle_event(
                &insert_remote(namespace, key, ContentStatus::Complete),
                namespace,
                &senders,
                &mut pending,
            )
            .await;
            let job = recv_job(&queue, Duration::from_secs(1))
                .await
                .expect("remote files entry was not queued for indexing");
            assert_eq!(job.doc_id, "doc1");
            assert_eq!(job.namespace_id, namespace);
        }

        handle_event(
            &insert_remote(namespace, "_collection", ContentStatus::Complete),
            namespace,
            &senders,
            &mut pending,
        )
        .await;
        assert!(recv_job(&queue, Duration::from_millis(200)).await.is_none());
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
        let (senders, _queue, _progress, mut events_rx) = dispatch();
        let mut pending = HashMap::new();

        let sync = LiveEvent::SyncFinished(SyncEvent {
            peer: SecretKey::generate().public(),
            origin: Origin::Accept,
            started: SystemTime::now(),
            finished: SystemTime::now(),
            result: Err("test sync".to_string()),
        });
        handle_event(&sync, namespace, &senders, &mut pending).await;
        assert!(
            await_invalidation(&mut events_rx, namespace, Duration::from_secs(1)).await,
            "peer sync did not invalidate the collection"
        );

        handle_event(
            &LiveEvent::PendingContentReady,
            namespace,
            &senders,
            &mut pending,
        )
        .await;
        handle_event(
            &LiveEvent::ContentReady {
                hash: Hash::new(b"landed content"),
            },
            namespace,
            &senders,
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
                RecordIdentifier::new(namespace, AuthorId::default(), b"files/doc1/extractions/p"),
                Record::new(content_hash, 17, 0),
            ),
            content_status: ContentStatus::Missing,
        };

        let (senders, queue, _progress, _events_rx) = dispatch();
        let mut pending = HashMap::new();

        handle_event(&event, namespace, &senders, &mut pending).await;

        assert_eq!(
            pending.get(&content_hash).map(|docs| docs.contains("doc1")),
            Some(true)
        );
        assert!(recv_job(&queue, Duration::from_millis(200)).await.is_none());

        handle_event(
            &LiveEvent::ContentReady { hash: content_hash },
            namespace,
            &senders,
            &mut pending,
        )
        .await;

        let job = recv_job(&queue, Duration::from_secs(1))
            .await
            .expect("content-ready entry was not queued for indexing");
        assert_eq!(job.doc_id, "doc1");
        assert_eq!(job.namespace_id, namespace);
        assert!(pending.is_empty());
    }

    /// Several documents can share one missing content hash; the ContentReady
    /// event releases every document waiting on it, not just the last one.
    #[tokio::test]
    async fn content_ready_releases_every_document_waiting_on_the_hash() {
        let namespace = NamespaceId::default();
        let content_hash = Hash::new(b"shared remote content");
        let (senders, queue, _progress, _events_rx) = dispatch();
        let mut pending = HashMap::new();

        for key in ["files/doc1/source", "files/doc2/source"] {
            handle_event(
                &insert_remote_with_hash(namespace, key, content_hash),
                namespace,
                &senders,
                &mut pending,
            )
            .await;
        }
        assert!(recv_job(&queue, Duration::from_millis(200)).await.is_none());

        handle_event(
            &LiveEvent::ContentReady { hash: content_hash },
            namespace,
            &senders,
            &mut pending,
        )
        .await;

        let mut doc_ids = Vec::new();
        for _ in 0..2 {
            doc_ids.push(
                recv_job(&queue, Duration::from_secs(1))
                    .await
                    .expect("document waiting on the content hash was not queued")
                    .doc_id,
            );
        }
        doc_ids.sort();
        assert_eq!(doc_ids, ["doc1", "doc2"]);
        assert!(recv_job(&queue, Duration::from_millis(200)).await.is_none());
        assert!(pending.is_empty());
    }
}
