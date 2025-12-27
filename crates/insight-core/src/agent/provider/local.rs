//! Local model provider using mistralrs

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use mistralrs::{
    CalledFunction, ChatCompletionChunkResponse, Delta, GgufModelBuilder, Model, RequestBuilder,
    Response, TextMessageRole, Tool, ToolCallResponse, ToolCallType, ToolChoice, ToolType,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::{ChatProvider, CompletedToolCall, CompletionResult, ProviderEvent, ToolDefinition};
use crate::agent::{ContentBlock, Message, MessageRole};
use crate::models::LanguageModelInfo;

/// Local LLM provider using mistralrs
pub struct LocalProvider {
    model: Arc<Model>,
    model_id: String,
}

impl LocalProvider {
    /// Load a GGUF model from local cache
    pub async fn load(model_path: &Path, model_info: &LanguageModelInfo) -> Result<Self> {
        let model = GgufModelBuilder::new(
            model_path.to_string_lossy().to_string(),
            vec![model_info.gguf_file.clone()],
        )
        .with_tok_model_id(&model_info.tokenizer_repo_id)
        .with_logging()
        .build()
        .await
        .context("Failed to load GGUF model")?;

        Ok(Self {
            model: Arc::new(model),
            model_id: model_info.id.clone(),
        })
    }

    /// Get a reference to the underlying model (for backwards compatibility)
    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }
}

#[async_trait]
impl ChatProvider for LocalProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult> {
        // Convert tools to mistralrs format
        let mistral_tools = convert_tools(tools);

        // Build request from messages
        let request = build_request(messages, &mistral_tools);

        // Stream the response
        let mut stream = self.model.stream_chat_request(request).await?;

        let mut text_content = String::new();
        let mut tool_calls: Vec<ToolCallResponse> = Vec::new();

        while let Some(chunk) = stream.next().await {
            if cancel_token.is_cancelled() {
                break;
            }

            match chunk {
                Response::Chunk(ChatCompletionChunkResponse { choices, .. }) => {
                    if let Some(choice) = choices.first() {
                        let Delta {
                            content: delta_content,
                            tool_calls: delta_tool_calls,
                            ..
                        } = &choice.delta;

                        // Stream text content
                        if let Some(text) = delta_content {
                            if !text.is_empty() {
                                let _ = event_tx.send(ProviderEvent::TextDelta(text.clone())).await;
                                text_content.push_str(text);
                            }
                        }

                        // Accumulate tool calls
                        if let Some(calls) = delta_tool_calls {
                            for call in calls {
                                if let Some(existing) =
                                    tool_calls.iter_mut().find(|tc| tc.index == call.index)
                                {
                                    // Accumulate arguments
                                    let args_delta = call.function.arguments.clone();
                                    existing.function.arguments.push_str(&args_delta);
                                    let _ = event_tx
                                        .send(ProviderEvent::ToolCallDelta {
                                            id: existing.id.clone(),
                                            arguments_delta: args_delta,
                                        })
                                        .await;
                                } else {
                                    // New tool call
                                    tool_calls.push(call.clone());
                                    let _ = event_tx
                                        .send(ProviderEvent::ToolCallStart {
                                            id: call.id.clone(),
                                            name: call.function.name.clone(),
                                        })
                                        .await;
                                }
                            }
                        }
                    }
                }
                Response::Done(_) => {
                    debug!("Streaming complete");
                    break;
                }
                Response::ModelError(msg, _) => {
                    let _ = event_tx.send(ProviderEvent::Error(msg.clone())).await;
                    return Err(anyhow::anyhow!("Model error: {}", msg));
                }
                _ => {}
            }
        }

        // Convert tool calls to completed format
        let completed_tool_calls: Vec<CompletedToolCall> = tool_calls
            .into_iter()
            .map(|tc| {
                let arguments: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
                CompletedToolCall {
                    id: tc.id,
                    name: tc.function.name,
                    arguments,
                }
            })
            .collect();

        // Emit completion events for tool calls
        for tc in &completed_tool_calls {
            let _ = event_tx
                .send(ProviderEvent::ToolCallComplete { id: tc.id.clone() })
                .await;
        }

        let _ = event_tx.send(ProviderEvent::Done).await;

        Ok(CompletionResult {
            text: text_content,
            tool_calls: completed_tool_calls,
        })
    }

    fn provider_name(&self) -> &'static str {
        "local"
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }
}

/// Convert provider-agnostic tool definitions to mistralrs format
fn convert_tools(tools: &[ToolDefinition]) -> Vec<Tool> {
    tools
        .iter()
        .map(|t| Tool {
            tp: ToolType::Function,
            function: mistralrs::Function {
                name: t.name.clone(),
                description: Some(t.description.clone()),
                parameters: Some(json_to_hashmap(&t.parameters)),
            },
        })
        .collect()
}

/// Convert serde_json::Value to HashMap for mistralrs
fn json_to_hashmap(value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    }
}

/// Build a RequestBuilder from messages
fn build_request(messages: &[Message], tools: &[Tool]) -> RequestBuilder {
    let mut request = RequestBuilder::new()
        .set_tools(tools.to_vec())
        .set_tool_choice(ToolChoice::Auto)
        .enable_thinking(false);

    for msg in messages {
        // Extract text content from blocks
        let text: String = msg
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        match msg.role {
            MessageRole::System => {
                request = request.add_message(TextMessageRole::System, &text);
            }
            MessageRole::User => {
                request = request.add_message(TextMessageRole::User, &text);
            }
            MessageRole::Assistant => {
                // Extract tool uses from content blocks
                let tool_uses: Vec<ToolCallResponse> = msg
                    .content
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, b)| match b {
                        ContentBlock::ToolUse {
                            id,
                            name,
                            arguments,
                        } => Some(ToolCallResponse {
                            index: idx,
                            id: id.clone(),
                            tp: ToolCallType::Function,
                            function: CalledFunction {
                                name: name.clone(),
                                arguments: arguments.to_string(),
                            },
                        }),
                        _ => None,
                    })
                    .collect();

                if !tool_uses.is_empty() {
                    request = request.add_message_with_tool_call(
                        TextMessageRole::Assistant,
                        text,
                        tool_uses,
                    );

                    // Add tool result messages
                    for block in &msg.content {
                        if let ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } = block
                        {
                            request =
                                request.add_tool_message(content.clone(), tool_use_id.clone());
                        }
                    }
                } else {
                    request = request.add_message(TextMessageRole::Assistant, &text);
                }
            }
        }
    }

    request
}
