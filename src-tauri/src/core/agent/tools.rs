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
        "read_chunk" => execute_read_chunk(tool_call, state).await,
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
        let content = get_str("content");

        // Truncate long passages
        let passage: String = content.chars().take(500).collect();
        let passage = if content.len() > 500 {
            format!("{}...", passage)
        } else {
            passage
        };

        results.push(format!(
            "- Document: {}\n  ID: {}\n  Chunk: {}\n  Passage: {}",
            parent_name, parent_id, chunk_index, passage
        ));
    }

    format!(
        "Found {} relevant passages:\n\n{}",
        doc_ids.len(),
        results.join("\n\n")
    )
}

async fn execute_read_chunk(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    let doc_id = tool_call.arguments["document_id"].as_str().unwrap_or("");
    let chunk_index = tool_call.arguments["chunk_index"].as_u64().unwrap_or(0) as usize;

    info!(document_id = %doc_id, chunk_index = chunk_index, "Reading chunk");

    // Build the chunk ID: "{parent_id}_chunk_{chunk_index}"
    let chunk_id = format!("{}_chunk_{}", doc_id, chunk_index);

    let index = state.search.read().await;

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
