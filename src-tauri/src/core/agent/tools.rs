use std::collections::HashMap;

use mistralrs::{Function, Tool, ToolType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tracing::{debug, info, warn};

use crate::core::search;
use crate::core::AppState;

/// Tool definition for LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

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

/// Get tool definitions for the LLM (legacy format)
pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search".to_string(),
            description: "Search documents in the collection. Returns document names, IDs, and relevant snippets. Use this to find documents related to a topic.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    },
                    "collection_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional: filter to specific collection IDs"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "read_document".to_string(),
            description: "Read the full text content of a document by its ID. Use this after searching to get the complete text of a relevant document.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "document_id": {
                        "type": "string",
                        "description": "The document ID to read"
                    }
                },
                "required": ["document_id"]
            }),
        },
    ]
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
                        },
                        "collection_ids": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Optional: filter to specific collection IDs"
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
                    "Read the full text content of a document by its ID. Use this after searching to get the complete text of a relevant document.".to_string()
                ),
                parameters: Some(json_to_hashmap(json!({
                    "type": "object",
                    "properties": {
                        "document_id": {
                            "type": "string",
                            "description": "The document ID to read"
                        }
                    },
                    "required": ["document_id"]
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
    let collection_ids: Option<Vec<String>> =
        tool_call.arguments["collection_ids"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        });

    info!(query = %query, "Executing search");
    if let Some(ref ids) = collection_ids {
        debug!(collection_ids = ?ids, "Filtering by collections");
    }

    let search_guard = state.search.read().await;
    let index = match search_guard.as_ref() {
        Some(i) => i,
        None => {
            warn!("Search index not initialized");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: "Search index not initialized".to_string(),
                is_error: true,
            };
        }
    };

    match search::search_index(index, query, 10, 0, collection_ids.as_deref()) {
        Ok(results) => {
            // Format results for LLM consumption
            let doc_ids: Vec<u32> = results.hits.iter().map(|h| h.doc_id).collect();
            info!(
                query = %query,
                hits = doc_ids.len(),
                "Search completed"
            );
            let formatted = format_search_results(index, &doc_ids);
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

fn format_search_results(index: &milli::Index, doc_ids: &[u32]) -> String {
    if doc_ids.is_empty() {
        return "No documents found.".to_string();
    }

    let mut results = Vec::new();
    let rtxn = match index.read_txn() {
        Ok(t) => t,
        Err(e) => return format!("Error reading index: {}", e),
    };

    for &doc_id in doc_ids {
        let id = search::get_document_field(index, &rtxn, doc_id, "id")
            .ok()
            .flatten()
            .unwrap_or_default();
        let name = search::get_document_field(index, &rtxn, doc_id, "name")
            .ok()
            .flatten()
            .unwrap_or_default();
        let content = search::get_document_field(index, &rtxn, doc_id, "content")
            .ok()
            .flatten()
            .unwrap_or_default();

        // Create snippet from first 200 chars
        let snippet: String = content.chars().take(200).collect();
        let snippet = if content.len() > 200 {
            format!("{}...", snippet)
        } else {
            snippet
        };

        results.push(format!(
            "- Document: {}\n  ID: {}\n  Snippet: {}",
            name, id, snippet
        ));
    }

    format!(
        "Found {} documents:\n\n{}",
        doc_ids.len(),
        results.join("\n\n")
    )
}

async fn execute_read_document(tool_call: &ToolCall, state: &AppState) -> ToolResult {
    let doc_id = tool_call.arguments["document_id"].as_str().unwrap_or("");

    info!(document_id = %doc_id, "Reading document");

    let search_guard = state.search.read().await;
    let index = match search_guard.as_ref() {
        Some(i) => i,
        None => {
            warn!("Search index not initialized");
            return ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: "Search index not initialized".to_string(),
                is_error: true,
            };
        }
    };

    // Get content field from search index by external ID
    match search::get_document_by_external_id(index, doc_id) {
        Ok(Some(content)) => {
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
        Ok(None) => {
            warn!(document_id = %doc_id, "Document not found");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Document not found: {}", doc_id),
                is_error: true,
            }
        }
        Err(e) => {
            warn!(document_id = %doc_id, error = %e, "Error reading document");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error reading document: {}", e),
                is_error: true,
            }
        }
    }
}
