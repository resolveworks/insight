use serde::Serialize;
use tauri::State;

use super::CollectionId;
use crate::core::storage::CollectionSharingStatus;
use crate::core::{sum_page_counts, AppState, CollectionInfo};
use crate::error::{CommandResult, ResultExt};

/// Event payload for a collection's sharing status or the device network.
///
/// Carries only the namespace whose sharing state changed; `None` means the
/// device's network reachability changed. Listeners re-read the status.
#[derive(Debug, Clone, Serialize)]
pub struct SharingStatusChanged {
    pub namespace: Option<String>,
}

/// Get all collections
#[tauri::command]
pub async fn get_collections(state: State<'_, AppState>) -> CommandResult<Vec<CollectionInfo>> {
    let storage = state.storage.read().await;

    let collections = storage.list_collections().await.storage_err()?;

    // Build CollectionInfo for each collection
    let mut result = Vec::with_capacity(collections.len());
    for (namespace_id, metadata) in collections {
        let documents = storage
            .list_documents(namespace_id)
            .await
            .unwrap_or_default();
        let document_count = documents.len();
        let mut page_counts = Vec::with_capacity(documents.len());
        for document in &documents {
            page_counts.push(
                storage
                    .page_count(namespace_id, &document.id)
                    .await
                    .storage_err()?,
            );
        }
        let total_pages = sum_page_counts(page_counts);
        result.push(CollectionInfo {
            id: namespace_id.to_string(),
            name: metadata.name,
            document_count,
            total_pages,
            created_at: Some(metadata.created_at),
        });
    }

    Ok(result)
}

/// Create a new collection
#[tauri::command]
pub async fn create_collection(
    name: String,
    state: State<'_, AppState>,
) -> CommandResult<CollectionInfo> {
    tracing::info!("Creating collection: {}", name);

    let storage = state.storage.read().await;

    let (namespace_id, metadata) = storage.create_collection(&name).await.storage_err()?;

    drop(storage);

    // Start watching the new collection for sync events (documents from peers)
    state.watch_namespace(namespace_id).await.internal_err()?;

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count: 0,
        total_pages: None,
        created_at: Some(metadata.created_at),
    })
}

/// Delete a collection and all its documents
#[tauri::command]
pub async fn delete_collection(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let namespace_id = collection_id.namespace();
    state.delete_collection(namespace_id).await.storage_err()
}

/// Read a collection's current sharing status.
#[tauri::command]
pub async fn get_collection_sharing_status(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<CollectionSharingStatus> {
    let namespace_id = collection_id.namespace();
    let storage = state.storage.read().await;

    storage
        .collection_sharing_status(namespace_id)
        .await
        .storage_err()
}

/// Create a collaboration ticket for a collection.
///
/// The ticket grants write access; the recipient can import the collection
/// using `import_collection` and then add, change and delete its documents.
#[tauri::command]
pub async fn share_collection(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let namespace_id = collection_id.namespace();
    tracing::info!("Sharing collection {} for collaboration", namespace_id);

    let storage = state.storage.read().await;

    storage.share_collection(namespace_id).await.storage_err()
}

/// Import a collection from a share ticket
///
/// The ticket string is obtained from someone who called `share_collection`.
/// After import, the collection will sync with the original peer.
#[tauri::command]
pub async fn import_collection(
    ticket: String,
    state: State<'_, AppState>,
) -> CommandResult<CollectionInfo> {
    tracing::info!("Importing collection from ticket");

    let (namespace_id, metadata) = state.import_collection(&ticket).await.storage_err()?;

    // The initial sync events may have completed before the watcher attached,
    // so queue pipeline work for whatever the imported state already holds.
    state
        .pipeline
        .resume_collection(namespace_id, &state.search)
        .await
        .internal_err()?;

    // Fetch collection info
    let storage = state.storage.read().await;

    let documents = storage
        .list_documents(namespace_id)
        .await
        .unwrap_or_default();
    let document_count = documents.len();
    let mut page_counts = Vec::with_capacity(documents.len());
    for document in &documents {
        page_counts.push(
            storage
                .page_count(namespace_id, &document.id)
                .await
                .storage_err()?,
        );
    }
    let total_pages = sum_page_counts(page_counts);

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count,
        total_pages,
        created_at: Some(metadata.created_at),
    })
}
