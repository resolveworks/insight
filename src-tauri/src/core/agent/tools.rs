use std::collections::HashMap;

use mistralrs::{Function, Tool, ToolType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::core::search;
use crate::core::AppState;

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
                    "Search documents in the collection. Returns document names, IDs, and relevant snippets. Use this to find documents related to a topic.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "The search query"
                        }
                    },
                    "required": ["query"]
                }))),
            },
        },
        Tool {
            tp: ToolType::Function,
            function: Function {
                name: "read_document".to_string(),
                description: Some(
                    "Read the full text content of a document. Use the document_id and collection_id from search results.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "document_id": {
                            "type": "string",
                            "description": "The document ID from search results"
                        },
                        "collection_id": {
                            "type": "string",
                            "description": "The collection ID from search results"
                        }
                    },
                    "required": ["document_id", "collection_id"]
                }))),
            },
        },
        Tool {
            tp: ToolType::Function,
            function: Function {
                name: "semantic_search".to_string(),
                description: Some(
                    "Search documents by meaning and concepts, not just keywords. Use this when looking for documents about a topic, theme, or idea - even if they don't contain the exact words. For exact phrases or specific terms, use the regular search tool instead.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "A description of what you're looking for (concepts, themes, topics)"
                        }
                    },
                    "required": ["query"]
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
pub async fn execute_tool(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    match tool_call.name.as_str() {
        "search" => execute_search(tool_call, state).await,
        "semantic_search" => execute_semantic_search(tool_call, state).await,
        "read_document" => execute_read_document(tool_call, state).await,
        _ => ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: format!("Unknown tool: {}", tool_call.name),
            is_error: true,
        },
    }
}

async fn execute_search(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    let query = tool_call.arguments["query"].as_str().unwrap_or("");

    info!(query = %query, "Executing search");

    let index = state.search.read().await;

    // Agent uses keyword-only search (no semantic embeddings or score filtering)
    match search::search_index(
        &index,
        search::SearchParams {
            query,
            limit: 10,
            ..Default::default()
        },
    ) {
        Ok(results) => {
            // Format results for LLM consumption
            let doc_ids: Vec<u32> = results.hits.iter().map(|h| h.doc_id).collect();
            info!(
                query = %query,
                hits = doc_ids.len(),
                "Search completed"
            );
            let formatted = format_search_results(&index, &doc_ids);
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

async fn execute_semantic_search(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    let query = tool_call.arguments["query"].as_str().unwrap_or("");

    info!(query = %query, "Executing semantic search");

    // Get embedder to encode the query
    let embedder_guard = state.embedder.read().await;
    let query_vector = match embedder_guard.as_ref() {
        Some(embedder) => match embedder.embed(query).await {
            Ok(vec) => {
                debug!(dimensions = vec.len(), "Query embedded");
                Some(vec)
            }
            Err(e) => {
                warn!(error = %e, "Failed to embed query, falling back to keyword search");
                None
            }
        },
        None => {
            warn!("No embedder configured, falling back to keyword search");
            None
        }
    };
    drop(embedder_guard);

    let index = state.search.read().await;

    // Use semantic search with ratio 1.0 (pure semantic) if we have embeddings
    let search_params = search::SearchParams {
        query,
        limit: 10,
        query_vector,
        semantic_ratio: 1.0,
        min_score: Some(0.3), // Filter out low-relevance results
        ..Default::default()
    };

    match search::search_index(&index, search_params) {
        Ok(results) => {
            let doc_ids: Vec<u32> = results.hits.iter().map(|h| h.doc_id).collect();
            info!(
                query = %query,
                hits = doc_ids.len(),
                "Semantic search completed"
            );
            let formatted = format_search_results(&index, &doc_ids);
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: formatted,
                is_error: false,
            }
        }
        Err(e) => {
            warn!(query = %query, error = %e, "Semantic search failed");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Semantic search error: {}", e),
                is_error: true,
            }
        }
    }
}

fn format_search_results(index: &milli::Index, doc_ids: &[u32]) -> String {
    if doc_ids.is_empty() {
        return "No matching passages found.".to_string();
    }

    let mut results = Vec::new();
    let rtxn = match index.read_txn() {
        Ok(t) => t,
        Err(e) => return format!("Error reading index: {}", e),
    };

    for &doc_id in doc_ids {
        // Get chunk fields
        let parent_id = search::get_document_field(index, &rtxn, doc_id, "parent_id")
            .ok()
            .flatten()
            .unwrap_or_default();
        let parent_name = search::get_document_field(index, &rtxn, doc_id, "parent_name")
            .ok()
            .flatten()
            .unwrap_or_default();
        let collection_id = search::get_document_field(index, &rtxn, doc_id, "collection_id")
            .ok()
            .flatten()
            .unwrap_or_default();
        let content = search::get_document_field(index, &rtxn, doc_id, "content")
            .ok()
            .flatten()
            .unwrap_or_default();

        // Content is already the chunk text - show it directly (truncate if very long)
        let passage: String = content.chars().take(500).collect();
        let passage = if content.len() > 500 {
            format!("{}...", passage)
        } else {
            passage
        };

        results.push(format!(
            "- Document: {}\n  ID: {}\n  Collection: {}\n  Passage: {}",
            parent_name, parent_id, collection_id, passage
        ));
    }

    format!(
        "Found {} relevant passages:\n\n{}",
        doc_ids.len(),
        results.join("\n\n")
    )
}

async fn execute_read_document(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    let doc_id = tool_call.arguments["document_id"].as_str().unwrap_or("");
    let collection_id = tool_call.arguments["collection_id"].as_str().unwrap_or("");

    info!(document_id = %doc_id, collection_id = %collection_id, "Reading document");

    // Parse collection_id as namespace
    let namespace_id: iroh_docs::NamespaceId = match collection_id.parse() {
        Ok(id) => id,
        Err(_) => {
            warn!(collection_id = %collection_id, "Invalid collection ID");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Invalid collection ID: {}", collection_id),
                is_error: true,
            };
        }
    };

    // Fetch document metadata from storage
    let storage = state.storage.read().await;
    let document = match storage.get_document(namespace_id, doc_id).await {
        Ok(Some(doc)) => doc,
        Ok(None) => {
            warn!(document_id = %doc_id, "Document not found");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Document not found: {}", doc_id),
                is_error: true,
            };
        }
        Err(e) => {
            warn!(document_id = %doc_id, error = %e, "Error finding document");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error finding document: {}", e),
                is_error: true,
            };
        }
    };

    // Get text content from blob storage
    let text_hash: iroh_blobs::Hash = match document.text_hash.parse() {
        Ok(h) => h,
        Err(_) => {
            warn!(document_id = %doc_id, "Invalid text hash");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Invalid text hash for document: {}", doc_id),
                is_error: true,
            };
        }
    };

    match storage.get_blob(&text_hash).await {
        Ok(Some(bytes)) => match String::from_utf8(bytes) {
            Ok(content) => {
                info!(
                    document_id = %doc_id,
                    content_len = content.len(),
                    "Document read successfully"
                );
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    content,
                    is_error: false,
                }
            }
            Err(e) => {
                warn!(document_id = %doc_id, error = %e, "Invalid UTF-8 in document");
                ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    content: format!("Invalid text encoding in document: {}", doc_id),
                    is_error: true,
                }
            }
        },
        Ok(None) => {
            warn!(document_id = %doc_id, "Text content not found in storage");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Text content not found for document: {}", doc_id),
                is_error: true,
            }
        }
        Err(e) => {
            warn!(document_id = %doc_id, error = %e, "Error reading text content");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error reading document: {}", e),
                is_error: true,
            }
        }
    }
}
