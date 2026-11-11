//! Event-driven document processing pipeline.
//!
//! # Architecture
//!
//! Local imports hand off explicitly between stages:
//!
//! ```text
//! import_files()
//!       │ store source bytes (format from the path's extension)
//!       ▼
//!   Extract worker ──commit extraction──► Index worker
//!                                           │
//!                                           ▼
//!                              complete canonical extraction text
//!                              → milli index
//! ```
//!
//! The watcher only queues index reconciliation for remote `files/` entries.
//! It ignores local insert events because the local import path already
//! handed the work off explicitly.

mod index_queue;
mod progress;
mod types;
mod watcher;
mod workers;

pub use index_queue::IndexQueue;
pub use progress::{PipelineProgress, ProgressTracker, StageProgress};
pub use types::{ExtractJob, IndexJob, Stage};
pub use watcher::{CollectionWatcher, JobSenders};

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt;
use iroh::Watcher;
use iroh_docs::NamespaceId;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

use crate::search::{indexed_document_ids, IndexWorkerHandle};
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
    /// Live sharing state changed: one collection's when `namespace` is set,
    /// otherwise the device's network reachability.
    SharingStatusChanged { namespace: Option<NamespaceId> },
}

/// Event-driven document processing pipeline.
///
/// Coordinates workers and watchers for document processing.
/// Each collection gets a watcher that dispatches to shared worker pools.
pub struct Pipeline {
    storage: Arc<RwLock<Storage>>,

    // Parser backends extraction dispatches to by stored MIME type
    registry: Arc<crate::parsers::Registry>,

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

        let registry = Arc::new(crate::parsers::default_registry());

        // Relay reachability is display state mirrored by the sharing read
        // model; this task only signals that it changed.
        spawn_relay_watcher(storage.clone(), events.clone(), cancel.child_token());

        // Spawn worker pools
        spawn_extract_workers(
            EXTRACT_WORKERS,
            extract_rx,
            registry.clone(),
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
                registry,
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
    /// Subscribes to iroh-docs events and dispatches them to workers. Returns
    /// once the subscription is active, so a caller may start sync afterwards
    /// without missing events. The result is `true` when this call inserted the
    /// watcher and `false` when it was already watching.
    pub async fn watch(&self, namespace_id: NamespaceId) -> anyhow::Result<bool> {
        let mut watchers = self.watchers.write().await;

        if watchers.contains_key(&namespace_id) {
            tracing::debug!(namespace = %namespace_id, "Already watching");
            return Ok(false);
        }

        let sharing = self.storage.read().await.sharing().clone();
        let senders = JobSenders {
            index: self.index_queue.clone(),
            events: self.events.clone(),
            sharing,
        };

        let watcher = CollectionWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            senders,
            self.cancel.child_token(),
        )
        .await?;

        watchers.insert(namespace_id, watcher);
        tracing::info!(namespace = %namespace_id, "Started watching collection");
        Ok(true)
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

            // An unsupported path fails the store stage like any other
            // store error; the UI reports it per file.
            let result = match self.registry.format_for_path(&path) {
                Ok(format) => {
                    let storage = self.storage.read().await;
                    storage.store_source(&path, namespace_id, format.mime).await
                }
                Err(e) => Err(e),
            };

            match result {
                Ok(metadata) => {
                    tracing::info!(doc_id = %metadata.id, path = %path.display(), "Stored source");
                    // Queue the next stage before settling this one so progress
                    // never reports every stage idle between the handoff.
                    self.enqueue_extract(namespace_id, metadata.id.clone())
                        .await;
                    self.progress.finish_ok(&collection_id, Stage::Store).await;
                    documents.push(metadata);
                }
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "Failed to store source");
                    self.progress.finish_err(&collection_id, Stage::Store).await;
                    errors.push((path, e.to_string()));
                }
            }
        }

        (documents, errors)
    }

    /// Queue catch-up work for a collection after startup.
    ///
    /// Locally authored documents without a canonical extraction are queued
    /// for extraction. Documents with a canonical extraction but no index
    /// record are queued for index reconciliation, as are indexed documents
    /// that no longer exist in storage (their records are evicted).
    ///
    /// Returns after enqueueing the work; the workers run it asynchronously.
    pub async fn resume_collection(
        &self,
        namespace_id: NamespaceId,
        index: &milli::Index,
    ) -> anyhow::Result<()> {
        let collection_id = namespace_id.to_string();

        let (documents, to_extract) = {
            let storage = self.storage.read().await;
            let documents = storage.list_documents(namespace_id).await?;
            let local_author = storage.author_id();
            let mut to_extract = Vec::new();
            for document in &documents {
                if document.extraction.is_none()
                    && storage.source_author(namespace_id, &document.id).await?
                        == Some(local_author)
                {
                    to_extract.push(document.id.clone());
                }
            }
            (documents, to_extract)
        };

        let indexed = indexed_document_ids(index, &collection_id)?;
        let present: HashSet<&str> = documents
            .iter()
            .map(|document| document.id.as_str())
            .collect();

        for doc_id in to_extract {
            self.enqueue_extract(namespace_id, doc_id).await;
        }

        for document in &documents {
            if document.extraction.is_some() && !indexed.contains(&document.id) {
                self.index_queue
                    .enqueue(namespace_id, document.id.clone())
                    .await;
            }
        }

        for document_id in indexed
            .iter()
            .filter(|document_id| !present.contains(document_id.as_str()))
        {
            self.index_queue
                .enqueue(namespace_id, document_id.clone())
                .await;
        }

        Ok(())
    }

    /// Queue index reconciliation for one document.
    ///
    /// Producers that change a document's indexable state outside the watcher
    /// (local delete and undelete) use this: the index worker re-reads storage
    /// and evicts records for absent or deleted documents, or reindexes the
    /// canonical extraction.
    pub async fn request_index(&self, namespace_id: NamespaceId, doc_id: String) {
        self.index_queue.enqueue(namespace_id, doc_id).await;
    }

    /// Queue an extraction job and its progress accounting.
    async fn enqueue_extract(&self, namespace_id: NamespaceId, doc_id: String) {
        self.progress
            .enqueue(&namespace_id.to_string(), Stage::Extract)
            .await;
        let _ = self.extract_tx.send(ExtractJob {
            namespace_id,
            doc_id,
        });
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

/// Emit a device-level sharing status change whenever home relay state changes.
fn spawn_relay_watcher(
    storage: Arc<RwLock<Storage>>,
    events: mpsc::UnboundedSender<AppEvent>,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let statuses = {
            let storage = storage.read().await;
            storage.home_relay_status().stream_updates_only()
        };
        tokio::pin!(statuses);

        loop {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => break,
                status = statuses.next() => {
                    if status.is_none() {
                        break;
                    }
                    if events
                        .send(AppEvent::SharingStatusChanged { namespace: None })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use milli::update::IndexerConfig;
    use tokio::sync::mpsc;

    use super::*;
    use crate::search::{
        document_external_id, get_indexed_document, get_indexed_document_by_external_id,
        indexed_document_ids, search_index, DocumentToIndex, SearchParams,
    };
    use crate::storage::{doc_meta_key, doc_source_key, doc_state_key, DocumentState};
    use crate::text::{Extraction, ParserId};

    /// One-page PDF with a standard Helvetica text run.
    const TEXT_PDF: &[u8] = include_bytes!("../../tests/fixtures/text.pdf");

    /// A real pipeline over temporary storage and a temporary search index.
    struct TestPipeline {
        _temp: tempfile::TempDir,
        storage: Arc<RwLock<Storage>>,
        index: Arc<milli::Index>,
        index_worker: IndexWorkerHandle,
        pipeline: Pipeline,
        _events: mpsc::UnboundedReceiver<AppEvent>,
    }

    async fn test_pipeline() -> TestPipeline {
        let temp = tempfile::tempdir().unwrap();
        let storage = Arc::new(RwLock::new(
            Storage::open(&temp.path().join("iroh")).await.unwrap(),
        ));
        let index = Arc::new(crate::search::open_index(&temp.path().join("search")).unwrap());
        let index_worker =
            crate::search::spawn_index_worker(index.clone(), IndexerConfig::default());
        let (pipeline, events) = Pipeline::new(storage.clone(), index_worker.clone());

        TestPipeline {
            _temp: temp,
            storage,
            index,
            index_worker,
            pipeline,
            _events: events,
        }
    }

    /// A local import runs Store → Extract → Index exactly once per document
    /// and the document becomes searchable by its extracted text.
    #[tokio::test]
    async fn import_stores_extracts_indexes_and_becomes_searchable() {
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
        pipeline.watch(namespace_id).await.unwrap();

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
                offset: 0,
                collection_id: &collection_id,
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1, "imported document is not searchable");
        let rtxn = index.read_txn().unwrap();
        let document = get_indexed_document(&index, &rtxn, results.hits[0].doc_id)
            .unwrap()
            .expect("hit must deserialize as an indexed document");
        assert_eq!(document.document_id, doc_id);

        pipeline.shutdown();
    }
    /// A locally authored document with no canonical extraction is extracted
    /// and indexed by the startup resume pass.
    #[tokio::test]
    async fn resume_extracts_and_indexes_local_document_without_extraction() {
        let test = test_pipeline().await;
        let (namespace_id, _) = test
            .storage
            .read()
            .await
            .create_collection("Resume Local")
            .await
            .unwrap();
        let document = test
            .storage
            .read()
            .await
            .add_document(namespace_id, "text.pdf", "application/pdf", TEXT_PDF)
            .await
            .unwrap();

        test.pipeline
            .resume_collection(namespace_id, &test.index)
            .await
            .unwrap();

        let collection_id = namespace_id.to_string();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let progress = test
                    .pipeline
                    .get_progress(&collection_id)
                    .await
                    .expect("collection progress");
                assert_eq!(progress.extract.failed, 0);
                assert_eq!(progress.index.failed, 0);
                if progress.extract.completed == 1
                    && progress.index.completed == 1
                    && !progress.is_active()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("resumed pipeline did not settle");

        let extraction = test
            .storage
            .read()
            .await
            .get_canonical_extraction(namespace_id, &document.id)
            .await
            .unwrap();
        assert!(
            extraction.is_some(),
            "resume did not extract the local document"
        );

        let results = search_index(
            &test.index,
            SearchParams {
                query: "digital",
                limit: 10,
                offset: 0,
                collection_id: &collection_id,
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1, "resumed document is not searchable");
        let rtxn = test.index.read_txn().unwrap();
        let indexed = get_indexed_document(&test.index, &rtxn, results.hits[0].doc_id)
            .unwrap()
            .expect("hit must deserialize as an indexed document");
        assert_eq!(indexed.document_id, document.id);

        test.pipeline.shutdown();
    }

    /// A source written by a peer is not extracted locally, even though it has
    /// no canonical extraction yet.
    #[tokio::test]
    async fn resume_skips_peer_authored_source_without_extraction() {
        let test = test_pipeline().await;
        let (namespace_id, peer, document_id) = {
            let storage = test.storage.read().await;
            let doc = storage.docs().create().await.unwrap();
            let namespace_id = doc.id();
            let peer = storage.docs().author_create().await.unwrap();
            let source_hash = storage.store_blob(TEXT_PDF).await.unwrap();
            let document_id = source_hash.to_hex();
            let metadata = DocumentMetadata {
                id: document_id.clone(),
                name: "peer.pdf".to_string(),
                file_type: "application/pdf".to_string(),
                extraction: None,
                tags: Vec::new(),
                created_at: "test".to_string(),
            };
            let meta_bytes = serde_json::to_vec(&metadata).unwrap();
            let meta_hash = storage.store_blob(&meta_bytes).await.unwrap();
            doc.set_hash(
                peer,
                doc_meta_key(&document_id).into_bytes(),
                meta_hash,
                meta_bytes.len() as u64,
            )
            .await
            .unwrap();
            doc.set_hash(
                peer,
                doc_source_key(&document_id).into_bytes(),
                source_hash,
                TEXT_PDF.len() as u64,
            )
            .await
            .unwrap();
            let state_bytes = serde_json::to_vec(&DocumentState::Active).unwrap();
            let state_hash = storage.store_blob(&state_bytes).await.unwrap();
            doc.set_hash(
                peer,
                doc_state_key(&document_id).into_bytes(),
                state_hash,
                state_bytes.len() as u64,
            )
            .await
            .unwrap();
            doc.close().await.unwrap();
            (namespace_id, peer, document_id)
        };

        assert_eq!(
            test.storage
                .read()
                .await
                .source_author(namespace_id, &document_id)
                .await
                .unwrap(),
            Some(peer)
        );

        test.pipeline
            .resume_collection(namespace_id, &test.index)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        let extraction = test
            .storage
            .read()
            .await
            .get_canonical_extraction(namespace_id, &document_id)
            .await
            .unwrap();
        assert!(
            extraction.is_none(),
            "peer-authored source was extracted locally"
        );
        assert!(
            test.pipeline
                .get_progress(&namespace_id.to_string())
                .await
                .is_none(),
            "peer-authored source was queued for extraction"
        );

        test.pipeline.shutdown();
    }

    /// A canonical extraction with no index record in its own collection is
    /// queued for indexing, even when the same document ID is indexed
    /// elsewhere.
    #[tokio::test]
    async fn resume_indexes_canonical_extraction_missing_from_its_collection() {
        let test = test_pipeline().await;
        let (namespace_id, _) = test
            .storage
            .read()
            .await
            .create_collection("Resume Canonical")
            .await
            .unwrap();
        let document = test
            .storage
            .read()
            .await
            .add_document(
                namespace_id,
                "report.pdf",
                "application/pdf",
                b"canonical source",
            )
            .await
            .unwrap();

        let parser = ParserId {
            name: "test".to_string(),
            version: "1".to_string(),
        };
        let extraction = Extraction {
            parser: parser.clone(),
            text: "resume marker text".to_string(),
            unit_kind: None,
            units: Vec::new(),
        };
        {
            let storage = test.storage.read().await;
            storage
                .write_extraction(namespace_id, &document.id, &extraction)
                .await
                .unwrap();
            storage
                .set_canonical_extraction(namespace_id, &document.id, &parser)
                .await
                .unwrap();
        }

        let other_collection = "other-collection";
        let other_document = DocumentToIndex {
            id: document_external_id(other_collection, &document.id),
            document_id: document.id.clone(),
            document_name: "report.pdf".to_string(),
            extraction: parser.key(),
            content: "other collection text".to_string(),
            collection_id: other_collection.to_string(),
        };
        test.index_worker
            .index_documents(vec![other_document])
            .await
            .unwrap();

        test.pipeline
            .resume_collection(namespace_id, &test.index)
            .await
            .unwrap();

        let collection_id = namespace_id.to_string();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let progress = test
                    .pipeline
                    .get_progress(&collection_id)
                    .await
                    .expect("collection progress");
                assert_eq!(progress.index.failed, 0);
                if progress.index.completed == 1 && !progress.is_active() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("resume did not index the canonical extraction");

        let indexed = indexed_document_ids(&test.index, &collection_id).unwrap();
        assert!(indexed.contains(&document.id));
        assert!(
            indexed_document_ids(&test.index, other_collection)
                .unwrap()
                .contains(&document.id),
            "the other collection's index record was changed"
        );

        let results = search_index(
            &test.index,
            SearchParams {
                query: "marker",
                limit: 10,
                offset: 0,
                collection_id: &collection_id,
            },
        )
        .unwrap();
        assert_eq!(
            results.hits.len(),
            1,
            "canonical extraction was not indexed"
        );

        test.pipeline.shutdown();
    }

    /// Index records whose document no longer exists in storage are
    /// reconciled away by the startup resume pass.
    #[tokio::test]
    async fn resume_evicts_records_for_documents_absent_from_storage() {
        let test = test_pipeline().await;
        let (namespace_id, _) = test
            .storage
            .read()
            .await
            .create_collection("Resume Evict")
            .await
            .unwrap();
        let collection_id = namespace_id.to_string();
        let ghost_id = "vanished-document";
        let external_id = document_external_id(&collection_id, ghost_id);
        test.index_worker
            .index_documents(vec![DocumentToIndex {
                id: external_id.clone(),
                document_id: ghost_id.to_string(),
                document_name: "vanished.pdf".to_string(),
                extraction: "test@1".to_string(),
                content: "vanished document text".to_string(),
                collection_id: collection_id.clone(),
            }])
            .await
            .unwrap();
        assert!(
            get_indexed_document_by_external_id(&test.index, &external_id)
                .unwrap()
                .is_some()
        );

        test.pipeline
            .resume_collection(namespace_id, &test.index)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let progress = test
                    .pipeline
                    .get_progress(&collection_id)
                    .await
                    .expect("collection progress");
                assert_eq!(progress.index.failed, 0);
                if progress.index.completed == 1 && !progress.is_active() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("resume did not reconcile the vanished document");

        assert!(
            get_indexed_document_by_external_id(&test.index, &external_id)
                .unwrap()
                .is_none(),
            "the index record for a vanished document was not evicted"
        );

        test.pipeline.shutdown();
    }

    /// Queue one reconciliation and wait until the index stage settles with
    /// at least `completed` finished jobs.
    async fn reconcile_and_wait(
        test: &TestPipeline,
        namespace_id: NamespaceId,
        doc_id: &str,
        completed: usize,
    ) {
        test.pipeline
            .request_index(namespace_id, doc_id.to_string())
            .await;
        let collection_id = namespace_id.to_string();
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let progress = test
                    .pipeline
                    .get_progress(&collection_id)
                    .await
                    .expect("collection progress");
                assert_eq!(progress.index.failed, 0);
                if progress.index.completed >= completed && !progress.is_active() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("requested reconciliation did not settle");
    }

    /// A requested reconciliation evicts the index record of a deleted
    /// document and indexes it again once the document is restored.
    #[tokio::test]
    async fn request_index_reconciles_a_reversible_deletion() {
        let test = test_pipeline().await;
        let (namespace_id, _) = test
            .storage
            .read()
            .await
            .create_collection("Reconcile Delete")
            .await
            .unwrap();
        let collection_id = namespace_id.to_string();
        let document = test
            .storage
            .read()
            .await
            .add_document(
                namespace_id,
                "report.pdf",
                "application/pdf",
                b"reconcile source",
            )
            .await
            .unwrap();
        let parser = ParserId {
            name: "test".to_string(),
            version: "1".to_string(),
        };
        let extraction = Extraction {
            parser: parser.clone(),
            text: "reconcile marker text".to_string(),
            unit_kind: None,
            units: Vec::new(),
        };
        {
            let storage = test.storage.read().await;
            storage
                .write_extraction(namespace_id, &document.id, &extraction)
                .await
                .unwrap();
            storage
                .set_canonical_extraction(namespace_id, &document.id, &parser)
                .await
                .unwrap();
        }

        reconcile_and_wait(&test, namespace_id, &document.id, 1).await;
        assert!(
            indexed_document_ids(&test.index, &collection_id)
                .unwrap()
                .contains(&document.id),
            "document's canonical extraction was not indexed"
        );

        test.storage
            .read()
            .await
            .delete_document(namespace_id, &document.id)
            .await
            .unwrap();
        reconcile_and_wait(&test, namespace_id, &document.id, 2).await;
        assert!(
            !indexed_document_ids(&test.index, &collection_id)
                .unwrap()
                .contains(&document.id),
            "deleted document's index record was not evicted"
        );

        test.storage
            .read()
            .await
            .undelete_document(namespace_id, &document.id)
            .await
            .unwrap();
        reconcile_and_wait(&test, namespace_id, &document.id, 3).await;
        assert!(
            indexed_document_ids(&test.index, &collection_id)
                .unwrap()
                .contains(&document.id),
            "restored document's canonical extraction was not indexed"
        );

        test.pipeline.shutdown();
    }
}
