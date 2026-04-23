use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use super::CollectionId;
use crate::core::{AppState, PipelineProgress};
use crate::error::{CommandError, CommandResult, ResultExt};

/// Document metadata returned to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub id: String,
    pub name: String,
    pub file_type: String,
    pub page_count: usize,
    pub tags: Vec<String>,
    pub created_at: String,
}

/// Event payload for document added
#[derive(Debug, Clone, Serialize)]
pub struct DocumentAddedEvent {
    pub collection_id: String,
    pub document: DocumentInfo,
}

/// Start importing files into a collection.
///
/// This queues files for the event-driven import pipeline:
/// 1. Store PDF source → triggers extract
/// 2. Extract text → triggers embed
/// 3. Generate embeddings → triggers index
/// 4. Index → document searchable
///
/// Progress is tracked per-stage via the pipeline.
/// Returns immediately with initial progress.
#[tauri::command]
pub async fn start_import<R: tauri::Runtime>(
    paths: Vec<String>,
    collection_id: CollectionId,
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> CommandResult<PipelineProgress> {
    let namespace_id = collection_id.namespace();
    let collection_id = namespace_id.to_string();

    tracing::info!(
        "Starting import: {} files into collection {}",
        paths.len(),
        collection_id
    );

    // Warn if embedder not ready
    {
        let embedder_guard = state.embedder.read().await;
        if embedder_guard.is_none() {
            tracing::warn!(
                "Embedder not yet configured - documents will be stored but embedding will fail"
            );
        }
    }

    // Convert paths to PathBuf
    let paths: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();

    // Clone pipeline for async task
    let pipeline = state.pipeline.clone();
    let app_clone = app.clone();
    let collection_id_clone = collection_id.clone();

    // Spawn async task to import files
    tokio::spawn(async move {
        let (success, errors) = pipeline.import_files(namespace_id, paths).await;

        tracing::info!(
            "Import complete for {}: {} successful, {} failed",
            collection_id_clone,
            success,
            errors.len()
        );

        for (path, error) in &errors {
            tracing::error!("Failed to import {:?}: {}", path, error);
        }

        // Emit progress update
        if let Some(progress) = pipeline.get_progress(&collection_id_clone).await {
            let _ = app_clone.emit("pipeline-progress", &progress);
        }
    });

    // Return initial progress
    let progress = state
        .pipeline
        .get_progress(&collection_id)
        .await
        .unwrap_or_else(|| PipelineProgress {
            collection_id: collection_id.clone(),
            ..Default::default()
        });

    Ok(progress)
}

/// Get pipeline progress for all active collections
///
/// Returns progress for each stage: Store, Extract, Embed, Index.
/// Each stage has counts for pending, active, completed, and failed items.
#[tauri::command]
pub async fn get_pipeline_progress(
    state: State<'_, AppState>,
) -> CommandResult<Vec<PipelineProgress>> {
    Ok(state.pipeline.get_all_progress().await)
}

/// Get pipeline progress for a specific collection
#[tauri::command]
pub async fn get_collection_pipeline_progress(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<Option<PipelineProgress>> {
    let key = collection_id.namespace().to_string();
    Ok(state.pipeline.get_progress(&key).await)
}

/// Get all documents in a collection
#[tauri::command]
pub async fn get_documents(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<Vec<DocumentInfo>> {
    let storage = state.storage.read().await;

    let documents = storage
        .list_documents(collection_id.namespace())
        .await
        .storage_err()?;

    Ok(documents
        .into_iter()
        .map(|m| DocumentInfo {
            id: m.id,
            name: m.name,
            file_type: m.file_type,
            page_count: m.page_count,
            tags: m.tags,
            created_at: m.created_at,
        })
        .collect())
}

/// Get a single document from a collection by ID
#[tauri::command]
pub async fn get_document(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<DocumentInfo> {
    let storage = state.storage.read().await;

    let document = storage
        .get_document(collection_id.namespace(), &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::document_not_found())?;

    Ok(DocumentInfo {
        id: document.id,
        name: document.name,
        file_type: document.file_type,
        page_count: document.page_count,
        tags: document.tags,
        created_at: document.created_at,
    })
}

/// Get the extracted text content of a document
#[tauri::command]
pub async fn get_document_text(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let storage = state.storage.read().await;

    // Fetch text directly from files/{id}/text entry
    let text_bytes = storage
        .get_document_text(collection_id.namespace(), &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::text_not_found())?;

    String::from_utf8(text_bytes).map_err(|e| CommandError::InvalidUtf8 {
        message: format!("Invalid UTF-8 in text content: {}", e),
    })
}

/// Get the text chunks for a document (read from stored embeddings)
#[tauri::command]
pub async fn get_document_chunks(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Vec<String>> {
    // Get current embedding model ID
    let model_id_guard = state.embedding_model_id.read().await;
    let model_id = model_id_guard
        .as_ref()
        .ok_or(CommandError::embedder_not_configured())?;

    // Fetch stored embeddings
    let storage = state.storage.read().await;
    let embeddings = storage
        .get_embeddings(collection_id.namespace(), &document_id, model_id)
        .await
        .storage_err()?;

    match embeddings {
        Some(data) => Ok(data.chunks.into_iter().map(|c| c.content).collect()),
        None => Ok(vec![]), // No embeddings stored yet
    }
}

/// Delete a document from a collection
#[tauri::command]
pub async fn delete_document(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let namespace_id = collection_id.namespace();
    tracing::info!(
        "Deleting document {} from collection {}",
        document_id,
        namespace_id
    );

    // Delete from storage first
    {
        let storage = state.storage.read().await;
        storage
            .delete_document(namespace_id, &document_id)
            .await
            .storage_err()?;
    }

    // Delete all chunks for this document from search index in background
    let index_worker = state.index_worker.clone();
    tokio::spawn(async move {
        match index_worker
            .delete_document_chunks(document_id.clone())
            .await
        {
            Ok(count) => {
                tracing::info!(
                    "Removed {} chunks for document {} from search index",
                    count,
                    document_id
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to remove chunks for document {} from search index: {}",
                    document_id,
                    e
                );
            }
        }
    });

    Ok(())
}
