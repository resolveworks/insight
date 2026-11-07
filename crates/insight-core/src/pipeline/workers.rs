//! Worker pools for pipeline stages.

use std::sync::Arc;

use anyhow::Context;
use tokio::sync::{mpsc, Mutex, RwLock};

use crate::search::{chunk_external_id, ChunkToIndex, IndexWorkerHandle};
use crate::storage::{CommitOutcome, DocumentMetadata, Storage};

use super::chunk::chunk_text;
use super::index_queue::IndexQueue;
use super::progress::ProgressTracker;
use super::types::{ExtractJob, Stage};

/// Shared receiver for multiple workers pulling from one unbounded channel.
pub struct SharedReceiver<T> {
    rx: Arc<Mutex<mpsc::UnboundedReceiver<T>>>,
}

impl<T> SharedReceiver<T> {
    pub fn new_unbounded(rx: mpsc::UnboundedReceiver<T>) -> Self {
        Self {
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub async fn recv(&self) -> Option<T> {
        self.rx.lock().await.recv().await
    }
}

impl<T> Clone for SharedReceiver<T> {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.clone(),
        }
    }
}

/// Spawn extract workers.
///
/// Workers extract text from stored PDFs and write an extraction to iroh.
/// A written extraction is queued for index reconciliation; terminal
/// outcomes with no extraction to index are not.
pub fn spawn_extract_workers(
    count: usize,
    rx: SharedReceiver<ExtractJob>,
    storage: Arc<RwLock<Storage>>,
    index_queue: IndexQueue,
    progress: ProgressTracker,
) {
    for i in 0..count {
        let rx = rx.clone();
        let storage = storage.clone();
        let index_queue = index_queue.clone();
        let progress = progress.clone();

        tokio::spawn(async move {
            tracing::debug!(worker = i, "Extract worker started");

            while let Some(job) = rx.recv().await {
                let collection_id = job.namespace_id.to_string();

                progress.start(&collection_id, Stage::Extract).await;

                match extract_document(&storage, &job).await {
                    Ok(Some(metadata)) => {
                        tracing::debug!(
                            doc_id = %job.doc_id,
                            parser = ?metadata.extraction.as_ref().map(|p| p.key()),
                            "Extracted text"
                        );
                        progress.finish_ok(&collection_id, Stage::Extract).await;
                        index_queue
                            .enqueue(job.namespace_id, job.doc_id.clone())
                            .await;
                    }
                    Ok(None) => {
                        // The job is still terminal: the document vanished or
                        // already had a canonical extraction. Neither leaves
                        // anything new to index.
                        progress.finish_ok(&collection_id, Stage::Extract).await;
                    }
                    Err(e) => {
                        tracing::error!(doc_id = %job.doc_id, error = %e, "Extract failed");
                        progress.finish_err(&collection_id, Stage::Extract).await;
                    }
                }
            }

            tracing::debug!(worker = i, "Extract worker stopped");
        });
    }
}

/// Spawn index worker.
///
/// Single worker that reconciles a document's chunks in milli with its
/// current canonical extraction. A document that is gone or has no local
/// canonical text loses its stale chunks.
pub fn spawn_index_worker(
    queue: IndexQueue,
    storage: Arc<RwLock<Storage>>,
    index_worker: IndexWorkerHandle,
    progress: ProgressTracker,
) {
    tokio::spawn(async move {
        tracing::debug!("Index worker started");

        while let Some(job) = queue.recv().await {
            let collection_id = job.namespace_id.to_string();

            progress.start(&collection_id, Stage::Index).await;

            let result = reconcile_document(
                &storage,
                &index_worker,
                job.namespace_id,
                &collection_id,
                &job.doc_id,
            )
            .await;

            match result {
                Ok(()) => {
                    tracing::info!(doc_id = %job.doc_id, "Document indexed");
                    progress.finish_ok(&collection_id, Stage::Index).await;
                }
                Err(e) => {
                    tracing::error!(doc_id = %job.doc_id, error = %e, "Index failed");
                    progress.finish_err(&collection_id, Stage::Index).await;
                }
            }
        }

        tracing::debug!("Index worker stopped");
    });
}

/// Extract a document's text and commit the resulting extraction.
///
/// `Ok(None)` is a terminal no-op: the document is gone or already has a
/// canonical extraction. The caller owns progress accounting.
async fn extract_document(
    storage: &RwLock<Storage>,
    job: &ExtractJob,
) -> anyhow::Result<Option<DocumentMetadata>> {
    let input = {
        let storage = storage.read().await;
        storage
            .extraction_input(job.namespace_id, &job.doc_id)
            .await?
    };
    let Some((_, source)) = input else {
        tracing::debug!(
            doc_id = %job.doc_id,
            "Extract has no work: document gone or already canonical"
        );
        return Ok(None);
    };

    // The storage guard is held only for the short input read and commit,
    // never across MuPDF.
    let extraction =
        tokio::task::spawn_blocking(move || crate::pdf::extract_text_from_bytes(source))
            .await
            .context("Extract task panicked")??;

    let storage = storage.read().await;
    match storage
        .commit_extraction(job.namespace_id, &job.doc_id, &extraction)
        .await?
    {
        CommitOutcome::Written(metadata) => Ok(Some(metadata)),
        CommitOutcome::DocumentGone => {
            tracing::debug!(doc_id = %job.doc_id, "Document deleted during extract");
            Ok(None)
        }
    }
}

/// Bring one document's chunks in one collection in line with storage.
///
/// Missing documents and missing or not-yet-local canonical text evict stale
/// chunks. Empty canonical text evicts them too; non-empty text replaces them.
async fn reconcile_document(
    storage: &RwLock<Storage>,
    index_worker: &IndexWorkerHandle,
    namespace_id: iroh_docs::NamespaceId,
    collection_id: &str,
    doc_id: &str,
) -> anyhow::Result<()> {
    let (metadata, extraction) = {
        let storage = storage.read().await;
        let metadata = storage.get_document(namespace_id, doc_id).await?;
        let extraction = match metadata.as_ref() {
            Some(_) => {
                storage
                    .get_canonical_extraction(namespace_id, doc_id)
                    .await?
            }
            None => None,
        };
        (metadata, extraction)
    };

    index_worker
        .delete_document_chunks(collection_id.to_string(), doc_id.to_string())
        .await?;

    let (Some(metadata), Some(extraction)) = (metadata, extraction) else {
        tracing::debug!(
            doc_id = %doc_id,
            "Document gone or canonical extraction unavailable; evicted stale chunks"
        );
        return Ok(());
    };

    let chunks = chunk_text(&extraction.text);
    if chunks.is_empty() {
        tracing::debug!(doc_id = %doc_id, "Document has no text to index");
        return Ok(());
    }

    let to_index: Vec<ChunkToIndex> = chunks
        .into_iter()
        .enumerate()
        .map(|(i, chunk)| ChunkToIndex {
            id: chunk_external_id(collection_id, &metadata.id, i),
            parent_id: metadata.id.clone(),
            parent_name: metadata.name.clone(),
            chunk_index: i,
            content: format!("[{}]\n\n{}", metadata.name, chunk.text),
            collection_id: collection_id.to_string(),
            text_offset_start: chunk.start,
            text_offset_end: chunk.end,
        })
        .collect();

    index_worker.index_chunks(to_index).await
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::pipeline::progress::ProgressTracker;

    /// A queued extract job whose document was deleted before the worker
    /// picked it up is still terminal: progress settles as completed instead
    /// of staying active, and no document entries are recreated.
    #[tokio::test]
    async fn queued_extract_for_deleted_document_completes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(RwLock::new(Storage::open(temp_dir.path()).await.unwrap()));
        let (namespace_id, _) = storage
            .read()
            .await
            .create_collection("Progress Test")
            .await
            .unwrap();
        let doc_id = storage
            .read()
            .await
            .add_document(namespace_id, "gone.pdf", "application/pdf", b"gone")
            .await
            .unwrap()
            .id;
        storage
            .read()
            .await
            .delete_document(namespace_id, &doc_id)
            .await
            .unwrap();

        let (extract_tx, extract_rx) = mpsc::unbounded_channel();
        let (events_tx, _events_rx) = mpsc::unbounded_channel();
        let progress = ProgressTracker::new(events_tx);
        let index_queue = IndexQueue::new(progress.clone());
        spawn_extract_workers(
            1,
            SharedReceiver::new_unbounded(extract_rx),
            storage.clone(),
            index_queue.clone(),
            progress.clone(),
        );

        let collection_id = namespace_id.to_string();
        progress.enqueue(&collection_id, Stage::Extract).await;
        extract_tx
            .send(ExtractJob {
                namespace_id,
                doc_id: doc_id.clone(),
            })
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let current = progress
                .get(&collection_id)
                .await
                .expect("collection progress");
            if !current.extract.is_active() {
                assert_eq!(current.extract.completed, 1);
                assert_eq!(current.extract.failed, 0);
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "extract progress never settled: {current:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        assert!(
            tokio::time::timeout(Duration::from_millis(200), index_queue.recv())
                .await
                .is_err(),
            "deleted document was indexed"
        );

        let storage = storage.read().await;
        assert!(storage
            .get_document(namespace_id, &doc_id)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_document_source(namespace_id, &doc_id)
            .await
            .unwrap()
            .is_none());
    }
}
