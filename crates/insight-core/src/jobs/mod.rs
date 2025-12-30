//! Document processing pipeline.
//!
//! Architecture:
//!
//! ```text
//! LOCAL IMPORT (direct)                    REMOTE SYNC (event-driven)
//! ─────────────────────                    ──────────────────────────
//! import_and_index_pdf()                   SyncWatcher (one per namespace)
//!         │                                        │
//!         ▼                                        ▼
//!    store PDF/text                         InsertRemote event
//!         │                                        │
//!         ▼                                        ▼
//!  process_document()  ◄─── shared ───►   process_document()
//!         │                                        │
//!         ▼                                        ▼
//!    Return Result                          Log completion
//! ```
//!
//! Local imports call `process_document()` directly for immediate results.
//! Remote sync uses `SyncWatcher` which calls the same shared function.

mod embed;
mod import;
mod index;
pub mod watcher;

pub use embed::{generate_and_store_embeddings, generate_embeddings_data};
pub use import::{ImportFileStatus, ImportProgress, ImportTracker};
pub use index::{spawn_index_worker, IndexWorkerHandle};
pub use watcher::SyncWatcher;

use std::path::Path;

use iroh_docs::NamespaceId;

use crate::embeddings::Embedder;
use crate::search::ChunkToIndex;
use crate::storage::{DocumentMetadata, EmbeddingData, Storage};

/// Import a PDF and process it immediately (embed + index).
///
/// This is the main entry point for local imports. It:
/// 1. Extracts text and stores the PDF to iroh
/// 2. Generates embeddings
/// 3. Indexes for search
///
/// Returns when the document is fully indexed and searchable.
pub async fn import_and_index_pdf(
    storage: &Storage,
    embedder: &Embedder,
    model_id: &str,
    namespace_id: NamespaceId,
    index_worker: &IndexWorkerHandle,
    path: &Path,
) -> anyhow::Result<DocumentMetadata> {
    // Store PDF to iroh (extracts text, creates metadata)
    let metadata = storage.import_pdf(path, namespace_id).await?;

    // Process immediately (embed + index)
    process_document(
        storage,
        embedder,
        model_id,
        namespace_id,
        index_worker,
        &metadata,
    )
    .await?;

    Ok(metadata)
}

/// Process a document: generate embeddings and index for search.
///
/// This is the shared core function used by both:
/// - Local imports (called directly after storing the document)
/// - Remote sync (called by SyncWatcher when documents arrive from peers)
///
/// Returns the embedding data on success.
pub async fn process_document(
    storage: &Storage,
    embedder: &Embedder,
    model_id: &str,
    namespace_id: NamespaceId,
    index_worker: &IndexWorkerHandle,
    metadata: &DocumentMetadata,
) -> anyhow::Result<EmbeddingData> {
    let collection_id = namespace_id.to_string();

    // Generate embeddings (fetches text from files/{id}/text entry)
    let embedding_data =
        embed::generate_embeddings_data(storage, embedder, model_id, namespace_id, metadata)
            .await?;

    // Store embeddings to iroh so they can be retrieved later and synced to peers
    storage
        .store_embeddings(namespace_id, &metadata.id, embedding_data.clone())
        .await?;

    // Delete old chunks (in case of re-processing)
    let deleted = index_worker
        .delete_document_chunks(metadata.id.clone())
        .await?;
    if deleted > 0 {
        tracing::debug!(doc_id = %metadata.id, deleted, "Deleted old chunks");
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

    let chunk_count = chunks.len();

    // Index chunks
    if !chunks.is_empty() {
        index_worker.index_chunks(chunks).await?;
    }

    tracing::info!(
        doc_id = %metadata.id,
        name = %metadata.name,
        chunk_count,
        "Document processed and indexed"
    );

    Ok(embedding_data)
}

#[cfg(test)]
mod tests {
    // Integration tests would go here
}
