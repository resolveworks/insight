//! Event-driven document processing pipeline.
//!
//! # Architecture
//!
//! The pipeline is fully event-driven using iroh-docs events:
//!
//! ```text
//! LOCAL IMPORT                        REMOTE SYNC
//! ────────────                        ───────────
//! store_pdf_source()                  Peer sends documents
//!       │                                   │
//!       ▼                                   ▼
//! InsertLocal(source)               InsertRemote(text)
//!       │                                   │
//!       └──────────► WATCHER ◄──────────────┘
//!                       │
//!     ┌─────────────────┼─────────────────┐
//!     │                 │                 │
//!     ▼                 ▼                 ▼
//! files/*/source   files/*/text   files/*/embeddings/*
//!     │                 │                 │
//!     ▼                 ▼                 ▼
//! Extract(4)        Embed(2)         Index(1)
//!     │                 │                 │
//!     ▼                 ▼                 ▼
//! InsertLocal       InsertLocal      Document searchable
//! (text)            (embeddings)
//! ```
//!
//! Each stage writes to iroh, which triggers the next stage via events.

mod progress;
mod types;
mod watcher;
mod workers;

pub use progress::{PipelineProgress, ProgressTracker, StageProgress};
pub use types::{EmbedJob, ExtractJob, IndexJob, ProgressUpdate, Stage};
pub use watcher::{CollectionWatcher, JobSenders};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::embeddings::Embedder;
use crate::search::IndexWorkerHandle;
use crate::storage::Storage;

use workers::{spawn_embed_workers, spawn_extract_workers, SharedReceiver};

/// Number of workers per stage.
const EXTRACT_WORKERS: usize = 4;
const EMBED_WORKERS: usize = 2;

/// Event-driven document processing pipeline.
///
/// Coordinates workers and watchers for document processing.
/// Each collection gets a watcher that dispatches to shared worker pools.
pub struct Pipeline {
    storage: Arc<RwLock<Storage>>,
    model_id: Arc<RwLock<Option<String>>>,

    // Worker pool channels (unbounded to avoid blocking the event watcher)
    extract_tx: mpsc::UnboundedSender<ExtractJob>,
    embed_tx: mpsc::UnboundedSender<EmbedJob>,
    index_tx: mpsc::UnboundedSender<IndexJob>,

    // Per-collection watchers
    watchers: Arc<RwLock<HashMap<NamespaceId, CollectionWatcher>>>,

    // Shared progress tracker
    progress: ProgressTracker,

    // Master cancellation token
    cancel: CancellationToken,
}

impl Pipeline {
    /// Create a new pipeline.
    ///
    /// Spawns worker pools for extract, embed, and index stages.
    /// Returns the pipeline and a receiver for progress updates.
    pub fn new(
        storage: Arc<RwLock<Storage>>,
        embedder: Arc<RwLock<Option<Embedder>>>,
        model_id: Arc<RwLock<Option<String>>>,
        index_worker: IndexWorkerHandle,
    ) -> (Self, mpsc::Receiver<PipelineProgress>) {
        let (progress, progress_rx) = ProgressTracker::new();
        let cancel = CancellationToken::new();

        // Create unbounded channels (avoids blocking the event watcher)
        let (extract_tx, extract_rx) = mpsc::unbounded_channel();
        let (embed_tx, embed_rx) = mpsc::unbounded_channel();
        let (index_tx, index_rx) = mpsc::unbounded_channel();

        // Shared receivers for multi-worker stages
        let extract_rx = SharedReceiver::new_unbounded(extract_rx);
        let embed_rx = SharedReceiver::new_unbounded(embed_rx);
        let index_rx = SharedReceiver::new_unbounded(index_rx);

        // Spawn worker pools
        spawn_extract_workers(
            EXTRACT_WORKERS,
            extract_rx,
            storage.clone(),
            progress.clone(),
        );

        spawn_embed_workers(
            EMBED_WORKERS,
            embed_rx,
            storage.clone(),
            embedder.clone(),
            model_id.clone(),
            progress.clone(),
        );

        workers::spawn_index_worker(
            index_rx,
            storage.clone(),
            index_worker.clone(),
            progress.clone(),
        );

        tracing::info!(
            extract_workers = EXTRACT_WORKERS,
            embed_workers = EMBED_WORKERS,
            "Pipeline started"
        );

        (
            Self {
                storage,
                model_id,
                extract_tx,
                embed_tx,
                index_tx,
                watchers: Arc::new(RwLock::new(HashMap::new())),
                progress,
                cancel,
            },
            progress_rx,
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
            embed: self.embed_tx.clone(),
            index: self.index_tx.clone(),
        };

        let watcher = CollectionWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            self.model_id.clone(),
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
    /// Stores each file's source bytes. The iroh events drive the rest:
    /// - InsertLocal(source) → Extract
    /// - InsertLocal(text) → Embed
    /// - InsertLocal(embeddings) → Index
    ///
    /// Returns (successful_count, errors).
    pub async fn import_files(
        &self,
        namespace_id: NamespaceId,
        paths: Vec<PathBuf>,
    ) -> (usize, Vec<(PathBuf, String)>) {
        let collection_id = namespace_id.to_string();
        let mut success = 0;
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
                Ok((doc_id, _hash)) => {
                    tracing::info!(doc_id = %doc_id, path = %path.display(), "Stored PDF source");
                    self.progress
                        .apply(ProgressUpdate::Completed {
                            collection_id: collection_id.clone(),
                            stage: Stage::Store,
                        })
                        .await;
                    success += 1;
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

        (success, errors)
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
