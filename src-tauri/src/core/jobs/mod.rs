//! Job queue system for document import pipeline.
//!
//! Architecture:
//!
//! ```text
//! import(paths)                          DocWatcher (per namespace)
//!      │                                       │
//!      ▼                                       │ (subscribes to iroh events)
//! ┌─────────────┐    ┌─────────────┐          │
//! │  extract_tx │───▶│  store_tx   │──▶ iroh ─┘
//! │  (8 tasks)  │    │  (storage)  │     │
//! └─────────────┘    └─────────────┘     │ InsertLocal/InsertRemote
//!                                        ▼
//!                    ┌─────────────┐    ┌─────────────┐
//!                    │  embed_tx   │◀───│  DocWatcher │
//!                    │  (batched)  │    └─────────────┘
//!                    └─────────────┘
//!                          │
//!                          ▼
//!                    ┌─────────────┐
//!                    │  index_tx   │───▶ completed events
//!                    │  (batched)  │
//!                    └─────────────┘
//! ```
//!
//! The store worker stores documents in iroh, which fires events.
//! DocWatcher subscribes to those events and triggers the embed → index pipeline.

mod embed;
mod extract;
mod index;
mod types;
pub mod watcher;
mod worker;

pub use types::{DocumentCompleted, DocumentFailed, Embedded, PipelineProgress, Stored};
pub use watcher::{DocWatcher, DocumentIndexFailed, DocumentIndexed};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use iroh_docs::NamespaceId;
use milli::update::IndexerConfig;
use milli::Index;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use super::embeddings::Embedder;
use super::storage::{DocumentMetadata, Storage};

use extract::ExtractRequest;
use types::Extracted;

/// Coordinates the document import pipeline.
///
/// Manages extraction and storage workers. Indexing is triggered by
/// DocWatcher instances that subscribe to iroh events.
pub struct JobCoordinator {
    /// Sender to submit extraction jobs
    extract_tx: mpsc::Sender<ExtractRequest>,
    /// Sender to the embedding pipeline (shared with DocWatchers)
    embed_tx: mpsc::Sender<Stored>,
    /// Cancellation token to stop all workers
    cancel: CancellationToken,
    /// Receiver for completed documents (from index stage)
    completed_rx: mpsc::Receiver<DocumentCompleted>,
    /// Receiver for failed extractions
    failed_rx: mpsc::Receiver<DocumentFailed>,
    /// Counter for pending extraction jobs
    pending: Arc<AtomicUsize>,
    /// Active document watchers by namespace
    watchers: HashMap<NamespaceId, DocWatcher>,
    /// Storage reference for creating watchers
    storage: Arc<RwLock<Storage>>,
}

impl JobCoordinator {
    /// Create a new job coordinator with the given dependencies.
    ///
    /// Spawns background workers for extraction, embedding, and indexing.
    /// Call `watch_namespace()` to start watching collections for indexing.
    pub fn new(
        storage: Arc<RwLock<Storage>>,
        embedder: Arc<RwLock<Option<Embedder>>>,
        index: Arc<RwLock<Index>>,
        indexer_config: Arc<Mutex<IndexerConfig>>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let pending = Arc::new(AtomicUsize::new(0));

        // Channel for completed document notifications (from indexing)
        let (completed_tx, completed_rx) = mpsc::channel::<DocumentCompleted>(64);

        // Channel for failed extraction notifications
        let (failed_tx, failed_rx) = mpsc::channel::<DocumentFailed>(64);

        // Build embed → index pipeline
        let index_tx = index::spawn(index, indexer_config, cancel.clone(), completed_tx);
        let embed_tx = embed::spawn(embedder, cancel.clone(), index_tx);

        // Storage stage (no longer forwards to embed - just stores)
        let store_tx = spawn_store_worker(storage.clone(), cancel.clone());

        // Extraction stage
        let extract_tx = extract::spawn(cancel.clone(), store_tx, failed_tx);

        Self {
            extract_tx,
            embed_tx,
            cancel,
            completed_rx,
            failed_rx,
            pending,
            watchers: HashMap::new(),
            storage,
        }
    }

    /// Start watching a namespace for document events.
    ///
    /// When documents are added (locally or via sync), they will be
    /// automatically sent through the embed → index pipeline.
    pub fn watch_namespace(&mut self, namespace_id: NamespaceId) {
        if self.watchers.contains_key(&namespace_id) {
            tracing::debug!(namespace = %namespace_id, "Already watching namespace");
            return;
        }

        let watcher = DocWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            self.embed_tx.clone(),
            self.cancel.clone(),
        );

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

    /// Submit paths for import into a collection.
    ///
    /// Returns immediately after queueing. The collection must be watched
    /// via `watch_namespace()` for documents to be indexed.
    pub async fn import(&self, paths: Vec<PathBuf>, collection_id: String) -> usize {
        let count = paths.len();
        self.pending.fetch_add(count, Ordering::SeqCst);

        for path in paths {
            let req = ExtractRequest {
                path,
                collection_id: collection_id.clone(),
            };
            if self.extract_tx.send(req).await.is_err() {
                tracing::warn!("Failed to submit extraction job - worker stopped");
                break;
            }
        }

        count
    }

    /// Receive the next completed document (indexed).
    ///
    /// Returns `None` if the pipeline is shut down.
    pub async fn recv_completed(&mut self) -> Option<DocumentCompleted> {
        let doc = self.completed_rx.recv().await;
        if doc.is_some() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        doc
    }

    /// Receive the next failed extraction.
    ///
    /// Returns `None` if the pipeline is shut down.
    pub async fn recv_failed(&mut self) -> Option<DocumentFailed> {
        let failed = self.failed_rx.recv().await;
        if failed.is_some() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        failed
    }

    /// Try to receive a completed document without blocking.
    pub fn try_recv_completed(&mut self) -> Option<DocumentCompleted> {
        match self.completed_rx.try_recv() {
            Ok(doc) => {
                self.pending.fetch_sub(1, Ordering::SeqCst);
                Some(doc)
            }
            Err(_) => None,
        }
    }

    /// Try to receive a failed extraction without blocking.
    pub fn try_recv_failed(&mut self) -> Option<DocumentFailed> {
        match self.failed_rx.try_recv() {
            Ok(failed) => {
                self.pending.fetch_sub(1, Ordering::SeqCst);
                Some(failed)
            }
            Err(_) => None,
        }
    }

    /// Get the number of pending extraction jobs.
    pub fn pending_count(&self) -> usize {
        self.pending.load(Ordering::SeqCst)
    }

    /// Cancel all in-flight work and shut down workers.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Check if the pipeline has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }
}

/// Spawn the storage worker that stores extracted documents.
///
/// Stores extracted PDFs and text in iroh-blobs, creates metadata entries.
/// Iroh events will trigger the DocWatcher to index the document.
fn spawn_store_worker(
    storage: Arc<RwLock<Storage>>,
    cancel: CancellationToken,
) -> mpsc::Sender<Extracted> {
    let (tx, mut rx) = mpsc::channel::<Extracted>(64);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    tracing::debug!("Store worker cancelled");
                    break;
                }

                Some(extracted) = rx.recv() => {
                    // Store blobs first to get content hashes
                    let storage_guard = storage.read().await;

                    let pdf_hash = match storage_guard.store_blob(&extracted.pdf_bytes).await {
                        Ok(hash) => hash.to_string(),
                        Err(e) => {
                            tracing::error!("Failed to store PDF blob: {}", e);
                            continue;
                        }
                    };

                    // Parse collection ID as namespace (needed for duplicate check)
                    let namespace_id = match extracted.collection_id.parse() {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::error!("Invalid collection ID '{}': {}", extracted.collection_id, e);
                            continue;
                        }
                    };

                    // Check for duplicate by pdf_hash (O(1) lookup via hash index)
                    match storage_guard.has_pdf_hash(namespace_id, &pdf_hash).await {
                        Ok(true) => {
                            tracing::info!(
                                name = %extracted.name,
                                pdf_hash = %pdf_hash,
                                collection_id = %extracted.collection_id,
                                "Duplicate document detected, skipping import"
                            );
                            continue;
                        }
                        Ok(false) => {
                            // No duplicate, proceed with import
                        }
                        Err(e) => {
                            tracing::warn!("Failed to check for duplicates: {}", e);
                            // Continue with import on error - better to have duplicates than miss documents
                        }
                    }

                    let text_hash = match storage_guard.store_blob(extracted.text.as_bytes()).await {
                        Ok(hash) => hash.to_string(),
                        Err(e) => {
                            tracing::error!("Failed to store text blob: {}", e);
                            continue;
                        }
                    };

                    // Generate document ID
                    let doc_id = uuid::Uuid::new_v4().to_string();

                    // Store metadata - this triggers iroh InsertLocal event
                    let metadata = DocumentMetadata {
                        id: doc_id.clone(),
                        name: extracted.name.clone(),
                        pdf_hash,
                        text_hash,
                        page_count: extracted.page_count,
                        tags: vec![],
                        created_at: chrono::Utc::now().to_rfc3339(),
                    };

                    if let Err(e) = storage_guard.add_document(namespace_id, metadata).await {
                        tracing::error!("Failed to store document metadata: {}", e);
                        continue;
                    }

                    tracing::debug!(
                        doc_id = %doc_id,
                        name = %extracted.name,
                        collection_id = %extracted.collection_id,
                        "Document stored, iroh event will trigger indexing"
                    );
                }

                else => {
                    tracing::debug!("Store worker shutting down - channel closed");
                    break;
                }
            }
        }

        tracing::debug!("Store worker stopped");
    });

    tx
}

#[cfg(test)]
mod tests {
    // Integration tests would go here
}
