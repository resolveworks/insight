//! Worker pools for pipeline stages.

use std::sync::Arc;

use tokio::sync::{mpsc, Mutex, RwLock};

use crate::embeddings::{generate_embeddings_data, Embedder};
use crate::search::{ChunkToIndex, IndexWorkerHandle};
use crate::storage::Storage;

use super::progress::ProgressTracker;
use super::types::{EmbedJob, ExtractJob, IndexJob, ProgressUpdate, Stage};

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
                        .extract_and_store_text(job.namespace_id, &job.doc_id)
                        .await
                };

                match result {
                    Ok(metadata) => {
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
                        // InsertLocal(files/*/text) will trigger embed via watcher
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

/// Spawn embed workers.
///
/// Workers generate embeddings and store them in iroh.
/// The InsertLocal event for files/*/embeddings/* triggers the next stage.
pub fn spawn_embed_workers(
    count: usize,
    rx: SharedReceiver<EmbedJob>,
    storage: Arc<RwLock<Storage>>,
    embedder: Arc<RwLock<Option<Embedder>>>,
    model_id: Arc<RwLock<Option<String>>>,
    progress: ProgressTracker,
) {
    for i in 0..count {
        let rx = rx.clone();
        let storage = storage.clone();
        let embedder = embedder.clone();
        let model_id = model_id.clone();
        let progress = progress.clone();

        tokio::spawn(async move {
            tracing::debug!(worker = i, "Embed worker started");

            while let Some(job) = rx.recv().await {
                let collection_id = job.namespace_id.to_string();

                // Mark as started
                progress
                    .apply(ProgressUpdate::Started {
                        collection_id: collection_id.clone(),
                        stage: Stage::Embed,
                    })
                    .await;

                // Get embedder and model ID
                let embedder_guard = embedder.read().await;
                let model_id_guard = model_id.read().await;

                let result = match (&*embedder_guard, &*model_id_guard) {
                    (Some(emb), Some(mid)) => {
                        // Get document metadata
                        let storage_guard = storage.read().await;
                        let metadata_result = storage_guard
                            .get_document(job.namespace_id, &job.doc_id)
                            .await;

                        match metadata_result {
                            Ok(Some(metadata)) => {
                                // Generate embeddings
                                let emb_result = generate_embeddings_data(
                                    &storage_guard,
                                    emb,
                                    mid,
                                    job.namespace_id,
                                    &metadata,
                                )
                                .await;

                                match emb_result {
                                    Ok(data) => {
                                        // Store embeddings - triggers InsertLocal
                                        storage_guard
                                            .store_embeddings(job.namespace_id, &job.doc_id, data)
                                            .await
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            Ok(None) => Err(anyhow::anyhow!("Document not found: {}", job.doc_id)),
                            Err(e) => Err(e),
                        }
                    }
                    _ => Err(anyhow::anyhow!("Embedder not configured")),
                };

                drop(embedder_guard);
                drop(model_id_guard);

                match result {
                    Ok(_) => {
                        tracing::debug!(doc_id = %job.doc_id, "Generated embeddings");
                        progress
                            .apply(ProgressUpdate::Completed {
                                collection_id,
                                stage: Stage::Embed,
                            })
                            .await;
                        // InsertLocal(files/*/embeddings/*) will trigger index via watcher
                    }
                    Err(e) => {
                        tracing::error!(doc_id = %job.doc_id, error = %e, "Embed failed");
                        progress
                            .apply(ProgressUpdate::Failed {
                                collection_id,
                                stage: Stage::Embed,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }
            }

            tracing::debug!(worker = i, "Embed worker stopped");
        });
    }
}

/// Spawn index worker.
///
/// Single worker that indexes embeddings into milli for search.
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

            // Get embeddings from storage
            let storage_guard = storage.read().await;
            let embeddings_result = storage_guard
                .get_embeddings(job.namespace_id, &job.doc_id, &job.model_id)
                .await;

            let metadata_result = storage_guard
                .get_document(job.namespace_id, &job.doc_id)
                .await;
            drop(storage_guard);

            let result = match (embeddings_result, metadata_result) {
                (Ok(Some(embedding_data)), Ok(Some(metadata))) => {
                    // Delete old chunks first
                    if let Err(e) = index_worker
                        .delete_document_chunks(job.doc_id.clone())
                        .await
                    {
                        tracing::warn!(doc_id = %job.doc_id, error = %e, "Failed to delete old chunks");
                    }

                    // Build chunks for indexing
                    let chunks: Vec<ChunkToIndex> = embedding_data
                        .chunks
                        .iter()
                        .map(|chunk| {
                            let enriched = format!("[{}]\n\n{}", metadata.name, chunk.content);
                            ChunkToIndex {
                                id: format!("{}_chunk_{}", metadata.id, chunk.index),
                                parent_id: metadata.id.clone(),
                                parent_name: metadata.name.clone(),
                                chunk_index: chunk.index,
                                content: enriched,
                                collection_id: collection_id.clone(),
                                page_count: metadata.page_count,
                                start_page: chunk.start_page,
                                end_page: chunk.end_page,
                                vector: Some(chunk.vector.clone()),
                            }
                        })
                        .collect();

                    if !chunks.is_empty() {
                        index_worker.index_chunks(chunks).await
                    } else {
                        Ok(())
                    }
                }
                (Ok(None), _) => Err(anyhow::anyhow!("Embeddings not found for {}", job.doc_id)),
                (_, Ok(None)) => Err(anyhow::anyhow!("Document not found: {}", job.doc_id)),
                (Err(e), _) | (_, Err(e)) => Err(e),
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
