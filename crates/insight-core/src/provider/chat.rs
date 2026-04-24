//! Chat provider role trait and supporting types.
//!
//! A [`ChatProvider`] streams a chat completion with optional tool calls.
//! Local and remote implementations share this interface so the agent loop
//! is provider-agnostic.

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::agent::Message;

use super::Provider;

/// Tool definition in a provider-agnostic shape.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Tool definitions available to the agent.
pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "search".to_string(),
            description: "Search documents using hybrid keyword and semantic matching. Finds documents by exact terms, concepts, and meaning. Returns document names, IDs, and relevant passages.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query - can be keywords, phrases, or natural language describing what you're looking for"
                    }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "read_chunk".to_string(),
            description: "Read a specific chunk from a document. Use this to get more context around a search result or read adjacent chunks. Chunk indices start at 0.".to_string(),
            parameters: serde_json::json!({
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
            }),
        },
        ToolDefinition {
            name: "list_documents".to_string(),
            description: "List all documents in the current collection(s) with their metadata. Use this to get an overview of available documents before searching, or to find documents by characteristics like page count rather than content. Returns document names, IDs, and page counts.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "get_collection_terms".to_string(),
            description: "Get the most common terms/words in the collection(s), sorted by how many documents contain them. Use this to understand what topics the documents cover before searching. Returns terms with their document frequency.".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of terms to return (default: 50, max: 200)"
                    }
                },
                "required": []
            }),
        },
    ]
}

/// Events emitted by providers during streaming.
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    TextDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, arguments_delta: String },
    ToolCallComplete { id: String },
    Done,
    Error(String),
}

/// Streaming completion result.
#[derive(Debug, Clone, Default)]
pub struct CompletionResult {
    pub text: String,
    pub tool_calls: Vec<CompletedToolCall>,
}

/// A completed tool call the model emitted.
#[derive(Debug, Clone)]
pub struct CompletedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Build [`CompletedToolCall`]s from raw `(id, name, arguments_json)` tuples.
///
/// Shared between chat providers: they accumulate tool calls differently
/// during streaming (local keeps a `Vec`, remotes keep index-keyed
/// `HashMap`s) but land on the same final shape. Malformed argument JSON
/// degrades to an empty object rather than failing the completion.
pub fn finalize_tool_calls<I>(raw: I) -> Vec<CompletedToolCall>
where
    I: IntoIterator<Item = (String, String, String)>,
{
    raw.into_iter()
        .map(|(id, name, args)| CompletedToolCall {
            id,
            name,
            arguments: serde_json::from_str(&args).unwrap_or_else(|_| serde_json::json!({})),
        })
        .collect()
}

/// Chat role trait. Extends [`Provider`] with a streaming completion method.
#[async_trait]
pub trait ChatProvider: Provider {
    /// Stream a chat completion with optional tool calling.
    ///
    /// Events stream via `event_tx` as content arrives. Tool calls are
    /// accumulated and returned in the final [`CompletionResult`].
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult>;
}
