use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::core::{import_and_index_pdf, search, AppState, CollectionInfo, ImportProgress};
use crate::error::{CommandError, CommandResult, ResultExt};
use iroh_docs::NamespaceId;

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

/// Event payload for document added
#[derive(Debug, Clone, Serialize)]
pub struct DocumentAddedEvent {
    pub collection_id: String,
    pub document: DocumentInfo,
}

// ============================================================================
// Import Commands
// ============================================================================

/// Start importing files into a collection.
///
/// Queues files for import and processes them asynchronously.
/// Progress is reported via `import-progress` events.
/// Returns immediately with initial progress.
#[tauri::command]
pub async fn start_import<R: tauri::Runtime>(
    paths: Vec<String>,
    collection_id: String,
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> CommandResult<ImportProgress> {
    tracing::info!(
        "Starting import: {} files into collection {}",
        paths.len(),
        collection_id
    );

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

    // Verify embedder is ready before queuing
    {
        let embedder_guard = state.embedder.read().await;
        let model_id_guard = state.embedding_model_id.read().await;
        if embedder_guard.is_none() || model_id_guard.is_none() {
            return Err(CommandError::embedder_not_configured());
        }
    }

    // Queue files for import
    state
        .import_tracker
        .queue_files(&collection_id, &paths)
        .await;

    // Emit initial progress event
    let progress = state.import_tracker.get_progress(&collection_id).await;
    let _ = app.emit("import-progress", &progress);

    // Clone what we need for the async task
    let state_clone = state.inner().clone();
    let app_clone = app.clone();

    // Spawn async task to process the imports
    tokio::spawn(async move {
        process_pending_imports(namespace_id, state_clone, app_clone).await;
    });

    Ok(progress)
}

/// Process all pending imports for a collection
async fn process_pending_imports<R: tauri::Runtime>(
    namespace_id: NamespaceId,
    state: AppState,
    app: AppHandle<R>,
) {
    let collection_id = namespace_id.to_string();

    // Get pending file paths
    let pending_paths = state.import_tracker.pending_paths(&collection_id).await;

    for path_str in pending_paths {
        // Mark file as in progress
        state.import_tracker.mark_in_progress(&path_str).await;

        // Emit progress event
        let progress = state.import_tracker.get_progress(&collection_id).await;
        let _ = app.emit("import-progress", &progress);

        // Get embedder and model ID
        let embedder_guard = state.embedder.read().await;
        let model_id_guard = state.embedding_model_id.read().await;

        let (embedder, model_id) = match (&*embedder_guard, &*model_id_guard) {
            (Some(e), Some(m)) => (e, m.clone()),
            _ => {
                state
                    .import_tracker
                    .mark_failed(&path_str, "Embedder not configured".to_string())
                    .await;
                continue;
            }
        };

        let path = std::path::Path::new(&path_str);
        let storage = state.storage.read().await;

        match import_and_index_pdf(
            &storage,
            embedder,
            &model_id,
            namespace_id,
            &state.index_worker,
            path,
        )
        .await
        {
            Ok(metadata) => {
                tracing::info!(doc_id = %metadata.id, name = %metadata.name, "PDF imported and indexed");

                let doc_info = DocumentInfo {
                    id: metadata.id.clone(),
                    name: metadata.name.clone(),
                    file_type: metadata.file_type,
                    page_count: metadata.page_count,
                    tags: metadata.tags,
                    created_at: metadata.created_at,
                };

                // Mark file as completed
                state
                    .import_tracker
                    .mark_completed(&path_str, metadata.id)
                    .await;

                // Emit document-added event for UI updates
                let _ = app.emit(
                    "document-added",
                    DocumentAddedEvent {
                        collection_id: collection_id.clone(),
                        document: doc_info,
                    },
                );
            }
            Err(e) => {
                tracing::error!("Import failed for {:?}: {}", path, e);
                state
                    .import_tracker
                    .mark_failed(&path_str, e.to_string())
                    .await;
            }
        }

        // Drop the guards before emitting
        drop(embedder_guard);
        drop(model_id_guard);
        drop(storage);

        // Emit progress event
        let progress = state.import_tracker.get_progress(&collection_id).await;
        let _ = app.emit("import-progress", &progress);
    }

    // Log before cleanup
    let progress = state.import_tracker.get_progress(&collection_id).await;
    tracing::info!(
        "Import complete for {}: {} successful, {} failed",
        collection_id,
        progress.completed,
        progress.failed
    );

    // Cleanup finished files
    state
        .import_tracker
        .cleanup_collection(&collection_id)
        .await;
}

/// Get import progress for all collections
#[tauri::command]
pub async fn get_import_progress(state: State<'_, AppState>) -> CommandResult<Vec<ImportProgress>> {
    Ok(state.import_tracker.get_all_progress().await)
}

/// Get all documents in a collection
#[tauri::command]
pub async fn get_documents(
    collection_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Vec<DocumentInfo>> {
    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

    let storage = state.storage.read().await;

    let documents = storage.list_documents(namespace_id).await.storage_err()?;

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
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<DocumentInfo> {
    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

    let storage = state.storage.read().await;

    let document = storage
        .get_document(namespace_id, &document_id)
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
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

    let storage = state.storage.read().await;

    // Fetch text directly from files/{id}/text entry
    let text_bytes = storage
        .get_document_text(namespace_id, &document_id)
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
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Vec<String>> {
    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

    // Get current embedding model ID
    let model_id_guard = state.embedding_model_id.read().await;
    let model_id = model_id_guard
        .as_ref()
        .ok_or(CommandError::embedder_not_configured())?;

    // Fetch stored embeddings
    let storage = state.storage.read().await;
    let embeddings = storage
        .get_embeddings(namespace_id, &document_id, model_id)
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
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(
        "Deleting document {} from collection {}",
        document_id,
        collection_id
    );

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

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

/// Delete a collection and all its documents
#[tauri::command]
pub async fn delete_collection(
    collection_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!("Deleting collection {}", collection_id);

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

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
    collection_id: String,
    writable: bool,
    state: State<'_, AppState>,
) -> CommandResult<String> {
    tracing::info!(
        "Sharing collection {} (writable: {})",
        collection_id,
        writable
    );

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| CommandError::invalid_collection_id())?;

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

// ============================================================================
// Conversation Commands
// ============================================================================

use crate::core::{agent, conversations};
use tokio_util::sync::CancellationToken;

/// List all saved conversations
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> CommandResult<Vec<conversations::ConversationSummary>> {
    conversations::list_conversations(&state.config.conversations_dir).storage_err()
}

/// Load a conversation by ID
#[tauri::command]
pub async fn load_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<agent::Conversation> {
    let path = conversations::conversation_path(&state.config.conversations_dir, &conversation_id);
    let conversation = conversations::load_conversation(&path).storage_err()?;

    // Add to in-memory cache
    state
        .conversations
        .write()
        .await
        .insert(conversation_id, conversation.clone());

    Ok(conversation)
}

/// Start a new chat conversation
///
/// Requires a chat provider to be configured first.
/// Optionally accepts collections to scope the conversation context.
#[tauri::command]
pub async fn start_chat(
    collections: Option<Vec<agent::CollectionInfo>>,
    state: State<'_, AppState>,
) -> CommandResult<agent::Conversation> {
    // Verify provider is configured
    let provider_guard = state.chat_provider.read().await;
    if provider_guard.is_none() {
        return Err(CommandError::provider_not_configured());
    }
    drop(provider_guard);

    // Enrich collection info with document counts and total pages
    let enriched_collections = match collections {
        Some(cols) => {
            let storage = state.storage.read().await;
            let mut enriched = Vec::with_capacity(cols.len());
            for col in cols {
                let namespace_id: NamespaceId = match col.id.parse() {
                    Ok(id) => id,
                    Err(_) => {
                        // Keep original if ID is invalid
                        enriched.push(col);
                        continue;
                    }
                };

                // Get documents to calculate total pages
                let documents = storage
                    .list_documents(namespace_id)
                    .await
                    .unwrap_or_default();
                let document_count = documents.len();
                let total_pages: usize = documents.iter().map(|d| d.page_count).sum();

                enriched.push(agent::CollectionInfo {
                    id: col.id,
                    name: col.name,
                    document_count,
                    total_pages,
                    created_at: None,
                });
            }
            Some(enriched)
        }
        None => None,
    };

    // Create new conversation with optional collection context
    let conversation_id = uuid::Uuid::new_v4().to_string();
    let conversation = agent::Conversation::with_collection_context(
        conversation_id.clone(),
        enriched_collections.as_deref(),
    );

    // Save to disk
    conversations::save_conversation(&state.config.conversations_dir, &conversation)
        .storage_err()?;

    state
        .conversations
        .write()
        .await
        .insert(conversation_id.clone(), conversation.clone());

    tracing::info!("Started new chat conversation: {}", conversation_id);
    Ok(conversation)
}

/// Send a message to a conversation and stream the response
///
/// Optionally accepts collections to filter agent searches.
#[tauri::command]
pub async fn send_message(
    conversation_id: String,
    message: String,
    collections: Option<Vec<agent::CollectionInfo>>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(
        "Sending message to conversation {}: {}",
        conversation_id,
        message
    );

    // Get conversation
    let mut conversations = state.conversations.write().await;
    let conversation = conversations
        .get_mut(&conversation_id)
        .ok_or(CommandError::conversation_not_found())?
        .clone();
    drop(conversations);

    // Verify provider is configured
    let provider_guard = state.chat_provider.read().await;
    if provider_guard.is_none() {
        return Err(CommandError::provider_not_configured());
    }
    drop(provider_guard);

    // Create cancellation token
    let cancel_token = CancellationToken::new();
    state
        .active_generations
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    // Create event channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<agent::AgentEvent>(100);

    // Spawn event forwarder to Tauri
    let app_handle = app.clone();
    let conv_id = conversation_id.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let event_name = format!("agent-event-{}", conv_id);
            if let Err(e) = app_handle.emit(&event_name, &event) {
                tracing::error!("Failed to emit agent event: {}", e);
            }
        }
    });

    // Clone state for the agent loop
    let conversations_dir = state.config.conversations_dir.clone();
    let state_clone = state.inner().clone();

    let conv_id = conversation_id.clone();
    let mut conversation = conversation;

    // Clone collection context for the spawned task
    let collections_clone = collections;

    // Run agent loop in background
    tokio::spawn(async move {
        // Create agent context with collection filtering
        let ctx = agent::AgentContext {
            state: state_clone.clone(),
            collections: collections_clone,
        };

        // Get provider reference for the agent loop
        let provider_guard = state_clone.chat_provider.read().await;
        let provider = match provider_guard.as_ref() {
            Some(p) => p.as_ref(),
            None => {
                tracing::error!("Provider not configured when running agent loop");
                return;
            }
        };

        if let Err(e) =
            agent::run_agent_loop(provider, &mut conversation, message, &ctx, tx, cancel_token)
                .await
        {
            tracing::error!(
                conversation_id = %conv_id,
                error = %e,
                error_chain = ?e,
                "Agent loop error"
            );
        }
        drop(provider_guard);

        // Generate title on first user message
        let user_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == agent::MessageRole::User)
            .count();
        if user_count == 1 {
            conversation.generate_title();
        }

        // Save to disk
        if let Err(e) = conversations::save_conversation(&conversations_dir, &conversation) {
            tracing::error!("Failed to save conversation: {}", e);
        }

        // Update conversation in state
        state_clone
            .conversations
            .write()
            .await
            .insert(conv_id, conversation);
    });

    Ok(())
}

/// Cancel an in-progress generation
#[tauri::command]
pub async fn cancel_generation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let generations = state.active_generations.read().await;
    if let Some(token) = generations.get(&conversation_id) {
        token.cancel();
        tracing::info!("Cancelled generation for conversation {}", conversation_id);
    }
    Ok(())
}

// ============================================================================
// Model Management Commands (Unified)
// ============================================================================

use crate::core::models;
use crate::core::ModelType;

/// Model info for frontend (unified across types)
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_gb: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<usize>,
}

impl From<models::LanguageModelInfo> for ModelInfo {
    fn from(m: models::LanguageModelInfo) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            size_gb: m.size_gb,
            dimensions: None,
        }
    }
}

impl From<models::EmbeddingModelInfo> for ModelInfo {
    fn from(m: models::EmbeddingModelInfo) -> Self {
        Self {
            id: m.id,
            name: m.name,
            description: m.description,
            size_gb: m.size_gb,
            dimensions: Some(m.dimensions),
        }
    }
}

/// Get list of available models for a type
#[tauri::command]
pub async fn get_available_models(model_type: ModelType) -> CommandResult<Vec<ModelInfo>> {
    Ok(match model_type {
        ModelType::Language => models::available_language_models()
            .into_iter()
            .map(ModelInfo::from)
            .collect(),
        ModelType::Embedding => models::available_embedding_models()
            .into_iter()
            .map(ModelInfo::from)
            .collect(),
    })
}

/// Model download status
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum DownloadStatus {
    NotDownloaded,
    Ready,
}

/// Get download status for a model
#[tauri::command]
pub async fn get_model_status(
    model_type: ModelType,
    model_id: String,
    state: State<'_, AppState>,
) -> CommandResult<DownloadStatus> {
    let is_downloaded = match model_type {
        ModelType::Language => {
            let model = models::get_language_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_manager.is_downloaded(&model)
        }
        ModelType::Embedding => {
            let model = models::get_embedding_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_manager.is_downloaded(&model)
        }
    };

    Ok(if is_downloaded {
        DownloadStatus::Ready
    } else {
        DownloadStatus::NotDownloaded
    })
}

/// Download a model with progress events
#[tauri::command]
pub async fn download_model(
    model_type: ModelType,
    model_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{ModelDownloadProgress, ModelStatus};

    // Check if already downloaded and get model for download
    let is_downloaded = match model_type {
        ModelType::Language => {
            let model = models::get_language_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_manager.is_downloaded(&model)
        }
        ModelType::Embedding => {
            let model = models::get_embedding_model(&model_id)
                .ok_or(CommandError::model_not_found(&model_id))?;
            state.model_manager.is_downloaded(&model)
        }
    };

    if is_downloaded {
        tracing::info!("Model {} is already downloaded", model_id);
        return Ok(());
    }

    // Create channels for status and progress
    let (status_tx, mut status_rx) = tokio::sync::mpsc::channel::<ModelStatus>(10);
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<ModelDownloadProgress>(100);

    // Forward events to frontend
    let app_status = app.clone();
    tokio::spawn(async move {
        while let Some(status) = status_rx.recv().await {
            let _ = app_status.emit("model-status-changed", &status);
        }
    });

    let app_progress = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = progress_rx.recv().await {
            let _ = app_progress.emit("model-download-progress", &progress);
        }
    });

    // Download based on type
    match model_type {
        ModelType::Language => {
            let model = models::get_language_model(&model_id).unwrap();
            state
                .model_manager
                .download(&model, model_type, status_tx, progress_tx)
                .await
                .external_err()?;
        }
        ModelType::Embedding => {
            let model = models::get_embedding_model(&model_id).unwrap();
            state
                .model_manager
                .download(&model, model_type, status_tx, progress_tx)
                .await
                .external_err()?;
        }
    }

    Ok(())
}

/// Embedding model status for frontend sync
#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingStatus {
    pub ready: bool,
    pub error: Option<String>,
    pub model_id: Option<String>,
}

/// Get the current embedding model status
///
/// Used by the frontend to sync state on HMR reload or initial load.
#[tauri::command]
pub async fn get_embedding_status(state: State<'_, AppState>) -> CommandResult<EmbeddingStatus> {
    let embedder_guard = state.embedder.read().await;
    let model_id_guard = state.embedding_model_id.read().await;

    Ok(EmbeddingStatus {
        ready: embedder_guard.is_some(),
        error: None, // Errors are transient during load, not persisted
        model_id: model_id_guard.clone(),
    })
}

/// Get the currently configured model ID for a type
#[tauri::command]
pub async fn get_current_model(
    model_type: ModelType,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    Ok(match model_type {
        ModelType::Language => {
            let provider_config = state.provider_config.read().await;
            provider_config.as_ref().map(|c| c.model_id().to_string())
        }
        ModelType::Embedding => {
            let model_id = state.embedding_model_id.read().await;
            model_id.clone()
        }
    })
}

/// Configure and load a model
#[tauri::command]
pub async fn configure_model(
    model_type: ModelType,
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    match model_type {
        ModelType::Language => configure_language_model_impl(model_id, state).await,
        ModelType::Embedding => configure_embedding_model_impl(model_id, state).await,
    }
}

async fn configure_language_model_impl(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{LocalProvider, ProviderConfig, Settings};

    if let Some(ref id) = model_id {
        let model = models::get_language_model(id).ok_or(CommandError::model_not_found(id))?;

        tracing::info!(
            "Configuring local language model: {} ({})",
            model.name,
            model.id
        );

        let model_path = state
            .model_manager
            .get_path(&model)
            .ok_or(CommandError::model_not_downloaded(id))?;

        let provider = LocalProvider::load(&model_path, &model)
            .await
            .map_err(|e| CommandError::internal(format!("Failed to load model: {}", e)))?;

        let provider_config = ProviderConfig::Local {
            model_id: id.clone(),
        };
        *state.chat_provider.write().await = Some(Box::new(provider));
        *state.provider_config.write().await = Some(provider_config.clone());

        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = Some(provider_config);
        settings.save(&state.config.settings_file).storage_err()?;
    } else {
        tracing::info!("Unloading chat provider");
        *state.chat_provider.write().await = None;
        *state.provider_config.write().await = None;

        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = None;
        settings.save(&state.config.settings_file).storage_err()?;
    }

    Ok(())
}

async fn configure_embedding_model_impl(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::{Embedder, Settings};

    if let Some(ref id) = model_id {
        let model = models::get_embedding_model(id).ok_or(CommandError::model_not_found(id))?;

        tracing::info!(
            "Configuring embedding model: {} ({})",
            model.name,
            model.hf_repo_id
        );

        let embedder = Embedder::new(&model.hf_repo_id, model.dimensions)
            .await
            .map_err(|e| CommandError::internal(format!("Failed to load embedder: {}", e)))?;

        {
            let index = &*state.search;
            let indexer_config = milli::update::IndexerConfig::default();
            search::configure_embedder(index, &indexer_config, "default", model.dimensions)
                .map_err(|e| {
                    CommandError::internal(format!("Failed to configure embedder in index: {}", e))
                })?;
        }

        *state.embedder.write().await = Some(embedder);
        *state.embedding_model_id.write().await = Some(id.clone());
    } else {
        tracing::info!("Disabling embedding model");

        {
            let index = &*state.search;
            let indexer_config = milli::update::IndexerConfig::default();
            if let Err(e) = search::remove_embedder(index, &indexer_config) {
                tracing::warn!("Failed to remove embedder from index: {}", e);
            }
        }

        *state.embedder.write().await = None;
        *state.embedding_model_id.write().await = None;
    }

    let mut settings = Settings::load(&state.config.settings_file);
    settings.embedding_model_id = model_id;
    settings.save(&state.config.settings_file).storage_err()?;

    Ok(())
}

// ============================================================================
// Provider Management Commands
// ============================================================================

use crate::core::{
    get_provider_families as core_get_provider_families, AnthropicProvider, OpenAIProvider,
    ProviderConfig, ProviderFamily, RemoteModelInfo,
};

/// Get available provider families
#[tauri::command]
pub async fn get_provider_families() -> Vec<ProviderFamily> {
    core_get_provider_families()
}

/// Get current provider configuration
#[tauri::command]
pub async fn get_current_provider(
    state: State<'_, AppState>,
) -> CommandResult<Option<ProviderConfig>> {
    let config = state.provider_config.read().await;
    Ok(config.clone())
}

/// Fetch available models from OpenAI API
#[tauri::command]
pub async fn fetch_openai_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    OpenAIProvider::fetch_models(&api_key).await.external_err()
}

/// Fetch available models for Anthropic (verifies API key)
#[tauri::command]
pub async fn fetch_anthropic_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    AnthropicProvider::verify_api_key(&api_key)
        .await
        .external_err()
}

/// Configure OpenAI as the chat provider
#[tauri::command]
pub async fn configure_openai_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::Settings;

    tracing::info!("Configuring OpenAI provider with model: {}", model);

    let provider = OpenAIProvider::new(&api_key, &model);
    let config = ProviderConfig::OpenAI {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    // Update state
    *state.chat_provider.write().await = Some(Box::new(provider));
    *state.provider_config.write().await = Some(config.clone());

    // Persist setting and store API key separately for easy switching
    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = Some(config);
    settings.openai_api_key = Some(api_key);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!("OpenAI provider configured successfully");
    Ok(())
}

/// Configure Anthropic as the chat provider
#[tauri::command]
pub async fn configure_anthropic_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::Settings;

    tracing::info!("Configuring Anthropic provider with model: {}", model);

    let provider = AnthropicProvider::new(&api_key, &model);
    let config = ProviderConfig::Anthropic {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    // Update state
    *state.chat_provider.write().await = Some(Box::new(provider));
    *state.provider_config.write().await = Some(config.clone());

    // Persist setting and store API key separately for easy switching
    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = Some(config);
    settings.anthropic_api_key = Some(api_key);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!("Anthropic provider configured successfully");
    Ok(())
}

/// Get stored API keys (for auto-populating when switching providers)
#[tauri::command]
pub async fn get_stored_api_keys(state: State<'_, AppState>) -> CommandResult<StoredApiKeys> {
    use crate::core::Settings;

    let settings = Settings::load(&state.config.settings_file);
    Ok(StoredApiKeys {
        openai: settings.openai_api_key,
        anthropic: settings.anthropic_api_key,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredApiKeys {
    pub openai: Option<String>,
    pub anthropic: Option<String>,
}

// ============================================================================
// Prediction Commands (Tab Completion)
// ============================================================================

const PREDICTION_PROMPT: &str = r#"Based on the conversation above, predict what the user is most likely to ask or say next.

Rules:
- Output ONLY the predicted message, nothing else
- Keep it concise (1-2 sentences max)
- Make it a natural follow-up question or statement
- If the assistant just answered a question, predict a likely follow-up
- If unsure, output nothing"#;

/// Predict what the user might say next in a conversation
#[tauri::command]
pub async fn predict_next_message(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    // Get conversation
    let conversations = state.conversations.read().await;
    let conversation = match conversations.get(&conversation_id) {
        Some(c) => c.clone(),
        None => return Ok(None),
    };
    drop(conversations);

    // Don't predict if conversation is too short (need at least user + assistant message)
    let non_system_messages = conversation
        .messages
        .iter()
        .filter(|m| m.role != agent::MessageRole::System)
        .count();
    if non_system_messages < 2 {
        return Ok(None);
    }

    // Check if there's already a prediction in progress, cancel it
    {
        let predictions = state.active_predictions.read().await;
        if let Some(token) = predictions.get(&conversation_id) {
            token.cancel();
        }
    }

    // Create new cancellation token
    let cancel_token = CancellationToken::new();
    state
        .active_predictions
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    // Get provider
    let provider_guard = state.chat_provider.read().await;
    let provider = match provider_guard.as_ref() {
        Some(p) => p.as_ref(),
        None => return Ok(None),
    };

    // Build prediction messages: conversation + prediction prompt
    let mut prediction_messages = conversation.messages.clone();
    prediction_messages.push(agent::Message {
        role: agent::MessageRole::User,
        content: vec![agent::ContentBlock::Text {
            text: PREDICTION_PROMPT.to_string(),
        }],
    });

    // Create channel for streaming (we'll collect the result)
    let (tx, mut rx) = tokio::sync::mpsc::channel::<agent::ProviderEvent>(50);

    // Spawn task to collect result
    let collect_task = tokio::spawn(async move {
        let mut result = String::new();
        while let Some(event) = rx.recv().await {
            if let agent::ProviderEvent::TextDelta(text) = event {
                result.push_str(&text);
            }
        }
        result
    });

    // Run completion with timeout
    let completion_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        provider.stream_completion(&prediction_messages, &[], tx, cancel_token.clone()),
    )
    .await;

    // Clean up
    state
        .active_predictions
        .write()
        .await
        .remove(&conversation_id);

    // Get collected text
    let collected_text = collect_task.await.unwrap_or_default();

    match completion_result {
        Ok(Ok(_)) => {
            let prediction = collected_text.trim().to_string();
            // Truncate if too long
            let prediction = if prediction.len() > 150 {
                format!("{}...", &prediction[..147])
            } else {
                prediction
            };
            if prediction.is_empty() {
                Ok(None)
            } else {
                Ok(Some(prediction))
            }
        }
        Ok(Err(e)) => {
            tracing::debug!("Prediction failed: {}", e);
            Ok(None)
        }
        Err(_) => {
            // Timeout
            tracing::debug!("Prediction timed out");
            Ok(None)
        }
    }
}

/// Cancel any pending prediction for a conversation
#[tauri::command]
pub async fn cancel_prediction(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let predictions = state.active_predictions.read().await;
    if let Some(token) = predictions.get(&conversation_id) {
        token.cancel();
    }
    Ok(())
}
