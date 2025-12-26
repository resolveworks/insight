use std::collections::HashMap;

use mistralrs::{Function, Tool, ToolType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use super::AgentContext;
use crate::search;

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Get tools in mistralrs format for structured tool calling
pub fn get_mistralrs_tools() -> Vec<Tool> {
    vec![
        Tool {
            tp: ToolType::Function,
            function: Function {
                name: "search".to_string(),
                description: Some(
                    "Search documents using hybrid keyword and semantic matching. Finds documents by exact terms, concepts, and meaning. Returns document names, IDs, and relevant passages.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query - can be keywords, phrases, or natural language describing what you're looking for"
                        }
                    },
                    "required": ["query"]
                }))),
            },
        },
        Tool {
            tp: ToolType::Function,
            function: Function {
                name: "read_chunk".to_string(),
                description: Some(
                    "Read a specific chunk from a document. Use this to get more context around a search result or read adjacent chunks. Chunk indices start at 0.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "document_id": {
                            "type": "string",
                            "description": "The document ID from search results"
                        },
                        "chunk_index": {
                            "type": "integer",
                            "description": "The chunk index (0-based). Use adjacent indices to read surrounding context."
                        }
                    },
                    "required": ["document_id", "chunk_index"]
                }))),
            },
        },
        Tool {
            tp: ToolType::Function,
            function: Function {
                name: "list_documents".to_string(),
                description: Some(
                    "List all documents in the current collection(s) with their metadata. Use this to get an overview of available documents before searching, or to find documents by characteristics like page count rather than content. Returns document names, IDs, and page counts.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }))),
            },
        },
    ]
}

/// Convert a serde_json::Value to HashMap<String, Value> for mistralrs
fn json_to_hashmap(value: Value) -> HashMap<String, Value> {
    match value {
        Value::Object(map) => map.into_iter().collect(),
        _ => HashMap::new(),
    }
}

/// Execute a tool call and return the result
pub async fn execute_tool(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    match tool_call.name.as_str() {
        "search" => execute_search(tool_call, ctx).await,
        "read_chunk" => execute_read_chunk(tool_call, ctx).await,
        "list_documents" => execute_list_documents(tool_call, ctx).await,
        _ => ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: format!("Unknown tool: {}", tool_call.name),
            is_error: true,
        },
    }
}

/// Hybrid search combining keyword (BM25) and semantic matching.
/// Falls back to keyword-only if no embedder is configured.
async fn execute_search(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    let query = tool_call.arguments["query"].as_str().unwrap_or("");

    info!(query = %query, "Executing hybrid search");

    // Try to get query embedding for semantic component
    let embedder_guard = ctx.state.embedder.read().await;
    let (query_vector, semantic_ratio) = match embedder_guard.as_ref() {
        Some(embedder) => match embedder.embed(query).await {
            Ok(vec) => {
                debug!(dimensions = vec.len(), "Query embedded for hybrid search");
                (Some(vec), 0.6) // 60% semantic, 40% keyword
            }
            Err(e) => {
                warn!(error = %e, "Failed to embed query, using keyword-only search");
                (None, 0.0)
            }
        },
        None => {
            debug!("No embedder configured, using keyword-only search");
            (None, 0.0)
        }
    };
    drop(embedder_guard);

    let index = ctx.state.search.read().await;
    let collection_ids = ctx.collection_ids();

    let search_params = search::SearchParams {
        query,
        limit: 15,
        query_vector,
        semantic_ratio,
        min_score: if semantic_ratio > 0.0 {
            Some(0.15)
        } else {
            None
        },
        collection_ids: collection_ids.as_deref(),
        ..Default::default()
    };

    match search::search_index(&index, search_params) {
        Ok(results) => {
            let doc_ids: Vec<u32> = results.hits.iter().map(|h| h.doc_id).collect();
            info!(
                query = %query,
                hits = doc_ids.len(),
                hybrid = semantic_ratio > 0.0,
                "Search completed"
            );
            let formatted = format_search_results(&index, &doc_ids, ctx);
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: formatted,
                is_error: false,
            }
        }
        Err(e) => {
            warn!(query = %query, error = %e, "Search failed");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Search error: {}", e),
                is_error: true,
            }
        }
    }
}

fn format_search_results(index: &milli::Index, doc_ids: &[u32], ctx: &AgentContext) -> String {
    if doc_ids.is_empty() {
        return "No matching passages found.".to_string();
    }

    // Build a lookup map from collection_id -> collection_name
    let collection_names: std::collections::HashMap<String, String> = ctx
        .collections
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut results = Vec::new();
    let rtxn = match index.read_txn() {
        Ok(t) => t,
        Err(e) => return format!("Error reading index: {}", e),
    };

    for &doc_id in doc_ids {
        let doc = match search::get_document(index, &rtxn, doc_id) {
            Ok(Some(d)) => d,
            _ => continue,
        };

        let get_str = |key: &str| -> String {
            doc.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let get_num =
            |key: &str| -> i64 { doc.get(key).and_then(|v| v.as_i64()).unwrap_or_default() };

        let parent_id = get_str("parent_id");
        let parent_name = get_str("parent_name");
        let chunk_index = get_num("chunk_index");
        let page_count = get_num("page_count");
        let start_page = get_num("start_page");
        let end_page = get_num("end_page");
        let collection_id = get_str("collection_id");
        let content = get_str("content");

        // Look up collection name, fall back to ID if not found
        let collection_name = collection_names
            .get(&collection_id)
            .cloned()
            .unwrap_or_else(|| collection_id.chars().take(8).collect::<String>() + "...");

        // Truncate long passages
        let passage: String = content.chars().take(500).collect();
        let passage = if content.len() > 500 {
            format!("{}...", passage)
        } else {
            passage
        };

        // Format page reference - show specific page(s) if available, otherwise total page count
        let page_ref = if start_page > 0 && end_page > 0 {
            if start_page == end_page {
                format!("p. {}", start_page)
            } else {
                format!("pp. {}-{}", start_page, end_page)
            }
        } else if page_count == 1 {
            "1 page".to_string()
        } else {
            format!("{} pages", page_count)
        };

        results.push(format!(
            "- Document: {} ({})\n  Collection: {}\n  ID: {} | Chunk: {}\n  Passage: {}",
            parent_name, page_ref, collection_name, parent_id, chunk_index, passage
        ));
    }

    format!(
        "Found {} relevant passages:\n\n{}",
        doc_ids.len(),
        results.join("\n\n")
    )
}

async fn execute_read_chunk(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    let doc_id = tool_call.arguments["document_id"].as_str().unwrap_or("");
    let chunk_index = tool_call.arguments["chunk_index"].as_u64().unwrap_or(0) as usize;

    info!(document_id = %doc_id, chunk_index = chunk_index, "Reading chunk");

    // Build the chunk ID: "{parent_id}_chunk_{chunk_index}"
    let chunk_id = format!("{}_chunk_{}", doc_id, chunk_index);

    let index = ctx.state.search.read().await;

    match search::get_document_by_external_id(&index, &chunk_id) {
        Ok(Some(content)) => {
            info!(
                document_id = %doc_id,
                chunk_index = chunk_index,
                content_len = content.len(),
                "Chunk read successfully"
            );

            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content,
                is_error: false,
            }
        }
        Ok(None) => {
            warn!(document_id = %doc_id, chunk_index = chunk_index, "Chunk not found");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!(
                    "Chunk {} not found for document {}. Try a different chunk index.",
                    chunk_index, doc_id
                ),
                is_error: true,
            }
        }
        Err(e) => {
            warn!(document_id = %doc_id, chunk_index = chunk_index, error = %e, "Error reading chunk");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error reading chunk: {}", e),
                is_error: true,
            }
        }
    }
}

async fn execute_list_documents(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    info!("Listing documents");

    // Get collection IDs to list from
    let collection_ids = ctx.collection_ids();

    if collection_ids.is_none()
        || collection_ids
            .as_ref()
            .map(|c| c.is_empty())
            .unwrap_or(true)
    {
        return ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: "No collections selected. Please select collections to list documents from."
                .to_string(),
            is_error: true,
        };
    }

    let collection_ids = collection_ids.unwrap();

    // Build a lookup map from collection_id -> collection_name
    let collection_names: std::collections::HashMap<String, String> = ctx
        .collections
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let storage = ctx.state.storage.read().await;
    let mut all_documents = Vec::new();

    for collection_id in &collection_ids {
        let namespace_id: iroh_docs::NamespaceId = match collection_id.parse() {
            Ok(id) => id,
            Err(_) => {
                warn!(collection_id = %collection_id, "Invalid collection ID, skipping");
                continue;
            }
        };

        let collection_name = collection_names
            .get(collection_id)
            .cloned()
            .unwrap_or_else(|| collection_id.chars().take(8).collect::<String>() + "...");

        match storage.list_documents(namespace_id).await {
            Ok(documents) => {
                for doc in documents {
                    all_documents.push((collection_name.clone(), doc));
                }
            }
            Err(e) => {
                warn!(collection_id = %collection_id, error = %e, "Failed to list documents");
            }
        }
    }

    drop(storage);

    if all_documents.is_empty() {
        return ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: "No documents found in the selected collections.".to_string(),
            is_error: false,
        };
    }

    // Sort by collection name, then document name
    all_documents.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let total_docs = all_documents.len();
    let total_pages: usize = all_documents.iter().map(|(_, d)| d.page_count).sum();

    // Limit output to avoid overwhelming the model
    const MAX_DOCS_TO_SHOW: usize = 25;
    let showing_all = total_docs <= MAX_DOCS_TO_SHOW;
    let docs_to_show: Vec<_> = all_documents.iter().take(MAX_DOCS_TO_SHOW).collect();

    // Format results
    let mut results = Vec::new();
    let mut current_collection = String::new();

    for (collection_name, doc) in docs_to_show {
        // Add collection header when it changes
        if *collection_name != current_collection {
            if !current_collection.is_empty() {
                results.push(String::new()); // Blank line between collections
            }
            results.push(format!("## {}", collection_name));
            current_collection = collection_name.clone();
        }

        // Format page info
        let page_info = if doc.page_count == 1 {
            "1p".to_string()
        } else {
            format!("{}p", doc.page_count)
        };

        // Full ID needed for read_chunk tool
        results.push(format!("- {} ({}) [{}]", doc.name, page_info, doc.id));
    }

    let summary = if showing_all {
        format!(
            "Found {} document{} ({} total pages):\n\n{}",
            total_docs,
            if total_docs == 1 { "" } else { "s" },
            total_pages,
            results.join("\n")
        )
    } else {
        format!(
            "Found {} documents ({} total pages). Showing first {}:\n\n{}\n\n... and {} more documents. Use search to find specific documents.",
            total_docs,
            total_pages,
            MAX_DOCS_TO_SHOW,
            results.join("\n"),
            total_docs - MAX_DOCS_TO_SHOW
        )
    };

    info!(
        document_count = total_docs,
        total_pages = total_pages,
        "Listed documents"
    );

    ToolResult {
        tool_call_id: tool_call.id.clone(),
        content: summary,
        is_error: false,
    }
}
