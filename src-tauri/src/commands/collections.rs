use tauri::State;

use super::CollectionId;
use crate::core::{AppState, CollectionInfo};
use crate::error::{CommandError, CommandResult, ResultExt};

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
        let total_pages: usize = documents.iter().map(|d| d.page_count).sum();
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
    state.watch_namespace(namespace_id).await;

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count: 0,
        total_pages: 0,
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
    let collection_id = namespace_id.to_string();
    tracing::info!("Deleting collection {}", collection_id);

    // Delete from storage first
    {
        let storage = state.storage.read().await;
        storage
            .delete_collection(namespace_id)
            .await
            .storage_err()?;
    }

    // Delete all chunks from search index in background
    let index_worker = state.index_worker.clone();
    tokio::spawn(async move {
        match index_worker
            .delete_collection_chunks(collection_id.clone())
            .await
        {
            Ok(deleted_count) => {
                tracing::info!(
                    "Removed {} chunks from search index for collection {}",
                    deleted_count,
                    collection_id
                );
            }
            Err(e) => {
                tracing::error!(
                    "Failed to remove chunks from search index for collection {}: {}",
                    collection_id,
                    e
                );
            }
        }
    });

    Ok(())
}

/// Share a collection with others
///
/// Generates a ticket string that can be shared. The recipient can import
/// the collection using `import_collection`.
#[tauri::command]
pub async fn share_collection(
    collection_id: CollectionId,
    writable: bool,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let namespace_id = collection_id.namespace();
    tracing::info!(
        "Sharing collection {} (writable: {})",
        namespace_id,
        writable
    );

    let storage = state.storage.read().await;

    storage
        .share_collection(namespace_id, writable)
        .await
        .storage_err()
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

    let namespace_id = {
        let storage = state.storage.read().await;
        storage.import_collection(&ticket).await.storage_err()?
    };

    // Start watching the imported collection for sync events
    state.watch_namespace(namespace_id).await;

    // Fetch collection info
    let storage = state.storage.read().await;

    let metadata = storage
        .get_collection_metadata(namespace_id)
        .await
        .storage_err()?
        .ok_or(CommandError::collection_not_found())?;

    let documents = storage
        .list_documents(namespace_id)
        .await
        .unwrap_or_default();
    let document_count = documents.len();
    let total_pages: usize = documents.iter().map(|d| d.page_count).sum();

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count,
        total_pages,
        created_at: Some(metadata.created_at),
    })
}
