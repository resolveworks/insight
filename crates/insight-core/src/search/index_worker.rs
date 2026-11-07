//! Index worker for milli search operations.
//!
//! Owns the milli Index and IndexerConfig, processing write operations
//! in a dedicated thread to avoid blocking the async runtime.
//!
//! LMDB (used by milli) only allows one writer at a time, so serializing
//! writes through a single worker is both correct and efficient.

use std::sync::Arc;
use std::thread;

use milli::update::IndexerConfig;
use milli::Index;
use tokio::sync::{mpsc, oneshot};

use super::{
    delete_chunks_by_collection, delete_document_chunks, index_chunks_batch, ChunkToIndex,
};

/// Request to the index worker.
pub enum IndexRequest {
    /// Index a batch of chunks (creates or updates documents).
    Index {
        chunks: Vec<ChunkToIndex>,
        response_tx: oneshot::Sender<anyhow::Result<()>>,
    },
    /// Delete all chunks belonging to a document in one collection.
    DeleteDocument {
        collection_id: String,
        doc_id: String,
        response_tx: oneshot::Sender<anyhow::Result<usize>>,
    },
    /// Delete all chunks in a collection.
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
}

impl IndexWorkerHandle {
    /// Index a batch of chunks.
    ///
    /// Returns when indexing is complete.
    pub async fn index_chunks(&self, chunks: Vec<ChunkToIndex>) -> anyhow::Result<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(IndexRequest::Index {
                chunks,
                response_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Index worker channel closed"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Index worker dropped response"))?
    }

    /// Delete all chunks for a document in one collection.
    ///
    /// Returns the number of chunks deleted.
    pub async fn delete_document_chunks(
        &self,
        collection_id: String,
        doc_id: String,
    ) -> anyhow::Result<usize> {
        let (response_tx, response_rx) = oneshot::channel();
        self.tx
            .send(IndexRequest::DeleteDocument {
                collection_id,
                doc_id,
                response_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("Index worker channel closed"))?;
        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("Index worker dropped response"))?
    }

    /// Delete all chunks in a collection.
    ///
    /// Returns the number of chunks deleted.
    pub async fn delete_collection_chunks(&self, collection_id: String) -> anyhow::Result<usize> {
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

    thread::Builder::new()
        .name("index-worker".into())
        .spawn(move || {
            tracing::info!("Index worker started");

            // Process requests until channel closed
            while let Some(request) = rx.blocking_recv() {
                process_request(&index, &indexer_config, request);
            }

            tracing::info!("Index worker stopped");
        })
        .expect("Failed to spawn index worker thread");

    IndexWorkerHandle { tx }
}

/// Process a single index request.
fn process_request(index: &Index, indexer_config: &IndexerConfig, request: IndexRequest) {
    match request {
        IndexRequest::Index {
            chunks,
            response_tx,
        } => {
            let chunk_count = chunks.len();
            tracing::debug!(chunk_count, "Processing index request");

            let result = index_chunks_batch(index, indexer_config, chunks);

            if let Err(ref e) = result {
                tracing::error!(error = %e, "Failed to index chunks");
            } else {
                tracing::debug!(chunk_count, "Indexed chunks successfully");
            }

            // Send response (ignore if receiver dropped)
            let _ = response_tx.send(result);
        }

        IndexRequest::DeleteDocument {
            collection_id,
            doc_id,
            response_tx,
        } => {
            tracing::debug!(
                collection_id = %collection_id,
                doc_id = %doc_id,
                "Processing delete document chunks request"
            );

            let result = delete_document_chunks(index, indexer_config, &collection_id, &doc_id);

            if let Err(ref e) = result {
                tracing::error!(
                    collection_id = %collection_id,
                    doc_id = %doc_id,
                    error = %e,
                    "Failed to delete document chunks"
                );
            } else if let Ok(count) = result {
                tracing::debug!(
                    collection_id = %collection_id,
                    doc_id = %doc_id,
                    deleted = count,
                    "Deleted document chunks"
                );
            }

            let _ = response_tx.send(result);
        }

        IndexRequest::DeleteCollection {
            collection_id,
            response_tx,
        } => {
            tracing::debug!(collection_id = %collection_id, "Processing delete collection chunks request");

            let result = delete_chunks_by_collection(index, indexer_config, &collection_id);

            if let Err(ref e) = result {
                tracing::error!(collection_id = %collection_id, error = %e, "Failed to delete collection chunks");
            } else if let Ok(count) = result {
                tracing::debug!(collection_id = %collection_id, deleted = count, "Deleted collection chunks");
            }

            let _ = response_tx.send(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search;
    use crate::search::chunk_external_id;
    use tempfile::tempdir;

    fn test_index() -> (Arc<Index>, IndexerConfig) {
        let dir = tempdir().unwrap();
        let index = search::open_index(dir.path()).unwrap();
        let config = IndexerConfig::default();
        (Arc::new(index), config)
    }

    #[tokio::test]
    async fn test_index_worker_delete_is_scoped_to_collection() {
        let (index, config) = test_index();
        let handle = spawn_index_worker(index.clone(), config);

        let chunks = vec![
            ChunkToIndex {
                id: chunk_external_id("col1", "doc1", 0),
                parent_id: "doc1".to_string(),
                parent_name: "test.pdf".to_string(),
                chunk_index: 0,
                content: "First collection content".to_string(),
                collection_id: "col1".to_string(),
                text_offset_start: 0,
                text_offset_end: 24,
            },
            ChunkToIndex {
                id: chunk_external_id("col2", "doc1", 0),
                parent_id: "doc1".to_string(),
                parent_name: "test.pdf".to_string(),
                chunk_index: 0,
                content: "Second collection content".to_string(),
                collection_id: "col2".to_string(),
                text_offset_start: 0,
                text_offset_end: 25,
            },
        ];
        handle.index_chunks(chunks).await.unwrap();

        let deleted = handle
            .delete_document_chunks("col1".to_string(), "doc1".to_string())
            .await
            .unwrap();
        assert_eq!(deleted, 1);

        assert!(search::get_indexed_chunk_by_external_id(
            &index,
            &chunk_external_id("col1", "doc1", 0)
        )
        .unwrap()
        .is_none());
        assert!(search::get_indexed_chunk_by_external_id(
            &index,
            &chunk_external_id("col2", "doc1", 0)
        )
        .unwrap()
        .is_some());
    }
}
