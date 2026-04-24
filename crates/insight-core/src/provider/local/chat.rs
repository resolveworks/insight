//! Local chat provider using mistralrs GGUF models.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

use crate::agent::{render_context_message, ContentBlock, Message, MessageRole};
use crate::models::LanguageModelInfo;
use crate::provider::{
    ChatProvider, CompletedToolCall, CompletionResult, MemoryKind, Provider, ProviderEvent,
    ToolDefinition,
};

use super::LocalModelState;

/// Local LLM provider backed by a mistralrs GGUF model.
///
/// Construction is cheap — it records what to load. Weights are brought
/// into memory on [`Provider::ensure_loaded`]. `unload` drops the inner
/// `Arc`; any in-flight inference keeps its own clone and finishes at the
/// natural boundary.
pub struct LocalChatProvider {
    model_path: PathBuf,
    gguf_file: String,
    tokenizer_repo_id: String,
    state: LocalModelState<Model>,
}

impl LocalChatProvider {
    pub fn new(model_path: impl AsRef<Path>, model_info: &LanguageModelInfo) -> Self {
        Self {
            model_path: model_path.as_ref().to_path_buf(),
            gguf_file: model_info.gguf_file.clone(),
            tokenizer_repo_id: model_info.tokenizer_repo_id.clone(),
            state: LocalModelState::new(model_info.id.clone()),
        }
    }
}

#[async_trait]
impl Provider for LocalChatProvider {
    fn provider_name(&self) -> &'static str {
        "local"
    }

    fn model_id(&self) -> &str {
        self.state.model_id()
    }

    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Local
    }

    fn coexist(&self) -> bool {
        self.state.coexist()
    }

    fn set_coexist(&self, coexist: bool) {
        self.state.set_coexist(coexist);
    }

    async fn is_loaded(&self) -> bool {
        self.state.is_loaded().await
    }

    async fn ensure_loaded(&self) -> Result<()> {
        let path = self.model_path.clone();
        let gguf = self.gguf_file.clone();
        let tok = self.tokenizer_repo_id.clone();
        let model_id = self.state.model_id().to_string();

        self.state
            .get_or_load(|| async move {
                tracing::info!("Loading local chat model '{}'...", model_id);
                let model = GgufModelBuilder::new(path.to_string_lossy().to_string(), vec![gguf])
                    .with_tok_model_id(&tok)
                    .with_logging()
                    .build()
                    .await
                    .context("Failed to load GGUF model")?;
                tracing::info!("Local chat model '{}' loaded", model_id);
                Ok(model)
            })
            .await
            .map(|_| ())
    }

    async fn unload(&self) -> Result<()> {
        if self.state.unload().await {
            tracing::info!("Unloaded local chat model '{}'", self.state.model_id());
        }
        Ok(())
    }
}

#[async_trait]
impl ChatProvider for LocalChatProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult> {
        self.ensure_loaded().await?;
        let model: Arc<Model> = self
            .state
            .current()
            .await
            .ok_or_else(|| anyhow::anyhow!("Local chat model not loaded"))?;

        let mistral_tools = convert_tools(tools);
        let request = build_request(messages, &mistral_tools);

        let mut stream = model.stream_chat_request(request).await?;

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

                        if let Some(text) = delta_content {
                            if !text.is_empty() {
                                let _ = event_tx.send(ProviderEvent::TextDelta(text.clone())).await;
                                text_content.push_str(text);
                            }
                        }

                        if let Some(calls) = delta_tool_calls {
                            for call in calls {
                                if let Some(existing) =
                                    tool_calls.iter_mut().find(|tc| tc.index == call.index)
                                {
                                    let args_delta = call.function.arguments.clone();
                                    existing.function.arguments.push_str(&args_delta);
                                    let _ = event_tx
                                        .send(ProviderEvent::ToolCallDelta {
                                            id: existing.id.clone(),
                                            arguments_delta: args_delta,
                                        })
                                        .await;
                                } else {
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
}

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

fn json_to_hashmap(value: &serde_json::Value) -> HashMap<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        _ => HashMap::new(),
    }
}

fn build_request(messages: &[Message], tools: &[Tool]) -> RequestBuilder {
    let mut request = RequestBuilder::new()
        .set_tools(tools.to_vec())
        .set_tool_choice(ToolChoice::Auto)
        .enable_thinking(false);

    for msg in messages {
        let text = msg.text();

        match msg.role {
            MessageRole::System => {
                request = request.add_message(TextMessageRole::System, &text);
            }
            MessageRole::Context => {
                // Chat templates vary in how mid-stream system messages are
                // rendered; tagging as a user note is the most portable way
                // to make sure the model actually sees the breadcrumb.
                request = request.add_message(TextMessageRole::User, render_context_message(&text));
            }
            MessageRole::User => {
                request = request.add_message(TextMessageRole::User, &text);
            }
            MessageRole::Assistant => {
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
