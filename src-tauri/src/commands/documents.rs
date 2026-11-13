use serde::{Deserialize, Serialize};
use tauri::State;

use super::CollectionId;
use crate::core::{AppState, PipelineProgress};
use crate::error::{CommandError, CommandResult, ResultExt};

/// Document metadata returned to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub id: String,
    pub name: String,
    pub file_type: String,
    pub page_count: Option<u32>,
    /// Whether the document is indexed with only partial search coverage.
    /// `None` while the document has no index record yet.
    pub partially_indexed: Option<bool>,
    pub tags: Vec<String>,
    pub created_at: String,
}

/// Event payload for a collection whose document listing may have changed.
///
/// Carries only the identity: listeners re-read storage via `get_documents`.
#[derive(Debug, Clone, Serialize)]
pub struct CollectionInvalidated {
    pub collection_id: String,
}

/// A document retained by a reversible deletion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletedDocumentInfo {
    pub id: String,
    pub name: String,
    /// Author that deleted the document, as the full author id string.
    pub deleted_by: String,
    /// RFC 3339 deletion timestamp.
    pub deleted_at: String,
}

/// A file that could not be stored during an import.
#[derive(Debug, Clone, Serialize)]
pub struct ImportFailure {
    pub path: String,
    pub message: String,
}

/// What an import stored, plus the files it could not store.
#[derive(Debug, Clone, Serialize)]
pub struct ImportOutcome {
    pub documents: Vec<DocumentInfo>,
    pub failures: Vec<ImportFailure>,
}

/// Every literal phrase match within one document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSearchResultsInfo {
    /// Whether part of the document is outside the search index's coverage.
    pub partially_indexed: bool,
    /// Matches in content order.
    pub matches: Vec<DocumentSearchMatchInfo>,
}

/// One occurrence of a searched phrase in a document's content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSearchMatchInfo {
    /// The exact matched substring.
    pub content: String,
    /// Zero-based Unicode character offset of the match in the extraction.
    pub offset: usize,
    /// Unicode character offset one past the match.
    pub next_offset: usize,
    /// Inclusive source pages covered by the match, when the extraction maps
    /// pages.
    pub source_range: Option<SourceRangeInfo>,
}

/// An inclusive range of source page numbers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceRangeInfo {
    pub start: usize,
    pub end: usize,
}

impl From<insight_core::search::DocumentSearchResults> for DocumentSearchResultsInfo {
    fn from(results: insight_core::search::DocumentSearchResults) -> Self {
        Self {
            partially_indexed: results.partially_indexed,
            matches: results
                .matches
                .into_iter()
                .map(DocumentSearchMatchInfo::from)
                .collect(),
        }
    }
}

impl From<insight_core::search::DocumentSearchMatch> for DocumentSearchMatchInfo {
    fn from(matched: insight_core::search::DocumentSearchMatch) -> Self {
        Self {
            content: matched.content,
            offset: matched.offset,
            next_offset: matched.next_offset,
            source_range: matched.source_range.map(|range| SourceRangeInfo {
                start: range.start,
                end: range.end,
            }),
        }
    }
}

/// Start importing files into a collection.
///
/// Runs the store stage of the event-driven import pipeline:
/// 1. Store source → triggers extract
/// 2. Extract text → triggers index
/// 3. Index the whole extraction → document searchable
///
/// Progress flows through the pipeline's progress tracker. Returns the
/// documents this import stored so the caller need not re-read them.
#[tauri::command]
pub async fn start_import(
    paths: Vec<String>,
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<ImportOutcome> {
    let namespace_id = collection_id.namespace();

    tracing::info!(
        "Starting import: {} files into collection {}",
        paths.len(),
        namespace_id
    );

    let paths: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();

    let (documents, errors) = state.pipeline.import_files(namespace_id, paths).await;

    for (path, error) in &errors {
        tracing::error!("Failed to import {:?}: {}", path, error);
    }

    Ok(ImportOutcome {
        documents: documents.into_iter().map(stored_document).collect(),
        failures: errors
            .into_iter()
            .map(|(path, message)| ImportFailure {
                path: path.display().to_string(),
                message,
            })
            .collect(),
    })
}

/// A document that was just stored has no extraction yet, so no page count.
fn stored_document(metadata: insight_core::storage::DocumentMetadata) -> DocumentInfo {
    DocumentInfo {
        id: metadata.id,
        name: metadata.name,
        file_type: metadata.file_type,
        page_count: None,
        partially_indexed: None,
        tags: metadata.tags,
        created_at: metadata.created_at,
    }
}

/// Get pipeline progress for all active collections
///
/// Returns progress for each stage: Store, Extract, Index.
/// Each stage has counts for pending, active, completed, and failed items.
#[tauri::command]
pub async fn get_pipeline_progress(
    state: State<'_, AppState>,
) -> CommandResult<Vec<PipelineProgress>> {
    Ok(state.pipeline.get_all_progress().await)
}

/// Get all documents in a collection
#[tauri::command]
pub async fn get_documents(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<Vec<DocumentInfo>> {
    let storage = state.storage.read().await;

    let namespace_id = collection_id.namespace();
    let collection_id = namespace_id.to_string();
    let documents = storage
        .list_documents_with_page_counts(namespace_id)
        .await
        .storage_err()?;

    let mut infos = Vec::with_capacity(documents.len());
    for (document, page_count) in documents {
        let partially_indexed =
            insight_core::search::is_partially_indexed(&state.search, &collection_id, &document.id)
                .storage_err()?;
        infos.push(DocumentInfo {
            page_count: page_count.map(|p| p as u32),
            partially_indexed,
            id: document.id,
            name: document.name,
            file_type: document.file_type,
            tags: document.tags,
            created_at: document.created_at,
        });
    }
    Ok(infos)
}

/// Get a single document from a collection by ID
#[tauri::command]
pub async fn get_document(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<DocumentInfo> {
    let storage = state.storage.read().await;

    let namespace_id = collection_id.namespace();
    let collection_id = namespace_id.to_string();
    let document = storage
        .get_document(namespace_id, &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::document_not_found())?;
    let page_count = storage
        .page_count(namespace_id, &document.id)
        .await
        .storage_err()?;

    let partially_indexed =
        insight_core::search::is_partially_indexed(&state.search, &collection_id, &document.id)
            .storage_err()?;

    Ok(DocumentInfo {
        page_count: page_count.map(|p| p as u32),
        partially_indexed,
        id: document.id,
        name: document.name,
        file_type: document.file_type,
        tags: document.tags,
        created_at: document.created_at,
    })
}

/// Get the canonical extraction text of a document
#[tauri::command]
pub async fn get_document_text(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let storage = state.storage.read().await;

    let extraction = storage
        .get_canonical_extraction(collection_id.namespace(), &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::text_not_found())?;

    Ok(extraction.text)
}

/// Search one document's indexed content for a literal phrase
///
/// Returns every occurrence in content order. Matches come from the search
/// index only: content outside its coverage is not scanned.
#[tauri::command]
pub async fn search_document(
    collection_id: CollectionId,
    document_id: String,
    query: String,
    state: State<'_, AppState>,
) -> CommandResult<DocumentSearchResultsInfo> {
    let storage = state.storage.read().await;

    let namespace_id = collection_id.namespace();
    let collection_id = namespace_id.to_string();
    let extraction = storage
        .get_canonical_extraction(namespace_id, &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::text_not_found())?;

    let results = insight_core::search::search_document(
        &state.search,
        insight_core::search::DocumentSearchParams {
            query: &query,
            collection_id: &collection_id,
            document_id: &document_id,
            extraction: &extraction,
        },
    )
    .storage_err()?;

    Ok(results.into())
}

/// Get a document's original source file bytes
#[tauri::command]
pub async fn get_document_source(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<tauri::ipc::Response> {
    let storage = state.storage.read().await;
    let namespace_id = collection_id.namespace();

    storage
        .get_document(namespace_id, &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::document_not_found())?;

    let source = storage
        .get_document_source(namespace_id, &document_id)
        .await
        .storage_err()?
        .ok_or(CommandError::source_not_available())?;

    Ok(tauri::ipc::Response::new(source))
}

/// Get the documents retained by reversible deletion in a collection
#[tauri::command]
pub async fn get_deleted_documents(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<Vec<DeletedDocumentInfo>> {
    let storage = state.storage.read().await;
    let deleted = storage
        .list_deleted_documents(collection_id.namespace())
        .await
        .storage_err()?;

    Ok(deleted
        .into_iter()
        .map(|document| DeletedDocumentInfo {
            id: document.metadata.id,
            name: document.metadata.name,
            deleted_by: document.deleted_by.to_string(),
            deleted_at: document.deleted_at,
        })
        .collect())
}

/// Delete a document from a collection
///
/// The deletion is reversible in storage; index reconciliation then evicts
/// the document's index record.
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

    {
        let storage = state.storage.read().await;
        storage
            .delete_document(namespace_id, &document_id)
            .await
            .storage_err()?;
    }

    state
        .pipeline
        .request_index(namespace_id, document_id)
        .await;

    Ok(())
}

/// Restore a deleted document to a collection
///
/// Index reconciliation reindexes the document's canonical extraction.
#[tauri::command]
pub async fn undelete_document(
    collection_id: CollectionId,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let namespace_id = collection_id.namespace();
    tracing::info!(
        "Restoring document {} in collection {}",
        document_id,
        namespace_id
    );

    {
        let storage = state.storage.read().await;
        storage
            .undelete_document(namespace_id, &document_id)
            .await
            .storage_err()?;
    }

    state
        .pipeline
        .request_index(namespace_id, document_id)
        .await;

    Ok(())
}
