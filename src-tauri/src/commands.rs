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

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub document: DocumentInfo,
    pub collection_id: String,
    pub score: f32,
    pub snippet: String,
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

/// Import a PDF file into a collection
#[tauri::command]
pub async fn import_pdf(
    path: String,
    collection_id: String,
    state: State<'_, AppState>,
) -> Result<DocumentInfo, String> {
    tracing::info!("Importing PDF: {} into collection {}", path, collection_id);

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| "Invalid collection ID")?;

    let path_ref = std::path::Path::new(&path);
    let file_name = path_ref
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.pdf")
        .to_string();

    // Extract text from PDF
    let extracted = pdf::extract_text(path_ref).map_err(|e| e.to_string())?;

    let doc_id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    // Store PDF and text blobs, get hashes from iroh
    let (pdf_hash, text_hash) = {
        let mut storage_guard = state.storage.write().await;
        let storage = storage_guard
            .as_mut()
            .ok_or_else(|| "Storage not initialized".to_string())?;

        // Store PDF bytes as blob - iroh returns the BLAKE3 hash
        let pdf_hash = storage
            .store_blob(&extracted.pdf_bytes)
            .await
            .map_err(|e| e.to_string())?;

        // Store extracted text as blob
        let text_hash = storage
            .store_blob(extracted.text.as_bytes())
            .await
            .map_err(|e| e.to_string())?;

        // Create document metadata and store in collection
        let metadata = DocumentMetadata {
            id: doc_id.clone(),
            name: file_name.clone(),
            pdf_hash: pdf_hash.to_string(),
            text_hash: text_hash.to_string(),
            page_count: extracted.page_count,
            tags: vec![],
            created_at: created_at.clone(),
        };

        storage
            .add_document(namespace_id, metadata)
            .await
            .map_err(|e| e.to_string())?;

        (pdf_hash.to_string(), text_hash.to_string())
    };

    // Index in milli search (acquire mutex to serialize indexing operations)
    let search_guard = state.search.read().await;
    if let Some(index) = search_guard.as_ref() {
        let indexer_config = state.indexer_config.lock().await;
        search::index_document(
            index,
            &indexer_config,
            &doc_id,
            &file_name,
            &extracted.text,
            &collection_id,
        )
        .map_err(|e| e.to_string())?;
        tracing::info!("Indexed document {} in milli", doc_id);
    } else {
        tracing::warn!("Search not initialized, document not indexed");
    }

    tracing::info!(
        "Imported {} ({} pages, {} chars)",
        file_name,
        extracted.page_count,
        extracted.text.len()
    );

    Ok(DocumentInfo {
        id: doc_id,
        name: file_name,
        pdf_hash,
        text_hash,
        page_count: extracted.page_count,
        tags: vec![],
        created_at,
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
#[tauri::command]
pub async fn import_pdfs_batch(
    paths: Vec<String>,
    collection_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<BatchImportResult, String> {
    tracing::info!(
        "Batch importing {} PDFs into collection {}",
        paths.len(),
        collection_id
    );

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| "Invalid collection ID")?;

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

    let mut successful = Vec::new();
    let mut failed = Vec::new();
    let mut docs_to_index = Vec::new();

    // Process each extraction as it completes: store → emit → collect for indexing
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

        // Store blobs and metadata
        let mut storage_guard = state.storage.write().await;
        let storage = match storage_guard.as_mut() {
            Some(s) => s,
            None => {
                failed.push(BatchImportError {
                    path: extraction.file_name,
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
                    path: extraction.file_name,
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
                    path: extraction.file_name,
                    error: format!("Failed to store text: {}", e),
                });
                continue;
            }
        };

        let metadata = DocumentMetadata {
            id: doc_id.clone(),
            name: extraction.file_name.clone(),
            pdf_hash: pdf_hash.clone(),
            text_hash: text_hash.clone(),
            page_count: extraction.extracted.page_count,
            tags: vec![],
            created_at: created_at.clone(),
        };

        if let Err(e) = storage.add_document(namespace_id, metadata).await {
            failed.push(BatchImportError {
                path: extraction.file_name,
                error: format!("Failed to add metadata: {}", e),
            });
            continue;
        }

        // Release lock before emitting event
        drop(storage_guard);

        let doc_info = DocumentInfo {
            id: doc_id.clone(),
            name: extraction.file_name.clone(),
            pdf_hash,
            text_hash,
            page_count: extraction.extracted.page_count,
            tags: vec![],
            created_at,
        };

        // Emit event immediately
        let _ = app.emit(
            "document-added",
            DocumentAddedEvent {
                collection_id: collection_id.clone(),
                document: doc_info.clone(),
            },
        );

        docs_to_index.push(search::DocToIndex {
            id: doc_id,
            name: extraction.file_name,
            content: extraction.extracted.text,
            collection_id: collection_id.clone(),
        });

        successful.push(doc_info);
    }

    tracing::info!(
        "Processed {} PDFs successfully, {} failed",
        successful.len(),
        failed.len()
    );

    // Batch index all documents
    if !docs_to_index.is_empty() {
        let search_guard = state.search.read().await;
        if let Some(index) = search_guard.as_ref() {
            let indexer_config = state.indexer_config.lock().await;
            let doc_count = docs_to_index.len();
            search::index_documents_batch(index, &indexer_config, docs_to_index)
                .map_err(|e| e.to_string())?;
            tracing::info!("Batch indexed {} documents in milli", doc_count);
        } else {
            tracing::warn!("Search not initialized, documents not indexed");
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
    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| "Invalid collection ID")?;

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

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| "Invalid collection ID")?;

    let mut storage_guard = state.storage.write().await;
    let storage = storage_guard
        .as_mut()
        .ok_or_else(|| "Storage not initialized".to_string())?;

    storage
        .delete_document(namespace_id, &document_id)
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Delete a collection and all its documents
#[tauri::command]
pub async fn delete_collection(
    collection_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!("Deleting collection {}", collection_id);

    let namespace_id: NamespaceId = collection_id
        .parse()
        .map_err(|_| "Invalid collection ID")?;

    let mut storage_guard = state.storage.write().await;
    let storage = storage_guard
        .as_mut()
        .ok_or_else(|| "Storage not initialized".to_string())?;

    storage
        .delete_collection(namespace_id)
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Search documents
#[tauri::command]
pub async fn search(
    query: String,
    limit: Option<usize>,
    collection_ids: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    tracing::info!(
        "Searching for: {} (collections: {:?})",
        query,
        collection_ids
    );

    let search_guard = state.search.read().await;
    let index = search_guard
        .as_ref()
        .ok_or_else(|| "Search not initialized".to_string())?;

    let limit = limit.unwrap_or(20);
    let results =
        search::search_index(index, &query, limit, collection_ids.as_deref())
            .map_err(|e| e.to_string())?;

    let mut search_results = Vec::new();
    for hit in results {
        let id = search::get_document_field(index, hit.doc_id, "id")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let name = search::get_document_field(index, hit.doc_id, "name")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let content = search::get_document_field(index, hit.doc_id, "content")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();
        let collection_id = search::get_document_field(index, hit.doc_id, "collection_id")
            .map_err(|e| e.to_string())?
            .unwrap_or_default();

        let snippet = content.chars().take(200).collect::<String>();
        let score =
            milli::score_details::ScoreDetails::global_score(hit.scores.iter()) as f32;

        search_results.push(SearchResult {
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

    Ok(search_results)
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
