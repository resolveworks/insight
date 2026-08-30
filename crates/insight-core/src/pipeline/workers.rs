//! Worker pools for pipeline stages.

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, RwLock};

use crate::search::{ChunkToIndex, IndexWorkerHandle};
use crate::storage::Storage;

use super::chunk::{chunk_text, find_chunk_offsets};
use super::progress::ProgressTracker;
use super::types::{ExtractJob, IndexJob, ProgressUpdate, Stage};

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
/// Workers extract text from stored PDFs and write to iroh.
/// The InsertLocal event for files/*/text triggers the next stage.
pub fn spawn_extract_workers(
    count: usize,
    rx: SharedReceiver<ExtractJob>,
    storage: Arc<RwLock<Storage>>,
    progress: ProgressTracker,
) {
    for i in 0..count {
        let rx = rx.clone();
        let storage = storage.clone();
        let progress = progress.clone();

        tokio::spawn(async move {
            tracing::debug!(worker = i, "Extract worker started");

            while let Some(job) = rx.recv().await {
                let collection_id = job.namespace_id.to_string();

                // Mark as started
                progress
                    .apply(ProgressUpdate::Started {
                        collection_id: collection_id.clone(),
                        stage: Stage::Extract,
                    })
                    .await;

                // Do the work
                let result = {
                    let storage = storage.read().await;
                    storage
                        .extract_and_dispatch(job.namespace_id, &job.doc_id)
                        .await
                };

                match result {
                    Ok(Some(metadata)) => {
                        tracing::debug!(
                            doc_id = %job.doc_id,
                            pages = metadata.page_count,
                            "Extracted text"
                        );
                        progress
                            .apply(ProgressUpdate::Completed {
                                collection_id,
                                stage: Stage::Extract,
                            })
                            .await;
                        // InsertLocal(files/*/text) will trigger index via watcher
                    }
                    Ok(None) => {
                        tracing::debug!(
                            doc_id = %job.doc_id,
                            "Document deleted before extract; skipping"
                        );
                    }
                    Err(e) => {
                        tracing::error!(doc_id = %job.doc_id, error = %e, "Extract failed");
                        progress
                            .apply(ProgressUpdate::Failed {
                                collection_id,
                                stage: Stage::Extract,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }
            }

            tracing::debug!(worker = i, "Extract worker stopped");
        });
    }
}

/// Spawn index worker.
///
/// Single worker that chunks document text and indexes it into milli
/// for search.
pub fn spawn_index_worker(
    rx: SharedReceiver<IndexJob>,
    storage: Arc<RwLock<Storage>>,
    index_worker: IndexWorkerHandle,
    progress: ProgressTracker,
) {
    tokio::spawn(async move {
        tracing::debug!("Index worker started");

        while let Some(job) = rx.recv().await {
            let collection_id = job.namespace_id.to_string();

            // Mark as started
            progress
                .apply(ProgressUpdate::Started {
                    collection_id: collection_id.clone(),
                    stage: Stage::Index,
                })
                .await;

            // Get metadata and text from storage
            let storage_guard = storage.read().await;
            let metadata_result = storage_guard
                .get_document(job.namespace_id, &job.doc_id)
                .await;
            let metadata = match metadata_result {
                Ok(Some(m)) => m,
                Ok(None) => {
                    // Document deleted before the job ran — skip cleanly.
                    tracing::debug!(
                        doc_id = %job.doc_id,
                        "Document deleted before index; skipping"
                    );
                    drop(storage_guard);
                    continue;
                }
                Err(e) => {
                    tracing::error!(doc_id = %job.doc_id, error = %e, "Index failed");
                    progress
                        .apply(ProgressUpdate::Failed {
                            collection_id,
                            stage: Stage::Index,
                            error: e.to_string(),
                        })
                        .await;
                    drop(storage_guard);
                    continue;
                }
            };

            let text_result = storage_guard
                .get_document_text(job.namespace_id, &job.doc_id)
                .await;
            drop(storage_guard);

            let result = match text_result {
                Ok(Some(text_bytes)) => {
                    let text = match String::from_utf8(text_bytes) {
                        Ok(t) => t,
                        Err(e) => {
                            let err = anyhow::anyhow!(
                                "Invalid UTF-8 in document {}: {}",
                                job.doc_id,
                                e
                            );
                            tracing::error!(doc_id = %job.doc_id, error = %err, "Index failed");
                            progress
                                .apply(ProgressUpdate::Failed {
                                    collection_id,
                                    stage: Stage::Index,
                                    error: err.to_string(),
                                })
                                .await;
                            continue;
                        }
                    };

                    let chunks = chunk_text(&text);
                    if chunks.is_empty() {
                        tracing::warn!(doc_id = %job.doc_id, "Document has no text to index");
                        Ok(())
                    } else {
                        let offsets = find_chunk_offsets(&text, &chunks);

                        // Delete old chunks first
                        if let Err(e) = index_worker
                            .delete_document_chunks(job.doc_id.clone())
                            .await
                        {
                            tracing::warn!(
                                doc_id = %job.doc_id,
                                error = %e,
                                "Failed to delete old chunks"
                            );
                        }

                        let to_index: Vec<ChunkToIndex> = chunks
                            .iter()
                            .enumerate()
                            .map(|(i, content)| {
                                let chunk_start_offset = offsets.get(i).copied().unwrap_or(0);
                                let chunk_end_offset = chunk_start_offset + content.len();
                                let start_page = crate::pdf::char_offset_to_page(
                                    chunk_start_offset,
                                    &metadata.page_boundaries,
                                );
                                let end_page = crate::pdf::char_offset_to_page(
                                    chunk_end_offset,
                                    &metadata.page_boundaries,
                                );

                                ChunkToIndex {
                                    id: format!("{}_chunk_{}", metadata.id, i),
                                    parent_id: metadata.id.clone(),
                                    parent_name: metadata.name.clone(),
                                    chunk_index: i,
                                    content: format!("[{}]\n\n{}", metadata.name, content),
                                    collection_id: collection_id.clone(),
                                    page_count: metadata.page_count,
                                    start_page,
                                    end_page,
                                }
                            })
                            .collect();

                        index_worker.index_chunks(to_index).await
                    }
                }
                Ok(None) => Err(anyhow::anyhow!("Text not found for {}", job.doc_id)),
                Err(e) => Err(e),
            };

            match result {
                Ok(_) => {
                    tracing::info!(doc_id = %job.doc_id, "Document indexed");
                    progress
                        .apply(ProgressUpdate::Completed {
                            collection_id,
                            stage: Stage::Index,
                        })
                        .await;
                }
                Err(e) => {
                    tracing::error!(doc_id = %job.doc_id, error = %e, "Index failed");
                    progress
                        .apply(ProgressUpdate::Failed {
                            collection_id,
                            stage: Stage::Index,
                            error: e.to_string(),
                        })
                        .await;
                }
            }
        }

        tracing::debug!("Index worker stopped");
    });
}
