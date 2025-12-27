//! Anthropic API provider
//!
//! Uses reqwest for streaming chat completions with tool calling via SSE.

use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::{
    ChatProvider, CompletedToolCall, CompletionResult, ProviderEvent, RemoteModelInfo,
    ToolDefinition,
};
use crate::agent::{ContentBlock, Message, MessageRole};

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Anthropic API provider
pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    /// Create a new Anthropic provider with the given API key and model
    pub fn new(api_key: &str, model: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            model: model.to_string(),
        }
    }

    /// Fetch available models from Anthropic API
    pub async fn fetch_models(api_key: &str) -> Result<Vec<RemoteModelInfo>> {
        let client = reqwest::Client::new();

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(api_key).context("Invalid API key format")?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );

        let response = client
            .get(ANTHROPIC_MODELS_URL)
            .headers(headers)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: AnthropicError = response.json().await?;
            return Err(anyhow::anyhow!(
                "Failed to fetch models: {}",
                error.error.message
            ));
        }

        let models_response: ModelsResponse = response.json().await?;

        let models: Vec<RemoteModelInfo> = models_response
            .data
            .into_iter()
            .map(|m| RemoteModelInfo {
                id: m.id,
                name: m.display_name,
                description: None,
            })
            .collect();

        Ok(models)
    }

    /// Verify API key by fetching models (alias for fetch_models for API consistency)
    pub async fn verify_api_key(api_key: &str) -> Result<Vec<RemoteModelInfo>> {
        Self::fetch_models(api_key).await
    }
}

#[async_trait]
impl ChatProvider for AnthropicProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult> {
        // Extract system message and convert other messages
        let (system, anthropic_messages) = convert_messages(messages);

        // Convert tools
        let anthropic_tools: Option<Vec<AnthropicTool>> = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| AnthropicTool {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.parameters.clone(),
                    })
                    .collect(),
            )
        };

        let request = AnthropicRequest {
            model: self.model.clone(),
            max_tokens: 8192,
            messages: anthropic_messages,
            system,
            tools: anthropic_tools,
            stream: Some(true),
        };

        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        headers.insert(
            "x-api-key",
            HeaderValue::from_str(&self.api_key).context("Invalid API key")?,
        );
        headers.insert(
            "anthropic-version",
            HeaderValue::from_static(ANTHROPIC_VERSION),
        );

        let response = self
            .client
            .post(ANTHROPIC_API_URL)
            .headers(headers)
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let error: AnthropicError = response.json().await?;
            return Err(anyhow::anyhow!(
                "Anthropic API error: {}",
                error.error.message
            ));
        }

        // Process SSE stream
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();
        let mut text_content = String::new();
        let mut tool_calls: std::collections::HashMap<usize, (String, String, String)> =
            std::collections::HashMap::new(); // index -> (id, name, arguments)

        while let Some(chunk_result) = stream.next().await {
            if cancel_token.is_cancelled() {
                break;
            }

            let chunk = chunk_result?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE events
            while let Some(event_end) = buffer.find("\n\n") {
                let event_data = buffer[..event_end].to_string();
                buffer = buffer[event_end + 2..].to_string();

                // Parse SSE event
                for line in event_data.lines() {
                    if let Some(data) = line.strip_prefix("data: ") {
                        if data == "[DONE]" {
                            continue;
                        }

                        if let Ok(event) = serde_json::from_str::<StreamEvent>(data) {
                            match event {
                                StreamEvent::ContentBlockStart {
                                    index,
                                    content_block,
                                } => match content_block {
                                    ContentBlockStart::ToolUse { id, name } => {
                                        tool_calls.insert(
                                            index,
                                            (id.clone(), name.clone(), String::new()),
                                        );
                                        let _ = event_tx
                                            .send(ProviderEvent::ToolCallStart { id, name })
                                            .await;
                                    }
                                    ContentBlockStart::Text { .. } => {}
                                },
                                StreamEvent::ContentBlockDelta { index, delta } => match delta {
                                    ContentBlockDelta::TextDelta { text } => {
                                        let _ = event_tx
                                            .send(ProviderEvent::TextDelta(text.clone()))
                                            .await;
                                        text_content.push_str(&text);
                                    }
                                    ContentBlockDelta::InputJsonDelta { partial_json } => {
                                        if let Some(tc) = tool_calls.get_mut(&index) {
                                            tc.2.push_str(&partial_json);
                                            let _ = event_tx
                                                .send(ProviderEvent::ToolCallDelta {
                                                    id: tc.0.clone(),
                                                    arguments_delta: partial_json,
                                                })
                                                .await;
                                        }
                                    }
                                },
                                StreamEvent::ContentBlockStop { index } => {
                                    if let Some(tc) = tool_calls.get(&index) {
                                        let _ = event_tx
                                            .send(ProviderEvent::ToolCallComplete {
                                                id: tc.0.clone(),
                                            })
                                            .await;
                                    }
                                }
                                StreamEvent::MessageStop => {
                                    debug!("Message complete");
                                }
                                StreamEvent::Error { error } => {
                                    let _ =
                                        event_tx.send(ProviderEvent::Error(error.message)).await;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Convert to completed tool calls
        let completed_tool_calls: Vec<CompletedToolCall> = tool_calls
            .into_values()
            .map(|(id, name, args)| {
                let arguments: serde_json::Value =
                    serde_json::from_str(&args).unwrap_or(serde_json::json!({}));
                CompletedToolCall {
                    id,
                    name,
                    arguments,
                }
            })
            .collect();

        let _ = event_tx.send(ProviderEvent::Done).await;

        Ok(CompletionResult {
            text: text_content,
            tool_calls: completed_tool_calls,
        })
    }

    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

/// Convert messages to Anthropic format, extracting system message
///
/// Anthropic requires tool_result blocks to be in a separate user message
/// after the assistant's tool_use message. This function handles splitting
/// assistant messages that contain both tool_use and tool_result blocks.
fn convert_messages(messages: &[Message]) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system = None;
    let mut result = Vec::new();

    for msg in messages {
        match msg.role {
            MessageRole::System => {
                // Extract system message text
                let text: String = msg
                    .content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text } => Some(text.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("");
                system = Some(text);
            }
            MessageRole::User => {
                let content = convert_user_blocks(&msg.content);
                result.push(AnthropicMessage {
                    role: "user".to_string(),
                    content,
                });
            }
            MessageRole::Assistant => {
                // Check if this message contains tool_result blocks
                // If so, we need to split into assistant (tool_use) + user (tool_result)
                let has_tool_results = msg
                    .content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolResult { .. }));

                // Add assistant message with text + tool_use blocks
                let assistant_content = convert_assistant_blocks(&msg.content);
                result.push(AnthropicMessage {
                    role: "assistant".to_string(),
                    content: assistant_content,
                });

                // If there are tool results, add them in a separate user message
                if has_tool_results {
                    let tool_result_content = extract_tool_results(&msg.content);
                    result.push(AnthropicMessage {
                        role: "user".to_string(),
                        content: tool_result_content,
                    });
                }
            }
        }
    }

    (system, result)
}

/// Convert content blocks to Anthropic format for assistant messages (text + tool_use)
fn convert_assistant_blocks(blocks: &[ContentBlock]) -> AnthropicContent {
    let mut parts = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                if !text.is_empty() {
                    parts.push(AnthropicContentPart::Text { text: text.clone() });
                }
            }
            ContentBlock::ToolUse {
                id,
                name,
                arguments,
            } => {
                parts.push(AnthropicContentPart::ToolUse {
                    id: id.clone(),
                    name: name.clone(),
                    input: arguments.clone(),
                });
            }
            ContentBlock::ToolResult { .. } => {
                // Tool results go in a separate user message
            }
        }
    }

    if parts.len() == 1 {
        if let AnthropicContentPart::Text { text } = &parts[0] {
            return AnthropicContent::Text(text.clone());
        }
    }

    AnthropicContent::Parts(parts)
}

/// Convert content blocks to Anthropic format for user messages (text + tool_result)
fn convert_user_blocks(blocks: &[ContentBlock]) -> AnthropicContent {
    let mut parts = Vec::new();

    for block in blocks {
        match block {
            ContentBlock::Text { text } => {
                if !text.is_empty() {
                    parts.push(AnthropicContentPart::Text { text: text.clone() });
                }
            }
            ContentBlock::ToolUse { .. } => {
                // Tool uses go in assistant messages
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                parts.push(AnthropicContentPart::ToolResult {
                    tool_use_id: tool_use_id.clone(),
                    content: content.clone(),
                    is_error: Some(*is_error),
                });
            }
        }
    }

    if parts.len() == 1 {
        if let AnthropicContentPart::Text { text } = &parts[0] {
            return AnthropicContent::Text(text.clone());
        }
    }

    AnthropicContent::Parts(parts)
}

/// Extract only tool_result blocks for the separate user message after tool_use
fn extract_tool_results(blocks: &[ContentBlock]) -> AnthropicContent {
    let parts: Vec<AnthropicContentPart> = blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => Some(AnthropicContentPart::ToolResult {
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: Some(*is_error),
            }),
            _ => None,
        })
        .collect();

    AnthropicContent::Parts(parts)
}

// ============================================================================
// API Types
// ============================================================================

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Parts(Vec<AnthropicContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentPart {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct AnthropicError {
    error: AnthropicErrorDetail,
}

#[derive(Debug, Deserialize)]
struct AnthropicErrorDetail {
    message: String,
}

// Models API response types
#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Debug, Deserialize)]
struct ModelInfo {
    id: String,
    display_name: String,
}

// Stream event types
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)] // Fields required for deserialization but not all are read
enum StreamEvent {
    MessageStart {
        message: serde_json::Value,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlockStart,
    },
    ContentBlockDelta {
        index: usize,
        delta: ContentBlockDelta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: serde_json::Value,
    },
    MessageStop,
    Ping,
    Error {
        error: AnthropicErrorDetail,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(dead_code)] // Fields required for deserialization but not all are read
enum ContentBlockStart {
    Text { text: String },
    ToolUse { id: String, name: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ContentBlockDelta {
    TextDelta { text: String },
    InputJsonDelta { partial_json: String },
}
