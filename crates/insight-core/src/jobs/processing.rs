//! Processing queue for embedding and indexing documents.
//!
//! This is Phase 2 of the two-phase import pipeline:
//! - Phase 1 (Import): Store PDF/text to iroh - tracked by ImportTracker
//! - Phase 2 (Processing): Generate embeddings + index - tracked here
//!
//! Documents enter the processing queue from two sources:
//! - Local imports: after storage completes
//! - Peer sync: when document metadata arrives from peers

use std::collections::HashMap;
use std::sync::Arc;

use iroh_docs::NamespaceId;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};

use crate::embeddings::Embedder;
use crate::storage::{DocumentMetadata, Storage};

use super::index::IndexWorkerHandle;

/// Status of a document being processed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ProcessingStatus {
    /// Queued for processing
    Pending,
    /// Currently being processed (embedding + indexing)
    InProgress,
    /// Successfully processed and indexed
    Completed,
    /// Processing failed
    Failed { error: String },
}

/// Summary of processing progress for a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessingProgress {
    pub collection_id: String,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub pending: usize,
    pub in_progress: usize,
}

/// Internal tracking of a document being processed
#[derive(Debug, Clone)]
struct TrackedDoc {
    collection_id: String,
    status: ProcessingStatus,
}

/// Tracks documents queued for embedding and indexing.
///
/// Unlike ImportTracker (which tracks file paths for local imports only),
/// ProcessingTracker tracks document IDs from both local imports and peer sync.
#[derive(Clone, Default)]
pub struct ProcessingTracker {
    /// Documents indexed by doc_id
    docs: Arc<RwLock<HashMap<String, TrackedDoc>>>,
}

impl ProcessingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a document for processing
    pub async fn queue(&self, collection_id: &str, doc_id: &str) {
        let mut docs = self.docs.write().await;
        docs.insert(
            doc_id.to_string(),
            TrackedDoc {
                collection_id: collection_id.to_string(),
                status: ProcessingStatus::Pending,
            },
        );
    }

    /// Get progress for a collection
    pub async fn get_progress(&self, collection_id: &str) -> ProcessingProgress {
        let docs = self.docs.read().await;
        let collection_docs: Vec<_> = docs
            .values()
            .filter(|d| d.collection_id == collection_id)
            .collect();

        ProcessingProgress {
            collection_id: collection_id.to_string(),
            total: collection_docs.len(),
            completed: collection_docs
                .iter()
                .filter(|d| matches!(d.status, ProcessingStatus::Completed))
                .count(),
            failed: collection_docs
                .iter()
                .filter(|d| matches!(d.status, ProcessingStatus::Failed { .. }))
                .count(),
            pending: collection_docs
                .iter()
                .filter(|d| matches!(d.status, ProcessingStatus::Pending))
                .count(),
            in_progress: collection_docs
                .iter()
                .filter(|d| matches!(d.status, ProcessingStatus::InProgress))
                .count(),
        }
    }

    /// Get progress for all collections with active processing
    pub async fn get_all_progress(&self) -> Vec<ProcessingProgress> {
        let docs = self.docs.read().await;

        // Group by collection
        let mut by_collection: HashMap<&str, Vec<&TrackedDoc>> = HashMap::new();
        for doc in docs.values() {
            by_collection
                .entry(&doc.collection_id)
                .or_default()
                .push(doc);
        }

        by_collection
            .into_iter()
            .filter(|(_, docs)| {
                // Only include collections with active processing
                docs.iter().any(|d| {
                    matches!(
                        d.status,
                        ProcessingStatus::Pending | ProcessingStatus::InProgress
                    )
                })
            })
            .map(|(collection_id, collection_docs)| ProcessingProgress {
                collection_id: collection_id.to_string(),
                total: collection_docs.len(),
                completed: collection_docs
                    .iter()
                    .filter(|d| matches!(d.status, ProcessingStatus::Completed))
                    .count(),
                failed: collection_docs
                    .iter()
                    .filter(|d| matches!(d.status, ProcessingStatus::Failed { .. }))
                    .count(),
                pending: collection_docs
                    .iter()
                    .filter(|d| matches!(d.status, ProcessingStatus::Pending))
                    .count(),
                in_progress: collection_docs
                    .iter()
                    .filter(|d| matches!(d.status, ProcessingStatus::InProgress))
                    .count(),
            })
            .collect()
    }

    /// Mark a document as in progress
    pub async fn mark_in_progress(&self, doc_id: &str) {
        let mut docs = self.docs.write().await;
        if let Some(doc) = docs.get_mut(doc_id) {
            doc.status = ProcessingStatus::InProgress;
        }
    }

    /// Mark a document as completed
    pub async fn mark_completed(&self, doc_id: &str) {
        let mut docs = self.docs.write().await;
        if let Some(doc) = docs.get_mut(doc_id) {
            doc.status = ProcessingStatus::Completed;
        }
    }

    /// Mark a document as failed
    pub async fn mark_failed(&self, doc_id: &str, error: String) {
        let mut docs = self.docs.write().await;
        if let Some(doc) = docs.get_mut(doc_id) {
            doc.status = ProcessingStatus::Failed { error };
        }
    }

    /// Remove all finished documents for a collection
    pub async fn cleanup_collection(&self, collection_id: &str) {
        let mut docs = self.docs.write().await;
        docs.retain(|_, d| {
            d.collection_id != collection_id
                || matches!(
                    d.status,
                    ProcessingStatus::Pending | ProcessingStatus::InProgress
                )
        });
    }

    /// Check if there are documents being processed for a collection
    pub async fn has_active_processing(&self, collection_id: &str) -> bool {
        let docs = self.docs.read().await;
        docs.values().any(|d| {
            d.collection_id == collection_id
                && matches!(
                    d.status,
                    ProcessingStatus::Pending | ProcessingStatus::InProgress
                )
        })
    }
}

/// A document queued for processing (embedding + indexing).
#[derive(Debug, Clone)]
pub struct DocumentToProcess {
    pub namespace_id: NamespaceId,
    pub metadata: DocumentMetadata,
}

/// Event emitted when processing completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ProcessingEvent {
    /// Document was successfully indexed and is now searchable.
    Indexed {
        collection_id: String,
        document_id: String,
        document_name: String,
    },
    /// Document processing failed.
    Failed {
        collection_id: String,
        document_id: String,
        document_name: String,
        error: String,
    },
    /// Processing progress changed (for UI updates).
    ProgressChanged { collection_id: String },
}

/// Handle to queue documents for processing.
///
/// The worker stops when all handles are dropped (channel closes).
#[derive(Clone)]
pub struct ProcessingWorkerHandle {
    tx: mpsc::Sender<DocumentToProcess>,
    tracker: ProcessingTracker,
}

impl ProcessingWorkerHandle {
    /// Queue a document for processing.
    ///
    /// Returns immediately. Processing happens asynchronously.
    pub async fn queue(&self, doc: DocumentToProcess) -> anyhow::Result<()> {
        let collection_id = doc.namespace_id.to_string();

        // Add to tracker first
        self.tracker.queue(&collection_id, &doc.metadata.id).await;

        // Send to worker
        self.tx
            .send(doc)
            .await
            .map_err(|_| anyhow::anyhow!("Processing worker channel closed"))?;

        Ok(())
    }

    /// Get processing progress for a collection.
    pub async fn get_progress(&self, collection_id: &str) -> ProcessingProgress {
        self.tracker.get_progress(collection_id).await
    }

    /// Get processing progress for all collections with active processing.
    pub async fn get_all_progress(&self) -> Vec<ProcessingProgress> {
        self.tracker.get_all_progress().await
    }

    /// Check if there are documents being processed for a collection.
    pub async fn has_active_processing(&self, collection_id: &str) -> bool {
        self.tracker.has_active_processing(collection_id).await
    }

    /// Remove finished documents from tracking for a collection.
    pub async fn cleanup_collection(&self, collection_id: &str) {
        self.tracker.cleanup_collection(collection_id).await;
    }
}

/// Spawn the processing worker.
///
/// The worker runs as a tokio task, processing documents (generating embeddings
/// and indexing) as they arrive. It shares access to the embedder with other
/// parts of the system via RwLock.
///
/// Returns a handle to queue documents and a receiver for processing events.
pub fn spawn_processing_worker(
    storage: Arc<RwLock<Storage>>,
    embedder: Arc<RwLock<Option<Embedder>>>,
    model_id: Arc<RwLock<Option<String>>>,
    index_worker: IndexWorkerHandle,
) -> (ProcessingWorkerHandle, mpsc::Receiver<ProcessingEvent>) {
    let (doc_tx, mut doc_rx) = mpsc::channel::<DocumentToProcess>(64);
    let (event_tx, event_rx) = mpsc::channel::<ProcessingEvent>(64);
    let tracker = ProcessingTracker::new();

    let handle = ProcessingWorkerHandle {
        tx: doc_tx,
        tracker: tracker.clone(),
    };

    tokio::spawn(async move {
        tracing::info!("Processing worker started");

        while let Some(doc) = doc_rx.recv().await {
            let collection_id = doc.namespace_id.to_string();
            let doc_id = doc.metadata.id.clone();
            let doc_name = doc.metadata.name.clone();

            tracing::debug!(doc_id = %doc_id, name = %doc_name, "Processing document");

            // Mark as in progress
            tracker.mark_in_progress(&doc_id).await;
            let _ = event_tx
                .send(ProcessingEvent::ProgressChanged {
                    collection_id: collection_id.clone(),
                })
                .await;

            // Get embedder and model ID
            let embedder_guard = embedder.read().await;
            let model_id_guard = model_id.read().await;

            let result = match (&*embedder_guard, &*model_id_guard) {
                (Some(emb), Some(mid)) => {
                    let storage_guard = storage.read().await;
                    super::process_document(
                        &storage_guard,
                        emb,
                        mid,
                        doc.namespace_id,
                        &index_worker,
                        &doc.metadata,
                    )
                    .await
                }
                _ => Err(anyhow::anyhow!("Embedder not configured")),
            };

            // Drop guards before updating tracker
            drop(embedder_guard);
            drop(model_id_guard);

            // Update tracker and emit event
            match result {
                Ok(_) => {
                    tracker.mark_completed(&doc_id).await;
                    tracing::info!(doc_id = %doc_id, name = %doc_name, "Document indexed");
                    let _ = event_tx
                        .send(ProcessingEvent::Indexed {
                            collection_id: collection_id.clone(),
                            document_id: doc_id,
                            document_name: doc_name,
                        })
                        .await;
                }
                Err(e) => {
                    let error = e.to_string();
                    tracker.mark_failed(&doc_id, error.clone()).await;
                    tracing::error!(doc_id = %doc_id, error = %error, "Failed to process document");
                    let _ = event_tx
                        .send(ProcessingEvent::Failed {
                            collection_id: collection_id.clone(),
                            document_id: doc_id,
                            document_name: doc_name,
                            error,
                        })
                        .await;
                }
            }

            // Emit progress changed
            let _ = event_tx
                .send(ProcessingEvent::ProgressChanged { collection_id })
                .await;
        }

        tracing::info!("Processing worker stopped");
    });

    (handle, event_rx)
}
