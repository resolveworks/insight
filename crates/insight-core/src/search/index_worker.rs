//! Index worker for milli search operations.
//!
//! Owns the milli Index and IndexerConfig, processing write operations
//! in a dedicated thread to avoid blocking the async runtime.
//!
//! LMDB (used by milli) only allows one writer at a time, so serializing
//! writes through a single worker is both correct and efficient.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::thread;

use milli::update::IndexerConfig;
use milli::Index;
use tokio::sync::{mpsc, oneshot};

use super::{
    delete_document, delete_documents_by_collection, index_documents_batch, DocumentToIndex,
};

/// Rendered collection-term context, cleared when a collection's documents change.
pub type TermsCache = Arc<Mutex<HashMap<String, String>>>;

/// Request to the index worker.
pub enum IndexRequest {
    /// Index a batch of whole documents (creates or updates records).
    Index {
        documents: Vec<DocumentToIndex>,
        response_tx: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Delete a document's index record in one collection.
    DeleteDocument {
        collection_id: String,
        document_id: String,
        response_tx: oneshot::Sender<anyhow::Result<bool>>,
    },
    /// Delete every document index record in a collection.
    DeleteCollection {
        collection_id: String,
        response_tx: oneshot::Sender<anyhow::Result<usize>>,
    },
}

/// Handle to send requests to the index worker.
///
/// The worker stops when all handles are dropped (channel closes).
#[derive(Clone)]
pub struct IndexWorkerHandle {
    tx: mpsc::Sender<IndexRequest>,
    terms_cache: TermsCache,
}

impl IndexWorkerHandle {
    /// The rendered term context cache invalidated by this worker's writes.
    pub fn terms_cache(&self) -> TermsCache {
        self.terms_cache.clone()
    }

    /// Index a batch of whole documents.
    ///
    /// Returns when indexing is complete.
    pub async fn index_documents(&self, documents: Vec<DocumentToIndex>) -> anyhow::Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(IndexRequest::Index {
                documents,
                response_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Index worker channel closed"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Index worker dropped response"))?
    }

    /// Delete a document's index record in one collection.
    ///
    /// Returns whether a record was deleted.
    pub async fn delete_document(
        &self,
        collection_id: String,
        document_id: String,
    ) -> anyhow::Result<bool> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(IndexRequest::DeleteDocument {
                collection_id,
                document_id,
                response_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Index worker channel closed"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Index worker dropped response"))?
    }

    /// Delete every document index record in a collection.
    ///
    /// Returns the number of records deleted.
    pub async fn delete_collection(&self, collection_id: String) -> anyhow::Result<usize> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(IndexRequest::DeleteCollection {
                collection_id,
                response_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Index worker channel closed"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Index worker dropped response"))?
    }
}

/// Spawn the index worker.
///
/// The worker runs in a dedicated OS thread, processing index write operations
/// without blocking the async runtime. This is necessary because milli uses
/// LMDB (blocking I/O) and rayon (blocking thread pool).
///
/// Returns a handle to send requests to the worker. The worker stops when
/// all handles are dropped (channel closes).
pub fn spawn_index_worker(index: Arc<Index>, indexer_config: IndexerConfig) -> IndexWorkerHandle {
    let (tx, mut rx) = mpsc::channel::<IndexRequest>(64);
    let terms_cache = TermsCache::default();
    let worker_terms_cache = terms_cache.clone();

    thread::Builder::new()
        .name("index-worker".into())
        .spawn(move || {
            tracing::info!("Index worker started");

            // Process requests until channel closed
            while let Some(request) = rx.blocking_recv() {
                process_request(&index, &indexer_config, &worker_terms_cache, request);
            }

            tracing::info!("Index worker stopped");
        })
        .expect("Failed to spawn index worker thread");

    IndexWorkerHandle { tx, terms_cache }
}

/// Process a single index request.
fn process_request(
    index: &Index,
    indexer_config: &IndexerConfig,
    terms_cache: &TermsCache,
    request: IndexRequest,
) {
    match request {
        IndexRequest::Index {
            documents,
            response_tx,
        } => {
            let document_count = documents.len();
            tracing::debug!(document_count, "Processing index request");

            let collections: HashSet<String> = documents
                .iter()
                .map(|document| document.collection_id.clone())
                .collect();
            let result = index_documents_batch(index, indexer_config, documents);

            if let Err(ref e) = result {
                tracing::error!(error = %e, "Failed to index documents");
            } else {
                let mut cache = terms_cache.lock().expect("terms cache lock poisoned");
                for collection_id in collections {
                    cache.remove(&collection_id);
                }
                tracing::debug!(document_count, "Indexed documents successfully");
            }

            // Send response (ignore if receiver dropped)
            let _ = response_tx.send(result);
        }

        IndexRequest::DeleteDocument {
            collection_id,
            document_id,
            response_tx,
        } => {
            tracing::debug!(
                collection_id = %collection_id,
                document_id = %document_id,
                "Processing delete document request"
            );

            let result = delete_document(index, indexer_config, &collection_id, &document_id);

            if let Err(ref e) = result {
                tracing::error!(
                    collection_id = %collection_id,
                    document_id = %document_id,
                    error = %e,
                    "Failed to delete document"
                );
            } else if let Ok(deleted) = result {
                terms_cache
                    .lock()
                    .expect("terms cache lock poisoned")
                    .remove(&collection_id);
                tracing::debug!(
                    collection_id = %collection_id,
                    document_id = %document_id,
                    deleted,
                    "Deleted document"
                );
            }

            let _ = response_tx.send(result);
        }

        IndexRequest::DeleteCollection {
            collection_id,
            response_tx,
        } => {
            tracing::debug!(collection_id = %collection_id, "Processing delete collection request");

            let result = delete_documents_by_collection(index, indexer_config, &collection_id);

            if let Err(ref e) = result {
                tracing::error!(collection_id = %collection_id, error = %e, "Failed to delete collection documents");
            } else if let Ok(count) = result {
                terms_cache
                    .lock()
                    .expect("terms cache lock poisoned")
                    .remove(&collection_id);
                tracing::debug!(collection_id = %collection_id, deleted = count, "Deleted collection documents");
            }

            let _ = response_tx.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search;
    use tempfile::tempdir;

    fn test_index() -> (Arc<Index>, IndexerConfig) {
        let dir = tempdir().unwrap();
        let index = search::open_index(dir.path()).unwrap();
        let config = IndexerConfig::default();
        (Arc::new(index), config)
    }

    fn document(collection_id: &str, document_id: &str, content: &str) -> DocumentToIndex {
        DocumentToIndex {
            id: search::document_external_id(collection_id, document_id),
            document_id: document_id.to_string(),
            document_name: "test.pdf".to_string(),
            content: content.to_string(),
            collection_id: collection_id.to_string(),
        }
    }

    #[tokio::test]
    async fn test_index_worker_delete_is_scoped_to_collection() {
        let (index, config) = test_index();
        let handle = spawn_index_worker(index.clone(), config);

        let documents = vec![
            document("col1", "doc1", "First collection content"),
            document("col2", "doc1", "Second collection content"),
        ];
        handle.index_documents(documents).await.unwrap();

        let deleted = handle
            .delete_document("col1".to_string(), "doc1".to_string())
            .await
            .unwrap();
        assert!(deleted);

        assert!(search::get_indexed_document_by_external_id(
            &index,
            &search::document_external_id("col1", "doc1")
        )
        .unwrap()
        .is_none());
        assert!(search::get_indexed_document_by_external_id(
            &index,
            &search::document_external_id("col2", "doc1")
        )
        .unwrap()
        .is_some());
    }
}
