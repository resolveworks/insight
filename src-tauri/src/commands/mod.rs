use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::core::{import_and_index_pdf, search, AppState};
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

/// Collection metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    pub document_count: usize,
    pub created_at: String,
}

/// Get all collections
#[tauri::command]
pub async fn get_collections(state: State<'_, AppState>) -> Result<Vec<CollectionInfo>, String> {
    let storage = state.storage.read().await;

    let collections = storage
        .list_collections()
        .await
        .map_err(|e| e.to_string())?;

    // Build CollectionInfo for each collection
    let mut result = Vec::with_capacity(collections.len());
    for (namespace_id, metadata) in collections {
        let document_count = storage.count_documents(namespace_id).await.unwrap_or(0);
        result.push(CollectionInfo {
            id: namespace_id.to_string(),
            name: metadata.name,
            document_count,
            created_at: metadata.created_at,
        });
    }

    Ok(result)
}

/// Create a new collection
#[tauri::command]
pub async fn create_collection(
    name: String,
    state: State<'_, AppState>,
) -> Result<CollectionInfo, String> {
    tracing::info!("Creating collection: {}", name);

    let storage = state.storage.read().await;

    let (namespace_id, metadata) = storage
        .create_collection(&name)
        .await
        .map_err(|e| e.to_string())?;

    drop(storage);

    // Start watching the new collection for sync events (documents from peers)
    state.watch_namespace(namespace_id).await;

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count: 0,
        created_at: metadata.created_at,
    })
}

/// Result of batch import
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchImportResult {
    pub successful: Vec<DocumentInfo>,
    pub failed: Vec<BatchImportError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchImportError {
    pub path: String,
    pub error: String,
}

/// Event payload for document added
#[derive(Debug, Clone, Serialize)]
pub struct DocumentAddedEvent {
    pub collection_id: String,
    pub document: DocumentInfo,
}

/// Import multiple PDF files into a collection.
///
/// Each file is imported and indexed directly (no event-based processing).
/// Returns when all documents are fully indexed and searchable.
#[tauri::command]
pub async fn import_pdfs_batch<R: tauri::Runtime>(
    paths: Vec<String>,
    collection_id: String,
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<BatchImportResult, String> {
    tracing::info!(
        "Importing {} PDFs into collection {}",
        paths.len(),
        collection_id
    );

    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Get embedder and model ID (required for import)
    let embedder_guard = state.embedder.read().await;
    let model_id_guard = state.embedding_model_id.read().await;

    let (embedder, model_id) = match (&*embedder_guard, &*model_id_guard) {
        (Some(e), Some(m)) => (e, m.clone()),
        _ => {
            return Err(
                "Embedder not configured. Please configure an embedding model first.".into(),
            )
        }
    };

    let mut successful = Vec::new();
    let mut failed = Vec::new();

    // Import and index each PDF directly
    for path_str in &paths {
        let path = std::path::Path::new(path_str);
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
                    id: metadata.id,
                    name: metadata.name.clone(),
                    file_type: metadata.file_type,
                    page_count: metadata.page_count,
                    tags: metadata.tags,
                    created_at: metadata.created_at,
                };

                // Emit event for frontend
                let _ = app.emit(
                    "document-added",
                    DocumentAddedEvent {
                        collection_id: collection_id.clone(),
                        document: doc_info.clone(),
                    },
                );

                successful.push(doc_info);
            }
            Err(e) => {
                tracing::error!("Import failed for {:?}: {}", path, e);
                failed.push(BatchImportError {
                    path: path_str.clone(),
                    error: e.to_string(),
                });
            }
        }
    }

    tracing::info!(
        "Import complete: {} successful, {} failed",
        successful.len(),
        failed.len()
    );

    Ok(BatchImportResult { successful, failed })
}

/// Get all documents in a collection
#[tauri::command]
pub async fn get_documents(
    collection_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentInfo>, String> {
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    let storage = state.storage.read().await;

    let documents = storage
        .list_documents(namespace_id)
        .await
        .map_err(|e| e.to_string())?;

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
) -> Result<DocumentInfo, String> {
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    let storage = state.storage.read().await;

    let document = storage
        .get_document(namespace_id, &document_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Document not found".to_string())?;

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
) -> Result<String, String> {
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    let storage = state.storage.read().await;

    // Fetch text directly from files/{id}/text entry
    let text_bytes = storage
        .get_document_text(namespace_id, &document_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Text content not found".to_string())?;

    String::from_utf8(text_bytes).map_err(|e| format!("Invalid UTF-8 in text content: {}", e))
}

/// Get the text chunks for a document (read from stored embeddings)
#[tauri::command]
pub async fn get_document_chunks(
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Get current embedding model ID
    let model_id_guard = state.embedding_model_id.read().await;
    let model_id = model_id_guard
        .as_ref()
        .ok_or_else(|| "No embedding model configured".to_string())?;

    // Fetch stored embeddings
    let storage = state.storage.read().await;
    let embeddings = storage
        .get_embeddings(namespace_id, &document_id, model_id)
        .await
        .map_err(|e| e.to_string())?;

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
) -> Result<(), String> {
    tracing::info!(
        "Deleting document {} from collection {}",
        document_id,
        collection_id
    );

    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Delete from storage first
    {
        let storage = state.storage.read().await;
        storage
            .delete_document(namespace_id, &document_id)
            .await
            .map_err(|e| e.to_string())?;
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
) -> Result<(), String> {
    tracing::info!("Deleting collection {}", collection_id);

    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Delete from storage first
    {
        let storage = state.storage.read().await;
        storage
            .delete_collection(namespace_id)
            .await
            .map_err(|e| e.to_string())?;
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
) -> Result<String, String> {
    tracing::info!(
        "Sharing collection {} (writable: {})",
        collection_id,
        writable
    );

    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    let storage = state.storage.read().await;

    storage
        .share_collection(namespace_id, writable)
        .await
        .map_err(|e| e.to_string())
}

/// Import a collection from a share ticket
///
/// The ticket string is obtained from someone who called `share_collection`.
/// After import, the collection will sync with the original peer.
#[tauri::command]
pub async fn import_collection(
    ticket: String,
    state: State<'_, AppState>,
) -> Result<CollectionInfo, String> {
    tracing::info!("Importing collection from ticket");

    let namespace_id = {
        let storage = state.storage.read().await;
        storage
            .import_collection(&ticket)
            .await
            .map_err(|e| e.to_string())?
    };

    // Start watching the imported collection for sync events
    state.watch_namespace(namespace_id).await;

    // Fetch collection info
    let storage = state.storage.read().await;

    let metadata = storage
        .get_collection_metadata(namespace_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or("Collection metadata not found after import")?;

    let document_count = storage.count_documents(namespace_id).await.unwrap_or(0);

    Ok(CollectionInfo {
        id: namespace_id.to_string(),
        name: metadata.name,
        document_count,
        created_at: metadata.created_at,
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
) -> Result<Vec<conversations::ConversationSummary>, String> {
    conversations::list_conversations(&state.config.conversations_dir).map_err(|e| e.to_string())
}

/// Load a conversation by ID
#[tauri::command]
pub async fn load_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> Result<agent::Conversation, String> {
    let path = conversations::conversation_path(&state.config.conversations_dir, &conversation_id);
    let conversation = conversations::load_conversation(&path).map_err(|e| e.to_string())?;

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
) -> Result<agent::Conversation, String> {
    // Verify provider is configured
    let provider_guard = state.chat_provider.read().await;
    if provider_guard.is_none() {
        return Err("No chat provider configured".to_string());
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
        .map_err(|e| e.to_string())?;

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
) -> Result<(), String> {
    tracing::info!(
        "Sending message to conversation {}: {}",
        conversation_id,
        message
    );

    // Get conversation
    let mut conversations = state.conversations.write().await;
    let conversation = conversations
        .get_mut(&conversation_id)
        .ok_or("Conversation not found")?
        .clone();
    drop(conversations);

    // Verify provider is configured
    let provider_guard = state.chat_provider.read().await;
    if provider_guard.is_none() {
        return Err("No chat provider configured".to_string());
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
) -> Result<(), String> {
    let generations = state.active_generations.read().await;
    if let Some(token) = generations.get(&conversation_id) {
        token.cancel();
        tracing::info!("Cancelled generation for conversation {}", conversation_id);
    }
    Ok(())
}

// ============================================================================
// Model Management Commands
// ============================================================================

use crate::core::models::{
    self, DownloadProgress, EmbeddingModelInfo, LanguageModelInfo, ModelManager, ModelStatus,
};

/// Get list of available language models
#[tauri::command]
pub async fn get_available_language_models() -> Result<Vec<LanguageModelInfo>, String> {
    Ok(models::available_language_models())
}

/// Get download status for a language model
#[tauri::command]
pub async fn get_language_model_status(model_id: Option<String>) -> Result<ModelStatus, String> {
    let manager = ModelManager::new().await.map_err(|e| e.to_string())?;

    // Use specified model or default
    let model = if let Some(id) = model_id {
        models::get_language_model(&id).ok_or_else(|| format!("Model not found: {}", id))?
    } else {
        models::default_language_model()
    };

    if manager.is_downloaded(&model) {
        let path = manager.get_path(&model).ok_or("Model path not found")?;
        Ok(ModelStatus::Ready { path })
    } else {
        Ok(ModelStatus::NotDownloaded)
    }
}

/// Download a language model with progress events
#[tauri::command]
pub async fn download_language_model(model_id: String, app: AppHandle) -> Result<(), String> {
    let manager = ModelManager::new().await.map_err(|e| e.to_string())?;

    let model = models::get_language_model(&model_id)
        .ok_or_else(|| format!("Model not found: {}", model_id))?;

    // Check if already downloaded
    if manager.is_downloaded(&model) {
        tracing::info!("Model {} is already downloaded", model.id);
        return Ok(());
    }

    tracing::info!(
        "Starting download of model: {} ({})",
        model.name,
        model.gguf_file
    );

    // Create progress channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DownloadProgress>(100);

    // Spawn event forwarder
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Err(e) = app_handle.emit("language-model-download-progress", &progress) {
                tracing::error!("Failed to emit download progress: {}", e);
            }
        }
    });

    // Download with progress
    manager
        .download(&model, tx)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    tracing::info!("Model download complete: {}", model.id);

    // Emit completion event
    let _ = app.emit("language-model-download-complete", &model);

    Ok(())
}

/// Get the currently configured language model ID
#[tauri::command]
pub async fn get_current_language_model(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let provider_config = state.provider_config.read().await;
    Ok(provider_config.as_ref().map(|c| c.model_id().to_string()))
}

/// Configure and load a local language model
#[tauri::command]
pub async fn configure_language_model(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::core::{LocalProvider, ProviderConfig, Settings};

    if let Some(ref id) = model_id {
        // Verify the model exists
        let model = models::get_language_model(id)
            .ok_or_else(|| format!("Language model not found: {}", id))?;

        tracing::info!(
            "Configuring local language model: {} ({})",
            model.name,
            model.id
        );

        // Get the model manager and path
        let manager = ModelManager::new()
            .await
            .map_err(|e| format!("Failed to create model manager: {}", e))?;

        let model_path = manager
            .get_path(&model)
            .ok_or_else(|| format!("Model not downloaded: {}", id))?;

        // Load the local provider
        let provider = LocalProvider::load(&model_path, &model)
            .await
            .map_err(|e| format!("Failed to load model: {}", e))?;

        // Update state
        let provider_config = ProviderConfig::Local {
            model_id: id.clone(),
        };
        *state.chat_provider.write().await = Some(Box::new(provider));
        *state.provider_config.write().await = Some(provider_config.clone());

        tracing::info!("Local language model configured: {}", id);

        // Persist setting
        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = Some(provider_config);
        settings
            .save(&state.config.settings_file)
            .map_err(|e| e.to_string())?;
    } else {
        // Unload the provider
        tracing::info!("Unloading chat provider");

        *state.chat_provider.write().await = None;
        *state.provider_config.write().await = None;

        // Clear setting
        let mut settings = Settings::load(&state.config.settings_file);
        settings.provider = None;
        settings
            .save(&state.config.settings_file)
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

// ============================================================================
// Embedding Model Commands
// ============================================================================

/// Get list of available embedding models
#[tauri::command]
pub async fn get_available_embedding_models() -> Result<Vec<EmbeddingModelInfo>, String> {
    Ok(models::available_embedding_models())
}

/// Get the currently configured embedding model ID
#[tauri::command]
pub async fn get_current_embedding_model(
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    let model_id = state.embedding_model_id.read().await;
    Ok(model_id.clone())
}

/// Embedding model status response
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum EmbeddingModelStatus {
    NotDownloaded,
    Ready,
}

/// Get download status for an embedding model
#[tauri::command]
pub async fn get_embedding_model_status(model_id: String) -> Result<EmbeddingModelStatus, String> {
    let manager = ModelManager::new().await.map_err(|e| e.to_string())?;

    let model = models::get_embedding_model(&model_id)
        .ok_or_else(|| format!("Embedding model not found: {}", model_id))?;

    if manager.is_downloaded(&model) {
        Ok(EmbeddingModelStatus::Ready)
    } else {
        Ok(EmbeddingModelStatus::NotDownloaded)
    }
}

/// Download an embedding model with progress events
#[tauri::command]
pub async fn download_embedding_model(model_id: String, app: AppHandle) -> Result<(), String> {
    let manager = ModelManager::new().await.map_err(|e| e.to_string())?;

    let model = models::get_embedding_model(&model_id)
        .ok_or_else(|| format!("Embedding model not found: {}", model_id))?;

    // Check if already downloaded
    if manager.is_downloaded(&model) {
        tracing::info!("Embedding model {} is already downloaded", model.id);
        return Ok(());
    }

    tracing::info!(
        "Starting download of embedding model: {} ({})",
        model.name,
        model.hf_repo_id
    );

    // Create progress channel
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DownloadProgress>(100);

    // Spawn event forwarder
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(progress) = rx.recv().await {
            if let Err(e) = app_handle.emit("embedding-model-download-progress", &progress) {
                tracing::error!("Failed to emit embedding download progress: {}", e);
            }
        }
    });

    // Download with progress
    manager
        .download(&model, tx)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    tracing::info!("Embedding model download complete: {}", model.id);

    // Emit completion event
    let _ = app.emit("embedding-model-download-complete", &model);

    Ok(())
}

/// Configure and load an embedding model for semantic search
#[tauri::command]
pub async fn configure_embedding_model(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::core::{Embedder, Settings};

    if let Some(ref id) = model_id {
        // Verify the model exists
        let model = models::get_embedding_model(id)
            .ok_or_else(|| format!("Embedding model not found: {}", id))?;

        tracing::info!(
            "Configuring embedding model: {} ({})",
            model.name,
            model.hf_repo_id
        );

        // Load embedder using mistralrs
        let embedder = Embedder::new(&model.hf_repo_id, model.dimensions)
            .await
            .map_err(|e| format!("Failed to load embedder: {}", e))?;

        // Configure milli index for vector search
        {
            let index = &*state.search;
            let indexer_config = milli::update::IndexerConfig::default();
            search::configure_embedder(index, &indexer_config, "default", model.dimensions)
                .map_err(|e| format!("Failed to configure embedder in index: {}", e))?;
        }

        // Update state
        *state.embedder.write().await = Some(embedder);
        *state.embedding_model_id.write().await = Some(id.clone());

        tracing::info!("Embedding model configured: {}", id);
    } else {
        // Disable embeddings
        tracing::info!("Disabling embedding model");

        // Remove embedder configuration from milli index
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

    // Persist setting
    let mut settings = Settings::load(&state.config.settings_file);
    settings.embedding_model_id = model_id;
    settings
        .save(&state.config.settings_file)
        .map_err(|e| e.to_string())?;

    Ok(())
}

// ============================================================================
// Boot Status Command
// ============================================================================

/// Boot status response for frontend
#[derive(Debug, Clone, Serialize)]
pub struct BootStatus {
    pub embedding_configured: bool,
    pub embedding_model_id: Option<String>,
    pub embedding_downloaded: bool,
}

/// Get boot status - called by frontend when ready to receive state
#[tauri::command]
pub async fn get_boot_status(state: State<'_, AppState>) -> Result<BootStatus, String> {
    let settings = crate::core::Settings::load(&state.config.settings_file);
    let (embedding_configured, embedding_downloaded) =
        crate::core::check_embedding_status(&settings).await;

    Ok(BootStatus {
        embedding_configured,
        embedding_model_id: settings.embedding_model_id,
        embedding_downloaded,
    })
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
) -> Result<Option<ProviderConfig>, String> {
    let config = state.provider_config.read().await;
    Ok(config.clone())
}

/// Fetch available models from OpenAI API
#[tauri::command]
pub async fn fetch_openai_models(api_key: String) -> Result<Vec<RemoteModelInfo>, String> {
    OpenAIProvider::fetch_models(&api_key)
        .await
        .map_err(|e| e.to_string())
}

/// Fetch available models for Anthropic (verifies API key)
#[tauri::command]
pub async fn fetch_anthropic_models(api_key: String) -> Result<Vec<RemoteModelInfo>, String> {
    AnthropicProvider::verify_api_key(&api_key)
        .await
        .map_err(|e| e.to_string())
}

/// Configure OpenAI as the chat provider
#[tauri::command]
pub async fn configure_openai_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    settings
        .save(&state.config.settings_file)
        .map_err(|e| e.to_string())?;

    tracing::info!("OpenAI provider configured successfully");
    Ok(())
}

/// Configure Anthropic as the chat provider
#[tauri::command]
pub async fn configure_anthropic_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    settings
        .save(&state.config.settings_file)
        .map_err(|e| e.to_string())?;

    tracing::info!("Anthropic provider configured successfully");
    Ok(())
}

/// Get stored API keys (for auto-populating when switching providers)
#[tauri::command]
pub async fn get_stored_api_keys(state: State<'_, AppState>) -> Result<StoredApiKeys, String> {
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
) -> Result<Option<String>, String> {
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
) -> Result<(), String> {
    let predictions = state.active_predictions.read().await;
    if let Some(token) = predictions.get(&conversation_id) {
        token.cancel();
    }
    Ok(())
}
