//! Chat provider abstraction for LLM inference
//!
//! This module provides a unified interface for different LLM backends:
//! - Local models via mistralrs
//! - OpenAI API
//! - Anthropic API

pub mod anthropic;
pub mod local;
pub mod openai;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::Message;

/// Provider-agnostic tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// Get tool definitions in provider-agnostic format
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

/// Events emitted by providers during streaming
#[derive(Debug, Clone)]
pub enum ProviderEvent {
    /// Text content delta
    TextDelta(String),
    /// A tool call has started
    ToolCallStart { id: String, name: String },
    /// Tool call arguments delta (streamed JSON)
    ToolCallDelta { id: String, arguments_delta: String },
    /// Tool call is complete
    ToolCallComplete { id: String },
    /// Generation is complete
    Done,
    /// An error occurred
    Error(String),
}

/// Unified chat provider interface
#[async_trait]
pub trait ChatProvider: Send + Sync {
    /// Stream a chat completion with optional tool calling
    ///
    /// The provider should emit events via `event_tx` as content streams in.
    /// Tool calls are accumulated and returned when complete.
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult>;

    /// Get the provider name (e.g., "local", "openai", "anthropic")
    fn provider_name(&self) -> &'static str;

    /// Get the model identifier
    fn model_id(&self) -> &str;
}

/// Result of a streaming completion
#[derive(Debug, Clone, Default)]
pub struct CompletionResult {
    /// Accumulated text content
    pub text: String,
    /// Completed tool calls
    pub tool_calls: Vec<CompletedToolCall>,
}

/// A completed tool call from the model
#[derive(Debug, Clone)]
pub struct CompletedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Provider configuration stored in settings
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    /// Local model via mistralrs
    Local { model_id: String },
    /// OpenAI API
    #[serde(rename = "openai")]
    OpenAI { api_key: String, model: String },
    /// Anthropic API
    Anthropic { api_key: String, model: String },
}

impl ProviderConfig {
    /// Get the provider type name
    pub fn provider_type(&self) -> &'static str {
        match self {
            ProviderConfig::Local { .. } => "local",
            ProviderConfig::OpenAI { .. } => "openai",
            ProviderConfig::Anthropic { .. } => "anthropic",
        }
    }

    /// Get the model ID
    pub fn model_id(&self) -> &str {
        match self {
            ProviderConfig::Local { model_id } => model_id,
            ProviderConfig::OpenAI { model, .. } => model,
            ProviderConfig::Anthropic { model, .. } => model,
        }
    }
}

/// Information about a remote model (from API)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModelInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Provider family for UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFamily {
    pub id: String,
    pub name: String,
    pub description: String,
    pub requires_api_key: bool,
}

/// Get available provider families
pub fn get_provider_families() -> Vec<ProviderFamily> {
    vec![
        ProviderFamily {
            id: "local".to_string(),
            name: "Local".to_string(),
            description: "Run models locally on your machine".to_string(),
            requires_api_key: false,
        },
        ProviderFamily {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            description: "GPT-4o, GPT-4, and other OpenAI models".to_string(),
            requires_api_key: true,
        },
        ProviderFamily {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            description: "Claude 3.5 Sonnet, Claude 3 Opus, and more".to_string(),
            requires_api_key: true,
        },
    ]
}
