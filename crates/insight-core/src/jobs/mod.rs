//! Job system for document indexing.
//!
//! Architecture:
//!
//! ```text
//! import_document() ──► Store to iroh ──► InsertLocal event
//!                                              │
//!                                              ▼
//!                                        DocWatcher
//!                                              │
//!                         ┌────────────────────┴────────────────────┐
//!                         │                                         │
//!                         ▼                                         ▼
//!                   files/* event                           embeddings/* event
//!                         │                                         │
//!                         ▼                                         ▼
//!                 Embed & store embeddings                    Index chunks
//!                 (triggers embeddings/*)                     Send completion
//! ```
//!
//! The import function stores documents directly to iroh, which fires events.
//! DocWatcher subscribes to those events and:
//! 1. On files/* events: generates embeddings, stores them (triggers embeddings/*)
//! 2. On embeddings/* events: indexes document with vectors, notifies completion

mod embed;
mod index;
mod types;
pub mod watcher;

pub use embed::generate_embeddings;
pub use index::{spawn_index_worker, IndexWorkerHandle};
pub use types::{DocumentCompleted, DocumentFailed};
pub use watcher::{DocWatcher, WatcherContext};

use std::collections::HashMap;
use std::sync::Arc;

use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;
use crate::storage::Storage;

/// Coordinates document watching and completion notifications.
///
/// Manages DocWatcher instances that subscribe to iroh events.
/// Import/storage is handled inline by the caller.
pub struct JobCoordinator {
    /// Sender for document completion notifications (shared with DocWatchers)
    completed_tx: mpsc::Sender<DocumentCompleted>,
    /// Receiver for document completion notifications
    completed_rx: mpsc::Receiver<DocumentCompleted>,
    /// Sender for failed documents
    failed_tx: mpsc::Sender<DocumentFailed>,
    /// Receiver for failed documents
    failed_rx: mpsc::Receiver<DocumentFailed>,
    /// Cancellation token to stop all watchers
    cancel: CancellationToken,
    /// Active document watchers by namespace
    watchers: HashMap<NamespaceId, DocWatcher>,
    /// Storage reference for creating watchers
    storage: Arc<RwLock<Storage>>,
    /// Embedder for generating embeddings
    embedder: Arc<RwLock<Option<Embedder>>>,
    /// Index worker handle for search operations
    index_worker: IndexWorkerHandle,
    /// Current embedding model ID for watchers
    embedding_model_id: Arc<RwLock<Option<String>>>,
}

impl JobCoordinator {
    /// Create a new job coordinator.
    ///
    /// Call `watch_namespace()` to start watching collections for indexing.
    pub fn new(
        storage: Arc<RwLock<Storage>>,
        embedder: Arc<RwLock<Option<Embedder>>>,
        embedding_model_id: Arc<RwLock<Option<String>>>,
        index_worker: IndexWorkerHandle,
    ) -> Self {
        let cancel = CancellationToken::new();

        // Channel for document completion notifications (from watchers)
        let (completed_tx, completed_rx) = mpsc::channel::<DocumentCompleted>(64);

        // Channel for failed documents
        let (failed_tx, failed_rx) = mpsc::channel::<DocumentFailed>(64);

        Self {
            completed_tx,
            completed_rx,
            failed_tx,
            failed_rx,
            cancel,
            watchers: HashMap::new(),
            storage,
            embedder,
            index_worker,
            embedding_model_id,
        }
    }

    /// Start watching a namespace for document events.
    ///
    /// When documents are added (locally or via sync), they will be
    /// automatically embedded and indexed.
    pub fn watch_namespace(&mut self, namespace_id: NamespaceId) {
        if self.watchers.contains_key(&namespace_id) {
            tracing::debug!(namespace = %namespace_id, "Already watching namespace");
            return;
        }

        let ctx = WatcherContext {
            storage: self.storage.clone(),
            embedder: self.embedder.clone(),
            index_worker: self.index_worker.clone(),
            completed_tx: self.completed_tx.clone(),
            failed_tx: self.failed_tx.clone(),
            current_model_id: self.embedding_model_id.clone(),
        };

        let watcher = DocWatcher::spawn(namespace_id, ctx, self.cancel.clone());

        self.watchers.insert(namespace_id, watcher);
        tracing::info!(namespace = %namespace_id, "Started watching namespace");
    }

    /// Stop watching a namespace.
    pub fn unwatch_namespace(&mut self, namespace_id: &NamespaceId) {
        if let Some(watcher) = self.watchers.remove(namespace_id) {
            watcher.stop();
            tracing::info!(namespace = %namespace_id, "Stopped watching namespace");
        }
    }

    /// Check if a namespace is being watched.
    pub fn is_watching(&self, namespace_id: &NamespaceId) -> bool {
        self.watchers.contains_key(namespace_id)
    }

    /// Get the number of namespaces being watched.
    pub fn watcher_count(&self) -> usize {
        self.watchers.len()
    }

    /// Receive the next completed document (indexed).
    ///
    /// Returns `None` if the pipeline is shut down.
    pub async fn recv_completed(&mut self) -> Option<DocumentCompleted> {
        self.completed_rx.recv().await
    }

    /// Try to receive a completed document without blocking.
    pub fn try_recv_completed(&mut self) -> Option<DocumentCompleted> {
        self.completed_rx.try_recv().ok()
    }

    /// Receive the next failed document.
    ///
    /// Returns `None` if the pipeline is shut down.
    pub async fn recv_failed(&mut self) -> Option<DocumentFailed> {
        self.failed_rx.recv().await
    }

    /// Try to receive a failed document without blocking.
    pub fn try_recv_failed(&mut self) -> Option<DocumentFailed> {
        self.failed_rx.try_recv().ok()
    }

    /// Cancel all watchers and shut down.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the coordinator has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

impl Drop for JobCoordinator {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would go here
}
