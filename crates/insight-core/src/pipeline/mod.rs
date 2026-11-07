//! Event-driven document processing pipeline.
//!
//! # Architecture
//!
//! Local imports hand off explicitly between stages:
//!
//! ```text
//! import_files()
//!       │ store_pdf_source()
//!       ▼
//!   Extract worker ──commit extraction──► Index worker
//!                                           │
//!                                           ▼
//!                              chunks canonical extraction text
//!                              (char-based) → milli index
//! ```
//!
//! The watcher only queues index reconciliation for remote `files/` entries.
//! It ignores local insert events because the local import path already
//! handed the work off explicitly.

mod chunk;
mod index_queue;
mod progress;
mod types;
mod watcher;
mod workers;

pub use chunk::{chunk_text, Chunk};
pub use index_queue::IndexQueue;
pub use progress::{PipelineProgress, ProgressTracker, StageProgress};
pub use types::{ExtractJob, IndexJob, Stage};
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

    // Coalescing queue shared by every index producer and the index worker
    index_queue: IndexQueue,

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

        // Create unbounded channel (avoids blocking the event watcher)
        let (extract_tx, extract_rx) = mpsc::unbounded_channel();
        let index_queue = IndexQueue::new(progress.clone());

        // Shared receiver for multi-worker stage
        let extract_rx = SharedReceiver::new_unbounded(extract_rx);

        // Spawn worker pools
        spawn_extract_workers(
            EXTRACT_WORKERS,
            extract_rx,
            storage.clone(),
            index_queue.clone(),
            progress.clone(),
        );

        workers::spawn_index_worker(
            index_queue.clone(),
            storage.clone(),
            index_worker,
            progress.clone(),
        );

        tracing::info!(extract_workers = EXTRACT_WORKERS, "Pipeline started");

        (
            Self {
                storage,
                extract_tx,
                index_queue,
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
            index: self.index_queue.clone(),
            events: self.events.clone(),
        };

        let watcher = CollectionWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            senders,
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
    /// Stores each file's source bytes and queues extraction for every
    /// document it stored, returning the metadata. The extract worker hands
    /// a successful extraction to the index worker.
    pub async fn import_files(
        &self,
        namespace_id: NamespaceId,
        paths: Vec<PathBuf>,
    ) -> (Vec<DocumentMetadata>, Vec<(PathBuf, String)>) {
        let collection_id = namespace_id.to_string();
        let mut documents = Vec::new();
        let mut errors = Vec::new();

        for path in paths {
            // Store runs inline rather than through a worker channel, but
            // still accounts for its job through the same transitions.
            self.progress.enqueue(&collection_id, Stage::Store).await;
            self.progress.start(&collection_id, Stage::Store).await;

            // Store PDF source
            let storage = self.storage.read().await;
            let result = storage.store_pdf_source(&path, namespace_id).await;
            drop(storage);

            match result {
                Ok(metadata) => {
                    tracing::info!(doc_id = %metadata.id, path = %path.display(), "Stored PDF source");
                    // Queue the next stage before settling this one so progress
                    // never reports every stage idle between the handoff.
                    self.progress.enqueue(&collection_id, Stage::Extract).await;
                    self.progress.finish_ok(&collection_id, Stage::Store).await;
                    let _ = self.extract_tx.send(ExtractJob {
                        namespace_id,
                        doc_id: metadata.id.clone(),
                    });
                    documents.push(metadata);
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to store PDF");
                    self.progress.finish_err(&collection_id, Stage::Store).await;
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use milli::update::IndexerConfig;

    use super::*;
    use crate::search::{get_indexed_chunk, search_index, SearchParams};

    /// A local import runs Store → Extract → Index exactly once per document
    /// and the document becomes searchable by its extracted text.
    #[tokio::test]
    async fn import_stores_extracts_indexes_and_becomes_searchable() {
        /// One-page PDF with a standard Helvetica text run.
        const TEXT_PDF: &[u8] = include_bytes!("../../tests/fixtures/text.pdf");

        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(RwLock::new(
            Storage::open(&temp_dir.path().join("iroh")).await.unwrap(),
        ));
        let index = Arc::new(crate::search::open_index(&temp_dir.path().join("search")).unwrap());
        let index_worker =
            crate::search::spawn_index_worker(index.clone(), IndexerConfig::default());
        let (namespace_id, _) = storage
            .read()
            .await
            .create_collection("Import Test")
            .await
            .unwrap();

        let (pipeline, _events) = Pipeline::new(storage.clone(), index_worker);
        pipeline.watch(namespace_id).await;

        let pdf_path = temp_dir.path().join("text.pdf");
        tokio::fs::write(&pdf_path, TEXT_PDF).await.unwrap();
        let (documents, errors) = pipeline.import_files(namespace_id, vec![pdf_path]).await;
        assert!(errors.is_empty(), "import failed: {errors:?}");
        assert_eq!(documents.len(), 1);
        let doc_id = documents[0].id.clone();

        let collection_id = namespace_id.to_string();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let progress = pipeline
                    .get_progress(&collection_id)
                    .await
                    .expect("collection progress");
                assert_eq!(progress.store.failed, 0);
                assert_eq!(progress.extract.failed, 0);
                assert_eq!(progress.index.failed, 0);
                if progress.store.completed == 1
                    && progress.extract.completed == 1
                    && progress.index.completed == 1
                    && !progress.is_active()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("pipeline did not settle");

        let results = search_index(
            &index,
            SearchParams {
                query: "digital",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1, "imported document is not searchable");
        let rtxn = index.read_txn().unwrap();
        let chunk = get_indexed_chunk(&index, &rtxn, results.hits[0].doc_id)
            .unwrap()
            .expect("hit must deserialize as an indexed chunk");
        assert_eq!(chunk.parent_id, doc_id);

        pipeline.shutdown();
    }
}
