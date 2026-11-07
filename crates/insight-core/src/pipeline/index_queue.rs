//! App-owned dispatch queue for index jobs.
//!
//! Every index producer — the explicit local extraction handoff and the
//! remote watcher — enqueues through the same queue. Jobs are coalesced by
//! `(namespace, document)`: while one job for a document waits in the
//! channel, further enqueues are no-ops. The marker is released when the
//! worker receives the job, so a change arriving during the run queues one
//! follow-up. A document therefore has at most one queued and one running job.

use std::collections::HashSet;
use std::sync::Arc;

use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, Mutex};

use super::progress::ProgressTracker;
use super::types::{IndexJob, Stage};
use super::workers::SharedReceiver;

/// Cloneable dispatcher shared by every index producer and the index worker.
#[derive(Clone)]
pub struct IndexQueue {
    tx: mpsc::UnboundedSender<IndexJob>,
    rx: SharedReceiver<IndexJob>,
    queued: Arc<Mutex<HashSet<(NamespaceId, String)>>>,
    progress: ProgressTracker,
}

impl IndexQueue {
    pub fn new(progress: ProgressTracker) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            tx,
            rx: SharedReceiver::new_unbounded(rx),
            queued: Arc::new(Mutex::new(HashSet::new())),
            progress,
        }
    }

    /// Queue a document for index reconciliation unless one is already
    /// waiting. Progress enqueues exactly the jobs actually sent; a coalesced
    /// duplicate sends nothing and changes nothing.
    pub async fn enqueue(&self, namespace_id: NamespaceId, doc_id: String) {
        let key = (namespace_id, doc_id);
        let mut queued = self.queued.lock().await;
        if !queued.insert(key.clone()) {
            return;
        }
        self.progress
            .enqueue(&namespace_id.to_string(), Stage::Index)
            .await;
        let _ = self.tx.send(IndexJob {
            namespace_id: key.0,
            doc_id: key.1,
        });
    }

    /// Receive the next queued job, releasing its queued marker before the
    /// caller starts the work.
    pub async fn recv(&self) -> Option<IndexJob> {
        let job = self.rx.recv().await?;
        self.queued
            .lock()
            .await
            .remove(&(job.namespace_id, job.doc_id.clone()));
        Some(job)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    const QUIET: Duration = Duration::from_millis(200);

    fn tracker() -> (
        ProgressTracker,
        mpsc::UnboundedReceiver<crate::pipeline::AppEvent>,
    ) {
        let (events_tx, events_rx) = mpsc::unbounded_channel();
        (ProgressTracker::new(events_tx), events_rx)
    }

    async fn recv_within(queue: &IndexQueue, budget: Duration) -> Option<IndexJob> {
        tokio::time::timeout(budget, queue.recv())
            .await
            .ok()
            .flatten()
    }

    /// A second enqueue while the first job still waits is a no-op: it sends
    /// no job and adds no pending progress.
    #[tokio::test]
    async fn suppresses_duplicate_while_a_job_is_queued() {
        let (progress, _events) = tracker();
        let queue = IndexQueue::new(progress.clone());
        let namespace = NamespaceId::default();

        queue.enqueue(namespace, "doc".to_string()).await;
        queue.enqueue(namespace, "doc".to_string()).await;

        let job = recv_within(&queue, Duration::from_secs(1))
            .await
            .expect("queued job was not sent");
        assert_eq!(job.doc_id, "doc");
        assert_eq!(job.namespace_id, namespace);
        assert!(
            recv_within(&queue, QUIET).await.is_none(),
            "duplicate enqueue sent a second job"
        );

        let progress = progress
            .get(&namespace.to_string())
            .await
            .expect("collection progress");
        assert_eq!(progress.index.pending, 1);
        assert_eq!(progress.index.total(), 1);
    }

    /// Document IDs can repeat across collections (source-hash IDs); the
    /// queued marker is per `(namespace, document)`.
    #[tokio::test]
    async fn keeps_namespaces_independent_for_the_same_document_id() {
        let (progress, _events) = tracker();
        let queue = IndexQueue::new(progress.clone());
        let first = NamespaceId::from([1u8; 32]);
        let second = NamespaceId::from([2u8; 32]);

        queue.enqueue(first, "doc".to_string()).await;
        queue.enqueue(second, "doc".to_string()).await;
        queue.enqueue(first, "doc".to_string()).await;

        let mut namespaces = Vec::new();
        for _ in 0..2 {
            namespaces.push(
                recv_within(&queue, Duration::from_secs(1))
                    .await
                    .expect("job was not sent")
                    .namespace_id,
            );
        }
        namespaces.sort_by_key(|namespace| *namespace.as_bytes());
        assert_eq!(namespaces, [first, second]);
        assert!(recv_within(&queue, QUIET).await.is_none());

        for namespace in [first, second] {
            let progress = progress
                .get(&namespace.to_string())
                .await
                .expect("collection progress");
            assert_eq!(progress.index.pending, 1);
        }
    }

    /// Receiving a job releases its marker, so a change during the run queues
    /// one follow-up rather than being dropped.
    #[tokio::test]
    async fn queues_a_follow_up_after_the_job_starts() {
        let (progress, _events) = tracker();
        let queue = IndexQueue::new(progress.clone());
        let namespace = NamespaceId::default();

        queue.enqueue(namespace, "doc".to_string()).await;
        recv_within(&queue, Duration::from_secs(1))
            .await
            .expect("queued job was not sent");

        queue.enqueue(namespace, "doc".to_string()).await;
        queue.enqueue(namespace, "doc".to_string()).await;
        let follow_up = recv_within(&queue, Duration::from_secs(1))
            .await
            .expect("change during the run did not queue a follow-up");
        assert_eq!(follow_up.doc_id, "doc");
        assert!(recv_within(&queue, QUIET).await.is_none());

        let progress = progress
            .get(&namespace.to_string())
            .await
            .expect("collection progress");
        assert_eq!(progress.index.pending, 2);
        assert_eq!(progress.index.total(), 2);
    }
}
