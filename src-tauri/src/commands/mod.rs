use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::core::{search, AppState};
use iroh_docs::NamespaceId;

/// Document metadata returned to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentInfo {
    pub id: String,
    pub name: String,
    pub pdf_hash: String,
    pub text_hash: String,
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

/// Single search hit (chunk-level result)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Unique chunk ID (e.g., "doc123_chunk_5")
    pub chunk_id: String,
    /// Parent document info
    pub document: DocumentInfo,
    pub collection_id: String,
    pub score: f32,
    /// Chunk text content
    pub snippet: String,
}

/// Paginated search response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub total_hits: usize,
    pub page: usize,
    pub page_size: usize,
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

    // Start watching the new collection for document events
    {
        let mut coordinator_guard = state.job_coordinator.write().await;
        if let Some(coordinator) = coordinator_guard.as_mut() {
            coordinator.watch_namespace(namespace_id);
        }
    }

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

/// Import multiple PDF files into a collection using the job pipeline.
///
/// Files flow through: extraction → storage → (iroh event) → embedding → indexing
/// Progress and completion events are emitted to the frontend.
#[tauri::command]
pub async fn import_pdfs_batch<R: tauri::Runtime>(
    paths: Vec<String>,
    collection_id: String,
    app: AppHandle<R>,
    state: State<'_, AppState>,
) -> Result<BatchImportResult, String> {
    tracing::info!(
        "Batch importing {} PDFs into collection {}",
        paths.len(),
        collection_id
    );

    // Validate collection ID
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Get the job coordinator
    let mut coordinator_guard = state.job_coordinator.write().await;
    let coordinator = coordinator_guard
        .as_mut()
        .ok_or("Job coordinator not initialized")?;

    // Ensure the collection is being watched for indexing events
    if !coordinator.is_watching(&namespace_id) {
        coordinator.watch_namespace(namespace_id);
    }

    // Convert paths to PathBuf
    let path_bufs: Vec<std::path::PathBuf> = paths.iter().map(std::path::PathBuf::from).collect();
    let total = path_bufs.len();

    // Submit all paths for import
    coordinator.import(path_bufs, collection_id.clone()).await;

    let mut successful = Vec::new();
    let mut failed = Vec::new();

    // Release coordinator lock - we'll poll non-blocking in the loop
    drop(coordinator_guard);

    // Collect results until all documents are processed
    while successful.len() + failed.len() < total {
        // Small delay to avoid busy-waiting
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Collect completions and failures while holding coordinator lock
        let (completed_batch, failed_batch) = {
            let mut coordinator_guard = state.job_coordinator.write().await;
            let coordinator = match coordinator_guard.as_mut() {
                Some(c) => c,
                None => break,
            };

            let mut completed = Vec::new();
            let mut failures = Vec::new();

            while let Some(c) = coordinator.try_recv_completed() {
                completed.push(c);
            }
            while let Some(f) = coordinator.try_recv_failed() {
                failures.push(f);
            }

            (completed, failures)
        };
        // Coordinator lock released here

        // Process completions with storage lock (acquired once for entire batch)
        if !completed_batch.is_empty() {
            let storage = state.storage.read().await;
            for completed in completed_batch {
                match storage.get_document(namespace_id, &completed.doc_id).await {
                    Ok(Some(metadata)) => {
                        let doc_info = DocumentInfo {
                            id: metadata.id,
                            name: metadata.name.clone(),
                            pdf_hash: metadata.pdf_hash,
                            text_hash: metadata.text_hash,
                            page_count: metadata.page_count,
                            tags: metadata.tags,
                            created_at: metadata.created_at,
                        };

                        // Emit event to frontend
                        let _ = app.emit(
                            "document-added",
                            DocumentAddedEvent {
                                collection_id: collection_id.clone(),
                                document: doc_info.clone(),
                            },
                        );

                        successful.push(doc_info);
                    }
                    Ok(None) => {
                        tracing::warn!(
                            "Document {} completed but not found in storage",
                            completed.doc_id
                        );
                        failed.push(BatchImportError {
                            path: completed.doc_id,
                            error: "Document not found after indexing".to_string(),
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to fetch document metadata: {}", e);
                        failed.push(BatchImportError {
                            path: completed.doc_id,
                            error: format!("Failed to fetch metadata: {}", e),
                        });
                    }
                }
            }
        }

        // Process failures
        for doc_failed in failed_batch {
            failed.push(BatchImportError {
                path: doc_failed.path,
                error: doc_failed.error,
            });
        }
    }

    tracing::info!(
        "Batch import complete: {} successful, {} failed",
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
            pdf_hash: m.pdf_hash,
            text_hash: m.text_hash,
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
        pdf_hash: document.pdf_hash,
        text_hash: document.text_hash,
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

    // Get document metadata to find text hash
    let document = storage
        .get_document(namespace_id, &document_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Document not found".to_string())?;

    // Parse the text hash and fetch the blob
    let text_hash: iroh_blobs::Hash = document
        .text_hash
        .parse()
        .map_err(|_| "Invalid text hash".to_string())?;

    let text_bytes = storage
        .get_blob(&text_hash)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Text content not found".to_string())?;

    String::from_utf8(text_bytes).map_err(|e| format!("Invalid UTF-8 in text content: {}", e))
}

/// Get the text chunks for a document (computed on-the-fly, not stored)
#[tauri::command]
pub async fn get_document_chunks(
    collection_id: String,
    document_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Get document text
    let storage = state.storage.read().await;

    let document = storage
        .get_document(namespace_id, &document_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Document not found".to_string())?;

    let text_hash: iroh_blobs::Hash = document
        .text_hash
        .parse()
        .map_err(|_| "Invalid text hash".to_string())?;

    let text_bytes = storage
        .get_blob(&text_hash)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Text content not found".to_string())?;

    let text = String::from_utf8(text_bytes)
        .map_err(|e| format!("Invalid UTF-8 in text content: {}", e))?;

    // Drop storage lock before accessing embedder
    drop(storage);

    // Get chunks from embedder
    let embedder_guard = state.embedder.read().await;
    let embedder = embedder_guard
        .as_ref()
        .ok_or_else(|| "Embedding model not loaded".to_string())?;

    Ok(embedder.chunk_text(&text))
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
    let search = state.search.clone();
    let indexer_config = state.indexer_config.clone();
    tokio::spawn(async move {
        let index = search.read().await;
        let indexer_config = indexer_config.lock().await;
        match search::delete_document_chunks(&index, &indexer_config, &document_id) {
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
    let search = state.search.clone();
    let indexer_config = state.indexer_config.clone();
    tokio::spawn(async move {
        let index = search.read().await;
        let indexer_config = indexer_config.lock().await;
        match search::delete_chunks_by_collection(&index, &indexer_config, &collection_id) {
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

/// Search documents
///
/// When `semantic_ratio > 0` and an embedding model is configured, performs hybrid search
/// combining keyword (BM25) and vector similarity.
#[tauri::command]
pub async fn search(
    query: String,
    page: Option<usize>,
    page_size: Option<usize>,
    collection_ids: Option<Vec<String>>,
    semantic_ratio: Option<f32>,
    min_score: Option<f32>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    let semantic_ratio = semantic_ratio.unwrap_or(0.0).clamp(0.0, 1.0);
    let min_score = min_score.map(|s| s.clamp(0.0, 1.0));

    tracing::info!(
        "Searching for: {} (collections: {:?}, semantic_ratio: {})",
        query,
        collection_ids,
        semantic_ratio
    );

    let index = state.search.read().await;

    let page = page.unwrap_or(0);
    let page_size = page_size.unwrap_or(20);
    let offset = page * page_size;

    let doc_count = search::get_document_count(&index).unwrap_or(0);
    tracing::info!(
        "Search params: page={}, page_size={}, offset={}, index_docs={}",
        page,
        page_size,
        offset,
        doc_count
    );

    // Generate query embedding if semantic search is requested
    let query_vector = if semantic_ratio > 0.0 {
        let embedder_guard = state.embedder.read().await;
        let embedder = embedder_guard
            .as_ref()
            .ok_or("Semantic search requires a configured embedding model")?;
        Some(embedder.embed(&query).await.map_err(|e| e.to_string())?)
    } else {
        None
    };

    let results = search::search_index(
        &index,
        search::SearchParams {
            query: &query,
            limit: page_size,
            offset,
            collection_ids: collection_ids.as_deref(),
            query_vector,
            semantic_ratio,
            min_score,
        },
    )
    .map_err(|e| e.to_string())?;

    tracing::info!(
        "Search returned: {} hits, total_hits={}",
        results.hits.len(),
        results.total_hits
    );

    let rtxn = index.read_txn().map_err(|e| e.to_string())?;

    let mut hits = Vec::new();
    for (i, hit) in results.hits.into_iter().enumerate() {
        let doc = search::get_document(&index, &rtxn, hit.doc_id)
            .map_err(|e| e.to_string())?
            .unwrap_or_default();

        let get_str = |key: &str| -> String {
            doc.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };

        let id = get_str("parent_id");
        let name = get_str("parent_name");
        let content = get_str("content");
        let collection_id = get_str("collection_id");

        // Content is now chunk text - use it as the snippet
        let snippet = content.chars().take(500).collect::<String>();
        let score = milli::score_details::ScoreDetails::global_score(hit.scores.iter()) as f32;

        // Unique chunk ID from parent + global position
        let chunk_id = format!("{}_{}", id, offset + i);

        hits.push(SearchHit {
            chunk_id,
            document: DocumentInfo {
                id,
                name,
                pdf_hash: String::new(),
                text_hash: String::new(),
                page_count: 0,
                tags: vec![],
                created_at: String::new(),
            },
            collection_id,
            score,
            snippet,
        });
    }

    Ok(SearchResponse {
        hits,
        total_hits: results.total_hits,
        page,
        page_size,
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
/// Requires a language model to be loaded first via `configure_language_model`.
#[tauri::command]
pub async fn start_chat(state: State<'_, AppState>) -> Result<agent::Conversation, String> {
    // Verify model is loaded
    let model_guard = state.agent_model.read().await;
    if model_guard.is_none() {
        return Err("No language model loaded".to_string());
    }
    drop(model_guard);

    // Create new conversation
    let conversation_id = uuid::Uuid::new_v4().to_string();
    let conversation = agent::Conversation::new(conversation_id.clone());

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
#[tauri::command]
pub async fn send_message(
    conversation_id: String,
    message: String,
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

    // Get model
    let model_guard = state.agent_model.read().await;
    let model = model_guard
        .as_ref()
        .ok_or("Model not loaded")?
        .model()
        .clone();
    drop(model_guard);

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

    // Clone state fields for the agent loop
    let config = state.config.clone();
    let storage = state.storage.clone();
    let search = state.search.clone();
    let indexer_config = state.indexer_config.clone();
    let embedder = state.embedder.clone();
    let embedding_model_id = state.embedding_model_id.clone();
    let agent_model = state.agent_model.clone();
    let language_model_id = state.language_model_id.clone();
    let conversations_arc = state.conversations.clone();
    let active_generations = state.active_generations.clone();
    let job_coordinator = state.job_coordinator.clone();
    let conversations_dir = state.config.conversations_dir.clone();

    let conv_id = conversation_id.clone();
    let mut conversation = conversation;

    // Run agent loop in background
    tokio::spawn(async move {
        let state_clone = crate::core::AppState {
            config,
            storage,
            search,
            indexer_config,
            embedder,
            embedding_model_id,
            agent_model,
            language_model_id,
            conversations: conversations_arc.clone(),
            active_generations,
            job_coordinator,
        };

        if let Err(e) = agent::run_agent_loop(
            &model,
            &mut conversation,
            message,
            &state_clone,
            tx,
            cancel_token,
        )
        .await
        {
            tracing::error!(
                conversation_id = %conv_id,
                error = %e,
                error_chain = ?e,
                "Agent loop error"
            );
        }

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
        conversations_arc
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

/// Unload the model to free memory
#[tauri::command]
pub async fn unload_model(state: State<'_, AppState>) -> Result<(), String> {
    let mut model_guard = state.agent_model.write().await;
    *model_guard = None;
    tracing::info!("LLM model unloaded");
    Ok(())
}

// ============================================================================
// Model Management Commands
// ============================================================================

use crate::core::models::{
    self, DownloadProgress, EmbeddingModelInfo, LanguageModelInfo, ModelManager, ModelStatus,
};

#[cfg(test)]
mod tests;

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
    let model_id = state.language_model_id.read().await;
    Ok(model_id.clone())
}

/// Configure and load a language model
#[tauri::command]
pub async fn configure_language_model(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    use crate::core::Settings;

    if let Some(ref id) = model_id {
        // Verify the model exists
        let model = models::get_language_model(id)
            .ok_or_else(|| format!("Language model not found: {}", id))?;

        tracing::info!("Configuring language model: {} ({})", model.name, model.id);

        // Get the model manager and path
        let manager = ModelManager::new()
            .await
            .map_err(|e| format!("Failed to create model manager: {}", e))?;

        let model_path = manager
            .get_path(&model)
            .ok_or_else(|| format!("Model not downloaded: {}", id))?;

        // Load the model
        let agent_model = agent::AgentModel::load(&model_path, &model)
            .await
            .map_err(|e| format!("Failed to load model: {}", e))?;

        // Update state
        *state.agent_model.write().await = Some(agent_model);
        *state.language_model_id.write().await = Some(id.clone());

        tracing::info!("Language model configured: {}", id);
    } else {
        // Unload the model
        tracing::info!("Unloading language model");

        *state.agent_model.write().await = None;
        *state.language_model_id.write().await = None;
    }

    // Persist setting
    let mut settings = Settings::load(&state.config.settings_file);
    settings.language_model_id = model_id;
    settings
        .save(&state.config.settings_file)
        .map_err(|e| e.to_string())?;

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
            let index = state.search.read().await;
            let indexer_config = state.indexer_config.lock().await;
            search::configure_embedder(&index, &indexer_config, "default", model.dimensions)
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
            let index = state.search.read().await;
            let indexer_config = state.indexer_config.lock().await;
            if let Err(e) = search::remove_embedder(&index, &indexer_config) {
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
