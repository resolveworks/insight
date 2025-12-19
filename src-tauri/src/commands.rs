use bumpalo::Bump;
use milli::documents::mmap_from_objects;
use milli::progress::Progress;
use milli::update::new::indexer::{self, DocumentOperation};
use milli::update::IndexerConfig;
use milli::vector::RuntimeEmbedders;
use milli::Index;
use serde::{Deserialize, Serialize};
use serde_json::Map;
use tauri::State;

use crate::core::storage::DocumentMetadata;
use crate::core::{pdf, AppState};
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

/// Search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub document: DocumentInfo,
    pub score: f32,
    pub snippet: String,
}

/// Index a document in milli
fn index_document(index: &Index, doc_id: &str, name: &str, content: &str) -> Result<(), String> {
    let indexer_config = IndexerConfig::default();

    // Create JSON document as Object (Map<String, Value>)
    let mut doc = Map::new();
    doc.insert("id".to_string(), serde_json::Value::String(doc_id.to_string()));
    doc.insert("name".to_string(), serde_json::Value::String(name.to_string()));
    doc.insert("content".to_string(), serde_json::Value::String(content.to_string()));

    // Use milli's mmap_from_objects helper
    let mmap = mmap_from_objects([doc]);

    // Get current fields map
    let rtxn = index.read_txn().map_err(|e| e.to_string())?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn).map_err(|e| e.to_string())?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    // No embedders configured yet
    let embedders = RuntimeEmbedders::default();

    // Create document operation
    let mut operation = DocumentOperation::new();
    operation.replace_documents(&mmap).map_err(|e| e.to_string())?;

    let indexer_alloc = Bump::new();
    let (document_changes, operation_stats, primary_key) = operation
        .into_changes(
            &indexer_alloc,
            index,
            &rtxn,
            None,
            &mut new_fields_ids_map,
            &|| false,
            Progress::default(),
            None,
        )
        .map_err(|e| e.to_string())?;

    // Check for errors in operation stats
    if let Some(error) = operation_stats.into_iter().find_map(|stat| stat.error) {
        return Err(format!("Document operation error: {}", error));
    }

    // Write transaction for indexing (rtxn stays alive as document_changes borrows from it)
    let mut wtxn = index.write_txn().map_err(|e| e.to_string())?;

    indexer_config
        .thread_pool
        .install(|| {
            indexer::index(
                &mut wtxn,
                index,
                &milli::ThreadPoolNoAbortBuilder::new().build().unwrap(),
                indexer_config.grenad_parameters(),
                &db_fields_ids_map,
                new_fields_ids_map,
                primary_key,
                &document_changes,
                embedders,
                &|| false,
                &Progress::default(),
                &Default::default(),
            )
        })
        .map_err(|e| format!("Thread pool error: {}", e))?
        .map_err(|e| e.to_string())?;

    wtxn.commit().map_err(|e| e.to_string())?;

    Ok(())
}

/// Search milli index
fn search_index(
    index: &Index,
    query: &str,
    limit: usize,
) -> Result<Vec<(u32, Vec<milli::score_details::ScoreDetails>)>, String> {
    let rtxn = index.read_txn().map_err(|e| e.to_string())?;
    let mut search = milli::Search::new(&rtxn, index);
    search.query(query);
    search.limit(limit);

    let result = search.execute().map_err(|e| e.to_string())?;

    Ok(result
        .documents_ids
        .into_iter()
        .zip(result.document_scores)
        .collect())
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

    // Store PDF and text blobs
    {
        let mut storage_guard = state.storage.write().await;
        let storage = storage_guard
            .as_mut()
            .ok_or_else(|| "Storage not initialized".to_string())?;

        // Read PDF bytes and store as blob
        let pdf_bytes = std::fs::read(path_ref).map_err(|e| e.to_string())?;
        storage
            .store_blob(&pdf_bytes)
            .await
            .map_err(|e| e.to_string())?;

        // Store extracted text as blob
        storage
            .store_blob(extracted.text.as_bytes())
            .await
            .map_err(|e| e.to_string())?;

        // Create document metadata and store in collection
        let metadata = DocumentMetadata {
            id: doc_id.clone(),
            name: file_name.clone(),
            pdf_hash: extracted.pdf_hash.clone(),
            text_hash: extracted.text_hash.clone(),
            page_count: extracted.page_count,
            tags: vec![],
            created_at: created_at.clone(),
        };

        storage
            .add_document(namespace_id, metadata)
            .await
            .map_err(|e| e.to_string())?;
    }

    // Index in milli search
    let search_guard = state.search.read().await;
    if let Some(index) = search_guard.as_ref() {
        index_document(index, &doc_id, &file_name, &extracted.text)?;
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
        pdf_hash: extracted.pdf_hash,
        text_hash: extracted.text_hash,
        page_count: extracted.page_count,
        tags: vec![],
        created_at,
    })
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

/// Search documents
#[tauri::command]
pub async fn search(
    query: String,
    limit: Option<usize>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    tracing::info!("Searching for: {}", query);

    let search_guard = state.search.read().await;
    let index = search_guard
        .as_ref()
        .ok_or_else(|| "Search not initialized".to_string())?;

    let limit = limit.unwrap_or(20);
    let results = search_index(index, &query, limit)?;

    // Convert internal doc IDs to SearchResults
    let rtxn = index.read_txn().map_err(|e| e.to_string())?;
    let fields_ids_map = index.fields_ids_map(&rtxn).map_err(|e| e.to_string())?;

    let mut search_results = Vec::new();
    for (doc_id, scores) in results {
        // Get document from index
        if let Ok(docs) = index.documents(&rtxn, [doc_id]) {
            if let Some((_id, obkv)) = docs.first() {
                // Extract fields
                let id_field = fields_ids_map.id("id");
                let name_field = fields_ids_map.id("name");
                let content_field = fields_ids_map.id("content");

                let id = id_field
                    .and_then(|fid| obkv.get(fid))
                    .and_then(|v| serde_json::from_slice::<String>(v).ok())
                    .unwrap_or_default();

                let name = name_field
                    .and_then(|fid| obkv.get(fid))
                    .and_then(|v| serde_json::from_slice::<String>(v).ok())
                    .unwrap_or_default();

                let content = content_field
                    .and_then(|fid| obkv.get(fid))
                    .and_then(|v| serde_json::from_slice::<String>(v).ok())
                    .unwrap_or_default();

                // Create snippet from content
                let snippet = content.chars().take(200).collect::<String>();

                // Calculate score from score details
                let score = milli::score_details::ScoreDetails::global_score(scores.iter()) as f32;

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
                    score,
                    snippet,
                });
            }
        }
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
