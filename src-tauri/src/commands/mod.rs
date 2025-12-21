use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::core::storage::DocumentMetadata;
use crate::core::{pdf, search, AppState};
use iroh_docs::NamespaceId;

/// Max concurrent PDF extractions to avoid stack overflow from too many parallel tasks
const MAX_CONCURRENT_EXTRACTIONS: usize = 8;

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

/// Single search hit
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub document: DocumentInfo,
    pub collection_id: String,
    pub score: f32,
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
    let mut storage_guard = state.storage.write().await;
    let storage = match storage_guard.as_mut() {
        Some(s) => s,
        None => return Ok(vec![]),
    };

    let collections = storage
        .list_collections()
        .await
        .map_err(|e| e.to_string())?;

    // Build CollectionInfo for each collection
    let mut result = Vec::with_capacity(collections.len());
    for (namespace_id, metadata) in collections {
        let document_count = storage.count_documents(namespace_id).unwrap_or(0);
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

    let mut storage_guard = state.storage.write().await;
    let storage = match storage_guard.as_mut() {
        Some(s) => s,
        None => return Err("Storage not initialized".to_string()),
    };

    let (namespace_id, metadata) = storage
        .create_collection(&name)
        .await
        .map_err(|e| e.to_string())?;

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

/// Extraction result passed from blocking task
struct ExtractionResult {
    file_name: String,
    extracted: pdf::ExtractedDocument,
}

/// Import multiple PDF files into a collection with parallel processing
///
/// Each file is fully processed (extract → store → index) before being marked successful.
/// This ensures no orphaned documents in storage without search index entries.
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

    let namespace_id: NamespaceId = collection_id.parse().map_err(|_| "Invalid collection ID")?;

    // Create extraction stream with bounded concurrency
    let mut extraction_stream = stream::iter(paths)
        .map(|path| async move {
            tokio::task::spawn_blocking({
                let path = path.clone();
                move || {
                    let path_ref = std::path::Path::new(&path);
                    let file_name = path_ref
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown.pdf")
                        .to_string();

                    match pdf::extract_text(path_ref) {
                        Ok(extracted) => Ok(ExtractionResult {
                            file_name,
                            extracted,
                        }),
                        Err(e) => Err((path, e.to_string())),
                    }
                }
            })
            .await
        })
        .buffer_unordered(MAX_CONCURRENT_EXTRACTIONS);

    const INDEX_BATCH_SIZE: usize = 50;

    let mut successful = Vec::new();
    let mut failed = Vec::new();
    let mut pending_index: Vec<(DocumentInfo, search::DocToIndex)> = Vec::new();

    // Macro to flush pending documents to index
    macro_rules! flush_pending {
        () => {
            if !pending_index.is_empty() {
                let (doc_infos, docs_to_index): (Vec<_>, Vec<_>) =
                    std::mem::take(&mut pending_index).into_iter().unzip();

                let search_guard = state.search.read().await;
                if let Some(index) = search_guard.as_ref() {
                    let indexer_config = state.indexer_config.lock().await;

                    match search::index_documents_batch(index, &indexer_config, docs_to_index) {
                        Ok(()) => {
                            for doc_info in doc_infos {
                                let _ = app.emit(
                                    "document-added",
                                    DocumentAddedEvent {
                                        collection_id: collection_id.clone(),
                                        document: doc_info.clone(),
                                    },
                                );
                                successful.push(doc_info);
                            }
                        }
                        Err(e) => {
                            tracing::error!(
                                "Failed to batch index {} documents: {}",
                                doc_infos.len(),
                                e
                            );
                            for doc_info in doc_infos {
                                failed.push(BatchImportError {
                                    path: doc_info.name,
                                    error: format!("Stored but failed to index: {}", e),
                                });
                            }
                        }
                    }
                } else {
                    tracing::warn!(
                        "Search not initialized, {} documents not indexed",
                        doc_infos.len()
                    );
                    for doc_info in doc_infos {
                        let _ = app.emit(
                            "document-added",
                            DocumentAddedEvent {
                                collection_id: collection_id.clone(),
                                document: doc_info.clone(),
                            },
                        );
                        successful.push(doc_info);
                    }
                }
            }
        };
    }

    // Process each extraction as it completes: store blobs and metadata
    // Indexing is batched every INDEX_BATCH_SIZE documents
    while let Some(result) = extraction_stream.next().await {
        let extraction = match result {
            Ok(Ok(e)) => e,
            Ok(Err((path, error))) => {
                failed.push(BatchImportError { path, error });
                continue;
            }
            Err(e) => {
                failed.push(BatchImportError {
                    path: "unknown".to_string(),
                    error: format!("Task join error: {}", e),
                });
                continue;
            }
        };

        let file_name = extraction.file_name.clone();
        let text_content = extraction.extracted.text.clone();

        // Store blobs and metadata
        let mut storage_guard = state.storage.write().await;
        let storage = match storage_guard.as_mut() {
            Some(s) => s,
            None => {
                failed.push(BatchImportError {
                    path: file_name,
                    error: "Storage not initialized".to_string(),
                });
                continue;
            }
        };

        let doc_id = uuid::Uuid::new_v4().to_string();
        let created_at = chrono::Utc::now().to_rfc3339();

        let pdf_hash = match storage.store_blob(&extraction.extracted.pdf_bytes).await {
            Ok(h) => h.to_string(),
            Err(e) => {
                failed.push(BatchImportError {
                    path: file_name,
                    error: format!("Failed to store PDF: {}", e),
                });
                continue;
            }
        };

        let text_hash = match storage
            .store_blob(extraction.extracted.text.as_bytes())
            .await
        {
            Ok(h) => h.to_string(),
            Err(e) => {
                failed.push(BatchImportError {
                    path: file_name,
                    error: format!("Failed to store text: {}", e),
                });
                continue;
            }
        };

        let metadata = DocumentMetadata {
            id: doc_id.clone(),
            name: file_name.clone(),
            pdf_hash: pdf_hash.clone(),
            text_hash: text_hash.clone(),
            page_count: extraction.extracted.page_count,
            tags: vec![],
            created_at: created_at.clone(),
        };

        if let Err(e) = storage.add_document(namespace_id, metadata).await {
            failed.push(BatchImportError {
                path: file_name,
                error: format!("Failed to add metadata: {}", e),
            });
            continue;
        }

        // Release storage lock
        drop(storage_guard);

        let doc_info = DocumentInfo {
            id: doc_id.clone(),
            name: file_name.clone(),
            pdf_hash,
            text_hash,
            page_count: extraction.extracted.page_count,
            tags: vec![],
            created_at,
        };

        // Generate embeddings if embedder is available
        let vectors = {
            let embedder_guard = state.embedder.read().await;
            if let Some(ref embedder) = *embedder_guard {
                match embedder.embed_document(&text_content).await {
                    Ok(vecs) => Some(vecs),
                    Err(e) => {
                        tracing::warn!("Failed to generate embeddings for {}: {}", file_name, e);
                        None
                    }
                }
            } else {
                None
            }
        };

        let doc_to_index = search::DocToIndex {
            id: doc_id,
            name: file_name,
            content: text_content,
            collection_id: collection_id.clone(),
            vectors,
        };

        pending_index.push((doc_info, doc_to_index));

        // Flush batch when it reaches INDEX_BATCH_SIZE
        if pending_index.len() >= INDEX_BATCH_SIZE {
            flush_pending!();
        }
    }

    // Flush any remaining documents
    flush_pending!();

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

    let mut storage_guard = state.storage.write().await;
    let storage = storage_guard
        .as_mut()
        .ok_or_else(|| "Storage not initialized".to_string())?;

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

    let mut storage_guard = state.storage.write().await;
    let storage = storage_guard
        .as_mut()
        .ok_or_else(|| "Storage not initialized".to_string())?;

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

    let mut storage_guard = state.storage.write().await;
    let storage = storage_guard
        .as_mut()
        .ok_or_else(|| "Storage not initialized".to_string())?;

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
        let mut storage_guard = state.storage.write().await;
        let storage = storage_guard
            .as_mut()
            .ok_or_else(|| "Storage not initialized".to_string())?;

        storage
            .delete_document(namespace_id, &document_id)
            .map_err(|e| e.to_string())?;
    }

    // Delete from search index in background
    let search = state.search.clone();
    let indexer_config = state.indexer_config.clone();
    tokio::spawn(async move {
        let search_guard = search.read().await;
        if let Some(index) = search_guard.as_ref() {
            let indexer_config = indexer_config.lock().await;
            if let Err(e) = search::delete_document(index, &indexer_config, &document_id) {
                tracing::error!(
                    "Failed to remove document {} from search index: {}",
                    document_id,
                    e
                );
            } else {
                tracing::info!("Removed document {} from search index", document_id);
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
        let mut storage_guard = state.storage.write().await;
        let storage = storage_guard
            .as_mut()
            .ok_or_else(|| "Storage not initialized".to_string())?;

        storage
            .delete_collection(namespace_id)
            .map_err(|e| e.to_string())?;
    }

    // Delete from search index in background
    let search = state.search.clone();
    let indexer_config = state.indexer_config.clone();
    tokio::spawn(async move {
        let search_guard = search.read().await;
        if let Some(index) = search_guard.as_ref() {
            let indexer_config = indexer_config.lock().await;
            match search::delete_documents_by_collection(index, &indexer_config, &collection_id) {
                Ok(deleted_count) => {
                    tracing::info!(
                        "Removed {} documents from search index for collection {}",
                        deleted_count,
                        collection_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to remove documents from search index for collection {}: {}",
                        collection_id,
                        e
                    );
                }
            }
        }
    });

    Ok(())
}

/// Search documents
#[tauri::command]
pub async fn search(
    query: String,
    page: Option<usize>,
    page_size: Option<usize>,
    collection_ids: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<SearchResponse, String> {
    tracing::info!(
        "Searching for: {} (collections: {:?})",
        query,
        collection_ids
    );

    let search_guard = state.search.read().await;
    let index = search_guard
        .as_ref()
        .ok_or_else(|| "Search not initialized".to_string())?;

    let page = page.unwrap_or(0);
    let page_size = page_size.unwrap_or(20);
    let offset = page * page_size;

    let doc_count = search::get_document_count(index).unwrap_or(0);
    tracing::info!(
        "Search params: page={}, page_size={}, offset={}, index_docs={}",
        page,
        page_size,
        offset,
        doc_count
    );

    let results = search::search_index(index, &query, page_size, offset, collection_ids.as_deref())
        .map_err(|e| e.to_string())?;

    tracing::info!(
        "Search returned: {} hits, total_hits={}",
        results.hits.len(),
        results.total_hits
    );

    let mut hits = Vec::new();
    for hit in results.hits {
        let id = search::get_document_field_by_internal_id(index, hit.doc_id, "id")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let name = search::get_document_field_by_internal_id(index, hit.doc_id, "name")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let content = search::get_document_field_by_internal_id(index, hit.doc_id, "content")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let collection_id =
            search::get_document_field_by_internal_id(index, hit.doc_id, "collection_id")
                .map_err(|e| e.to_string())?
                .unwrap_or_default();

        let snippet = content.chars().take(200).collect::<String>();
        let score = milli::score_details::ScoreDetails::global_score(hit.scores.iter()) as f32;

        hits.push(SearchHit {
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

/// Get the local node ID for P2P connections
#[tauri::command]
pub async fn get_node_id(state: State<'_, AppState>) -> Result<String, String> {
    let storage = state.storage.read().await;
    if storage.is_none() {
        return Ok("not-initialized".to_string());
    }

    // TODO: Get node ID from iroh endpoint
    Ok("initializing...".to_string())
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
#[tauri::command]
pub async fn start_chat(
    model_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<agent::Conversation, String> {
    // Get model info
    let model_info = if let Some(id) = model_id {
        models::get_model(&id).ok_or_else(|| format!("Model not found: {}", id))?
    } else {
        models::default_model()
    };

    // Ensure model is loaded
    let mut model_guard = state.agent_model.write().await;
    if model_guard.is_none() {
        // Get the path to the downloaded model
        let manager = ModelManager::new()
            .await
            .map_err(|e| format!("Failed to create model manager: {}", e))?;

        let model_path = manager
            .get_model_path(&model_info)
            .ok_or_else(|| format!("Model not downloaded: {}", model_info.id))?;

        tracing::info!(
            "Loading LLM model: {} from {:?}",
            model_info.name,
            model_path
        );
        let model = agent::AgentModel::load(&model_path, &model_info)
            .await
            .map_err(|e| format!("Failed to load model: {}", e))?;
        *model_guard = Some(model);
        tracing::info!("LLM model loaded: {}", model_info.name);
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
    let conversations_arc = state.conversations.clone();
    let active_generations = state.active_generations.clone();
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
            conversations: conversations_arc.clone(),
            active_generations,
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

use crate::core::models::{self, DownloadProgress, EmbeddingModelInfo, ModelInfo, ModelManager, ModelStatus};

#[cfg(test)]
mod tests;

/// Get list of available models
#[tauri::command]
pub async fn get_available_models() -> Result<Vec<ModelInfo>, String> {
    Ok(models::available_models())
}

/// Get download status for a specific model
#[tauri::command]
pub async fn get_model_status(
    model_id: Option<String>,
    _state: State<'_, AppState>,
) -> Result<ModelStatus, String> {
    let manager = ModelManager::new()
        .await
        .map_err(|e| e.to_string())?;

    // Use specified model or default
    let model = if let Some(id) = model_id {
        models::get_model(&id).ok_or_else(|| format!("Model not found: {}", id))?
    } else {
        models::default_model()
    };

    if manager.is_downloaded(&model) {
        let path = manager
            .get_model_path(&model)
            .ok_or("Model path not found")?;
        Ok(ModelStatus::Ready { path })
    } else {
        Ok(ModelStatus::NotDownloaded)
    }
}

/// Download a specific model with progress events
#[tauri::command]
pub async fn download_model(
    model_id: String,
    app: AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = ModelManager::new()
        .await
        .map_err(|e| e.to_string())?;

    let model =
        models::get_model(&model_id).ok_or_else(|| format!("Model not found: {}", model_id))?;

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
            if let Err(e) = app_handle.emit("model-download-progress", &progress) {
                tracing::error!("Failed to emit download progress: {}", e);
            }
        }
    });

    // Download with progress
    manager
        .download_model(&model, tx)
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    tracing::info!("Model download complete: {}", model.id);

    // Emit completion event
    let _ = app.emit("model-download-complete", &model);

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
pub async fn get_embedding_model_status(
    model_id: String,
    _state: State<'_, AppState>,
) -> Result<EmbeddingModelStatus, String> {
    let manager = ModelManager::new()
        .await
        .map_err(|e| e.to_string())?;

    let model = models::get_embedding_model(&model_id)
        .ok_or_else(|| format!("Embedding model not found: {}", model_id))?;

    if manager.is_embedding_model_downloaded(&model) {
        Ok(EmbeddingModelStatus::Ready)
    } else {
        Ok(EmbeddingModelStatus::NotDownloaded)
    }
}

/// Download an embedding model with progress events
#[tauri::command]
pub async fn download_embedding_model(
    model_id: String,
    app: AppHandle,
    _state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = ModelManager::new()
        .await
        .map_err(|e| e.to_string())?;

    let model = models::get_embedding_model(&model_id)
        .ok_or_else(|| format!("Embedding model not found: {}", model_id))?;

    // Check if already downloaded
    if manager.is_embedding_model_downloaded(&model) {
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
        .download_embedding_model(&model, tx)
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

        // Update state
        *state.embedder.write().await = Some(embedder);
        *state.embedding_model_id.write().await = Some(id.clone());

        tracing::info!("Embedding model configured: {}", id);
    } else {
        // Disable embeddings
        tracing::info!("Disabling embedding model");
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
