//! Event-driven document processing pipeline.
//!
//! # Architecture
//!
//! The pipeline is fully event-driven using iroh-docs events:
//!
//! ```text
//! LOCAL IMPORT                     REMOTE SYNC
//! store_pdf_source()               peer sends files/{id}/meta or an extraction
//!       │                                │
//!       ▼                                ▼
//! InsertLocal(source)             InsertRemote(meta/extraction)
//!       │                                │
//!       └────────► WATCHER ◄─────────────┘
//!                      │
//!         ┌────────────┴───────────┐
//!         ▼                        ▼
//!   files/*/source (local)   files/*/meta or
//!         │                  files/*/extractions/*
//!         ▼                        │
//!     Extract worker               ▼
//!         │                  Index worker
//!         ▼                        │
//!   writes an extraction          ▼
//!                          chunks canonical extraction text
//!                          (char-based) → milli index
//! ```
//!
//! Each stage writes to iroh, which triggers the next stage via events.

mod chunk;
mod progress;
mod types;
mod watcher;
mod workers;

pub use chunk::{chunk_text, Chunk};
pub use progress::{PipelineProgress, ProgressTracker, StageProgress};
pub use types::{ExtractJob, IndexJob, ProgressUpdate, Stage};
pub use watcher::{CollectionWatcher, JobSenders};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::search::IndexWorkerHandle;
use crate::storage::{DocumentMetadata, Storage};

use workers::{spawn_extract_workers, SharedReceiver};

/// Number of workers per stage.
const EXTRACT_WORKERS: usize = 4;

/// Events the pipeline surfaces to the application layer.
///
/// Progress is display state; invalidation says only *that* a peer changed a
/// collection's entries, never what they now contain — listeners re-read storage.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Pipeline progress changed for a collection.
    Progress(PipelineProgress),
    /// A peer changed entries in a collection; readers of its document listing must re-read.
    CollectionInvalidated { namespace: NamespaceId },
}

/// Event-driven document processing pipeline.
///
/// Coordinates workers and watchers for document processing.
/// Each collection gets a watcher that dispatches to shared worker pools.
pub struct Pipeline {
    storage: Arc<RwLock<Storage>>,

    // Worker pool channels (unbounded to avoid blocking the event watcher)
    extract_tx: mpsc::UnboundedSender<ExtractJob>,
    index_tx: mpsc::UnboundedSender<IndexJob>,

    // Per-collection watchers
    watchers: Arc<RwLock<HashMap<NamespaceId, CollectionWatcher>>>,

    // Shared progress tracker
    progress: ProgressTracker,

    // Master cancellation token
    cancel: CancellationToken,

    // Application-layer event sink (progress + invalidation)
    events: mpsc::UnboundedSender<AppEvent>,
}

impl Pipeline {
    /// Create a new pipeline.
    ///
    /// Spawns worker pools for extract and index stages.
    /// Returns the pipeline and the receiver for application-layer events.
    pub fn new(
        storage: Arc<RwLock<Storage>>,
        index_worker: IndexWorkerHandle,
    ) -> (Self, mpsc::UnboundedReceiver<AppEvent>) {
        let (events, events_rx) = mpsc::unbounded_channel();
        let progress = ProgressTracker::new(events.clone());
        let cancel = CancellationToken::new();

        // Create unbounded channels (avoids blocking the event watcher)
        let (extract_tx, extract_rx) = mpsc::unbounded_channel();
        let (index_tx, index_rx) = mpsc::unbounded_channel();

        // Shared receivers for multi-worker stages
        let extract_rx = SharedReceiver::new_unbounded(extract_rx);
        let index_rx = SharedReceiver::new_unbounded(index_rx);

        // Spawn worker pools
        spawn_extract_workers(
            EXTRACT_WORKERS,
            extract_rx,
            storage.clone(),
            progress.clone(),
        );

        workers::spawn_index_worker(index_rx, storage.clone(), index_worker, progress.clone());

        tracing::info!(extract_workers = EXTRACT_WORKERS, "Pipeline started");

        (
            Self {
                storage,
                extract_tx,
                index_tx,
                watchers: Arc::new(RwLock::new(HashMap::new())),
                progress,
                cancel,
                events,
            },
            events_rx,
        )
    }

    /// Start watching a collection for events.
    ///
    /// The watcher subscribes to iroh-docs events and dispatches to workers.
    pub async fn watch(&self, namespace_id: NamespaceId) {
        let mut watchers = self.watchers.write().await;

        if watchers.contains_key(&namespace_id) {
            tracing::debug!(namespace = %namespace_id, "Already watching");
            return;
        }

        let senders = JobSenders {
            extract: self.extract_tx.clone(),
            index: self.index_tx.clone(),
            events: self.events.clone(),
        };

        let watcher = CollectionWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            senders,
            self.progress.clone(),
            self.cancel.child_token(),
        );

        watchers.insert(namespace_id, watcher);
        tracing::info!(namespace = %namespace_id, "Started watching collection");
    }

    /// Stop watching a collection.
    pub async fn unwatch(&self, namespace_id: &NamespaceId) {
        let mut watchers = self.watchers.write().await;
        if let Some(watcher) = watchers.remove(namespace_id) {
            watcher.stop();
            tracing::info!(namespace = %namespace_id, "Stopped watching collection");
        }
    }

    /// Import files into a collection.
    ///
    /// Stores each file's source bytes, returning the metadata of every
    /// document it stored. The iroh events drive the rest:
    /// - InsertLocal(source) → Extract
    /// - InsertLocal(extraction) → Index
    pub async fn import_files(
        &self,
        namespace_id: NamespaceId,
        paths: Vec<PathBuf>,
    ) -> (Vec<DocumentMetadata>, Vec<(PathBuf, String)>) {
        let collection_id = namespace_id.to_string();
        let mut documents = Vec::new();
        let mut errors = Vec::new();

        for path in paths {
            // Track store stage
            self.progress
                .apply(ProgressUpdate::Queued {
                    collection_id: collection_id.clone(),
                    stage: Stage::Store,
                })
                .await;

            self.progress
                .apply(ProgressUpdate::Started {
                    collection_id: collection_id.clone(),
                    stage: Stage::Store,
                })
                .await;

            // Store PDF source
            let storage = self.storage.read().await;
            let result = storage.store_pdf_source(&path, namespace_id).await;
            drop(storage);

            match result {
                Ok(metadata) => {
                    tracing::info!(doc_id = %metadata.id, path = %path.display(), "Stored PDF source");
                    self.progress
                        .apply(ProgressUpdate::Completed {
                            collection_id: collection_id.clone(),
                            stage: Stage::Store,
                        })
                        .await;
                    documents.push(metadata);
                    // InsertLocal(files/*/source) will trigger extract via watcher
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to store PDF");
                    self.progress
                        .apply(ProgressUpdate::Failed {
                            collection_id: collection_id.clone(),
                            stage: Stage::Store,
                            error: e.to_string(),
                        })
                        .await;
                    errors.push((path, e.to_string()));
                }
            }
        }

        (documents, errors)
    }

    /// Get progress for a collection.
    pub async fn get_progress(&self, collection_id: &str) -> Option<PipelineProgress> {
        self.progress.get(collection_id).await
    }

    /// Get progress for all active collections.
    pub async fn get_all_progress(&self) -> Vec<PipelineProgress> {
        self.progress.get_all_active().await
    }

    /// Get the progress tracker for external use.
    pub fn progress_tracker(&self) -> &ProgressTracker {
        &self.progress
    }

    /// Shutdown the pipeline.
    pub fn shutdown(&self) {
        self.cancel.cancel();
        tracing::info!("Pipeline shutdown requested");
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
