//! Job queue system for document import pipeline.
//!
//! Provides a coordinator that manages separate queues for:
//! - PDF extraction (CPU-bound, parallel via spawn_blocking)
//! - Embedding generation (GPU-bound, batched for efficiency)
//! - Search indexing (I/O-bound, batched for LMDB efficiency)
//!
//! # Architecture
//!
//! ```text
//! import(paths)
//!      │
//!      ▼
//! ┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
//! │  extract_tx │───▶│  store_tx   │───▶│  embed_tx   │───▶│  index_tx   │
//! │  (8 tasks)  │    │  (async)    │    │  (batched)  │    │  (batched)  │
//! └─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
//!                                                                │
//!                                                                ▼
//!                                                          completed_tx
//!                                                          (events)
//! ```

mod embed;
mod extract;
mod index;
mod types;
mod worker;

pub use types::{DocumentCompleted, DocumentFailed, Embedded, PipelineProgress, Stored};

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

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
/// Manages separate worker queues for extraction, embedding, and indexing,
/// each with their own concurrency and batching strategies.
pub struct JobCoordinator {
    /// Sender to submit extraction jobs
    extract_tx: mpsc::Sender<ExtractRequest>,
    /// Cancellation token to stop all workers
    cancel: CancellationToken,
    /// Receiver for completed documents
    completed_rx: mpsc::Receiver<DocumentCompleted>,
    /// Receiver for failed documents
    failed_rx: mpsc::Receiver<DocumentFailed>,
    /// Counter for pending jobs
    pending: Arc<AtomicUsize>,
}

impl JobCoordinator {
    /// Create a new job coordinator with the given dependencies.
    ///
    /// Spawns background workers for each pipeline stage.
    pub fn new(
        storage: Arc<RwLock<Storage>>,
        embedder: Arc<RwLock<Option<Embedder>>>,
        index: Arc<RwLock<Index>>,
        indexer_config: Arc<Mutex<IndexerConfig>>,
    ) -> Self {
        let cancel = CancellationToken::new();
        let pending = Arc::new(AtomicUsize::new(0));

        // Channel for completed document notifications
        let (completed_tx, completed_rx) = mpsc::channel::<DocumentCompleted>(64);

        // Channel for failed document notifications
        let (failed_tx, failed_rx) = mpsc::channel::<DocumentFailed>(64);

        // Build pipeline from back to front:
        // index <- embed <- store <- extract

        // Indexing stage (final)
        let index_tx = index::spawn(index, indexer_config, cancel.clone(), completed_tx);

        // Embedding stage
        let embed_tx = embed::spawn(embedder, cancel.clone(), index_tx);

        // Storage stage (connects extract -> embed)
        let store_tx = spawn_store_worker(storage, cancel.clone(), embed_tx);

        // Extraction stage (first)
        let extract_tx = extract::spawn(cancel.clone(), store_tx, failed_tx);

        Self {
            extract_tx,
            cancel,
            completed_rx,
            failed_rx,
            pending,
        }
    }

    /// Submit paths for import into a collection.
    ///
    /// Returns immediately after queueing. Use `recv_completed()` and
    /// `recv_failed()` to track progress.
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

    /// Receive the next completed document.
    ///
    /// Returns `None` if the pipeline is shut down.
    pub async fn recv_completed(&mut self) -> Option<DocumentCompleted> {
        let doc = self.completed_rx.recv().await;
        if doc.is_some() {
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        doc
    }

    /// Receive the next failed document.
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

    /// Try to receive a failed document without blocking.
    pub fn try_recv_failed(&mut self) -> Option<DocumentFailed> {
        match self.failed_rx.try_recv() {
            Ok(failed) => {
                self.pending.fetch_sub(1, Ordering::SeqCst);
                Some(failed)
            }
            Err(_) => None,
        }
    }

    /// Get the number of pending jobs in the pipeline.
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

/// Spawn the storage worker that connects extraction to embedding.
///
/// Stores extracted PDFs and text in iroh-blobs, creates metadata entries,
/// then forwards to the embedding stage.
fn spawn_store_worker(
    storage: Arc<RwLock<Storage>>,
    cancel: CancellationToken,
    embed_tx: mpsc::Sender<Stored>,
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
                    // Generate document ID
                    let doc_id = uuid::Uuid::new_v4().to_string();

                    // Store blobs
                    let mut storage_guard = storage.write().await;

                    let pdf_hash = match storage_guard.store_blob(&extracted.pdf_bytes).await {
                        Ok(hash) => hash.to_string(),
                        Err(e) => {
                            tracing::error!("Failed to store PDF blob: {}", e);
                            continue;
                        }
                    };

                    let text_hash = match storage_guard.store_blob(extracted.text.as_bytes()).await {
                        Ok(hash) => hash.to_string(),
                        Err(e) => {
                            tracing::error!("Failed to store text blob: {}", e);
                            continue;
                        }
                    };

                    // Parse collection ID as namespace
                    let namespace_id = match extracted.collection_id.parse() {
                        Ok(id) => id,
                        Err(e) => {
                            tracing::error!("Invalid collection ID '{}': {}", extracted.collection_id, e);
                            continue;
                        }
                    };

                    // Store metadata
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

                    drop(storage_guard);

                    // Forward to embedding stage
                    let stored = Stored {
                        doc_id,
                        name: extracted.name,
                        text: extracted.text,
                        collection_id: extracted.collection_id,
                    };

                    if embed_tx.send(stored).await.is_err() {
                        tracing::warn!("Failed to forward to embedding - channel closed");
                        break;
                    }
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
    // Integration tests would go here, but require setting up
    // storage, embedder, and index fixtures
}
