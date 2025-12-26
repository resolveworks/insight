//! Indexing worker.
//!
//! Batches chunks for efficient LMDB transactions.

use std::sync::Arc;
use std::time::Duration;

use milli::update::IndexerConfig;
use milli::Index;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use crate::search::{self, ChunkToIndex};

use super::types::{DocumentCompleted, Embedded};
use super::worker::{BatchConfig, Batcher};

/// Maximum chunks to batch for a single index transaction.
/// LMDB transactions have overhead, so batching improves throughput.
const BATCH_SIZE: usize = 100;

/// Maximum time to wait for a full batch before processing a partial one.
const BATCH_TIMEOUT: Duration = Duration::from_millis(200);

/// Spawns the indexing worker.
///
/// Returns a sender to submit embedded documents for indexing.
/// Completed documents are notified via `completed_tx`.
pub fn spawn(
    index: Arc<RwLock<Index>>,
    indexer_config: Arc<Mutex<IndexerConfig>>,
    cancel: CancellationToken,
    completed_tx: mpsc::Sender<DocumentCompleted>,
) -> mpsc::Sender<Embedded> {
    let (tx, rx) = mpsc::channel::<Embedded>(64);

    let config = BatchConfig {
        max_size: BATCH_SIZE,
        max_wait: BATCH_TIMEOUT,
    };
    let mut batcher = Batcher::new(rx, config);

    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;

                _ = cancel.cancelled() => {
                    tracing::debug!("Indexing worker cancelled");
                    break;
                }

                batch = batcher.next_batch() => {
                    let Some(docs) = batch else {
                        tracing::debug!("Indexing worker shutting down - channel closed");
                        break;
                    };

                    let doc_count = docs.len();
                    tracing::debug!("Processing indexing batch of {} documents", doc_count);

                    // Collect all chunks and document info for this batch
                    let mut chunks_to_index = Vec::new();
                    let mut doc_infos = Vec::new();

                    for doc in docs {
                        doc_infos.push(DocumentCompleted {
                            doc_id: doc.doc_id.clone(),
                            collection_id: doc.collection_id.clone(),
                        });

                        for chunk in doc.chunks {
                            chunks_to_index.push(ChunkToIndex {
                                id: format!("{}_chunk_{}", doc.doc_id, chunk.index),
                                parent_id: doc.doc_id.clone(),
                                parent_name: doc.name.clone(),
                                chunk_index: chunk.index,
                                content: chunk.content,
                                collection_id: doc.collection_id.clone(),
                                page_count: doc.page_count,
                                vector: chunk.vector,
                            });
                        }
                    }

                    // Index the batch
                    let chunk_count = chunks_to_index.len();
                    if !chunks_to_index.is_empty() {
                        let index_guard = index.read().await;
                        let config_guard = indexer_config.lock().await;

                        if let Err(e) = search::index_chunks_batch(&index_guard, &config_guard, chunks_to_index) {
                            tracing::error!("Failed to index batch: {}", e);
                            // Continue processing - don't stop on indexing errors
                            continue;
                        }

                        tracing::info!(
                            "Indexed {} chunks from {} documents",
                            chunk_count,
                            doc_count
                        );
                    }

                    // Notify completion for each document
                    for doc_info in doc_infos {
                        if completed_tx.send(doc_info).await.is_err() {
                            tracing::warn!("Failed to send completion - channel closed");
                            return;
                        }
                    }
                }
            }
        }

        tracing::debug!("Indexing worker stopped");
    });

    tx
}
