//! OpenAI chat provider via the Responses API.

use anyhow::{Context, Result};
use async_openai::{
    config::OpenAIConfig,
    types::responses::{
        CreateResponse, EasyInputContent, EasyInputMessage, FunctionCallOutput,
        FunctionCallOutputItemParam, FunctionTool, FunctionToolCall, InputItem, InputParam, Item,
        MessageType, OutputItem, Role, Tool,
    },
    Client,
};
use async_trait::async_trait;
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use crate::agent::{render_context_message, ContentBlock, Message, MessageRole};
use crate::provider::{
    ChatProvider, CompletedToolCall, CompletionResult, Provider, ProviderEvent, RemoteModelInfo,
    ToolDefinition,
};

pub struct OpenAIChatProvider {
    client: Client<OpenAIConfig>,
    model: String,
}

impl OpenAIChatProvider {
    pub fn new(api_key: &str, model: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
            model: model.to_string(),
        }
    }

    pub async fn fetch_models(api_key: &str) -> Result<Vec<RemoteModelInfo>> {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let client = Client::with_config(config);

        let models = client
            .models()
            .list()
            .await
            .context("Failed to list models")?;

        // Filter to chat/reasoning models only
        let chat_models: Vec<RemoteModelInfo> = models
            .data
            .into_iter()
            .filter(|m| {
                m.id.starts_with("gpt-5")
                    || m.id.starts_with("gpt-4")
                    || m.id.starts_with("gpt-3.5")
                    || m.id.starts_with("o1")
                    || m.id.starts_with("o3")
            })
            .map(|m| RemoteModelInfo {
                id: m.id.clone(),
                name: format_model_name(&m.id),
                description: None,
            })
            .collect();

        Ok(chat_models)
    }
}

fn format_model_name(id: &str) -> String {
    match id {
        "gpt-5" => "GPT-5".to_string(),
        "gpt-5-mini" => "GPT-5 Mini".to_string(),
        "gpt-5-nano" => "GPT-5 Nano".to_string(),
        "gpt-5-pro" => "GPT-5 Pro".to_string(),
        "gpt-5.2" => "GPT-5.2".to_string(),
        "gpt-4o" => "GPT-4o".to_string(),
        "gpt-4o-mini" => "GPT-4o Mini".to_string(),
        "gpt-4-turbo" => "GPT-4 Turbo".to_string(),
        "gpt-4" => "GPT-4".to_string(),
        "gpt-3.5-turbo" => "GPT-3.5 Turbo".to_string(),
        "o1" => "o1".to_string(),
        "o1-mini" => "o1 Mini".to_string(),
        "o1-preview" => "o1 Preview".to_string(),
        "o3" => "o3".to_string(),
        "o3-mini" => "o3 Mini".to_string(),
        _ => id.to_string(),
    }
}

impl Provider for OpenAIChatProvider {
    fn provider_name(&self) -> &'static str {
        "openai"
    }

    fn model_id(&self) -> &str {
        &self.model
    }
}

#[async_trait]
impl ChatProvider for OpenAIChatProvider {
    async fn stream_completion(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        event_tx: mpsc::Sender<ProviderEvent>,
        cancel_token: CancellationToken,
    ) -> Result<CompletionResult> {
        let (instructions, input_items) = convert_messages(messages);

        // OpenAI strict mode requires additionalProperties: false in schemas
        let openai_tools: Option<Vec<Tool>> = if tools.is_empty() {
            None
        } else {
            Some(
                tools
                    .iter()
                    .map(|t| {
                        let mut params = t.parameters.clone();
                        if let Some(obj) = params.as_object_mut() {
                            obj.insert(
                                "additionalProperties".to_string(),
                                serde_json::Value::Bool(false),
                            );
                        }
                        Tool::Function(FunctionTool {
                            name: t.name.clone(),
                            description: Some(t.description.clone()),
                            parameters: Some(params),
                            strict: Some(true),
                        })
                    })
                    .collect(),
            )
        };

        let request = CreateResponse {
            model: Some(self.model.clone()),
            input: InputParam::Items(input_items),
            instructions,
            tools: openai_tools,
            stream: Some(true),
            ..Default::default()
        };

        let mut stream = self
            .client
            .responses()
            .create_stream(request)
            .await
            .context("Failed to create response stream")?;

        let mut text_content = String::new();
        let mut tool_calls: std::collections::HashMap<u32, (String, String, String)> =
            std::collections::HashMap::new();

        while let Some(event_result) = stream.next().await {
            if cancel_token.is_cancelled() {
                break;
            }

            let event = event_result.context("Stream error")?;

            use async_openai::types::responses::ResponseStreamEvent;

            match event {
                ResponseStreamEvent::ResponseOutputTextDelta(delta) => {
                    let _ = event_tx
                        .send(ProviderEvent::TextDelta(delta.delta.clone()))
                        .await;
                    text_content.push_str(&delta.delta);
                }

                ResponseStreamEvent::ResponseOutputItemAdded(item_added) => {
                    if let OutputItem::FunctionCall(fc) = &item_added.item {
                        tool_calls.insert(
                            item_added.output_index,
                            (fc.call_id.clone(), fc.name.clone(), String::new()),
                        );
                        let _ = event_tx
                            .send(ProviderEvent::ToolCallStart {
                                id: fc.call_id.clone(),
                                name: fc.name.clone(),
                            })
                            .await;
                    }
                }

                ResponseStreamEvent::ResponseFunctionCallArgumentsDelta(delta) => {
                    if let Some(tc) = tool_calls.get_mut(&delta.output_index) {
                        tc.2.push_str(&delta.delta);
                        let _ = event_tx
                            .send(ProviderEvent::ToolCallDelta {
                                id: tc.0.clone(),
                                arguments_delta: delta.delta,
                            })
                            .await;
                    }
                }

                ResponseStreamEvent::ResponseFunctionCallArgumentsDone(done) => {
                    if let Some(tc) = tool_calls.get_mut(&done.output_index) {
                        tc.2 = done.arguments;
                        let _ = event_tx
                            .send(ProviderEvent::ToolCallComplete { id: tc.0.clone() })
                            .await;
                    }
                }

                ResponseStreamEvent::ResponseCompleted(_) => {
                    debug!("Response completed");
                }

                ResponseStreamEvent::ResponseFailed(failed) => {
                    let error_msg = format!("Response failed: {:?}", failed.response.error);
                    let _ = event_tx.send(ProviderEvent::Error(error_msg.clone())).await;
                    return Err(anyhow::anyhow!(error_msg));
                }

                ResponseStreamEvent::ResponseError(err) => {
                    let _ = event_tx
                        .send(ProviderEvent::Error(err.message.clone()))
                        .await;
                    return Err(anyhow::anyhow!("OpenAI error: {}", err.message));
                }

                _ => {}
            }
        }

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
}

fn convert_messages(messages: &[Message]) -> (Option<String>, Vec<InputItem>) {
    let mut instructions = None;
    let mut items = Vec::new();

    for msg in messages {
        match msg.role {
            MessageRole::System => {
                instructions = Some(msg.text());
            }
            MessageRole::Context => {
                // The Responses API exposes a single `instructions` field, so
                // breadcrumbs ride along as tagged user-role notes at the point
                // they occurred in the transcript.
                items.push(InputItem::EasyMessage(EasyInputMessage {
                    r#type: MessageType::Message,
                    role: Role::User,
                    content: EasyInputContent::Text(render_context_message(&msg.text())),
                }));
            }
            MessageRole::User => {
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            items.push(InputItem::EasyMessage(EasyInputMessage {
                                r#type: MessageType::Message,
                                role: Role::User,
                                content: EasyInputContent::Text(text.clone()),
                            }));
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            items.push(InputItem::Item(Item::FunctionCallOutput(
                                FunctionCallOutputItemParam {
                                    call_id: tool_use_id.clone(),
                                    output: FunctionCallOutput::Text(content.clone()),
                                    id: None,
                                    status: None,
                                },
                            )));
                        }
                        _ => {}
                    }
                }
            }
            MessageRole::Assistant => {
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text } => {
                            items.push(InputItem::EasyMessage(EasyInputMessage {
                                r#type: MessageType::Message,
                                role: Role::Assistant,
                                content: EasyInputContent::Text(text.clone()),
                            }));
                        }
                        ContentBlock::ToolUse {
                            id,
                            name,
                            arguments,
                        } => {
                            items.push(InputItem::Item(Item::FunctionCall(FunctionToolCall {
                                call_id: id.clone(),
                                name: name.clone(),
                                arguments: arguments.to_string(),
                                id: None,
                                status: None,
                            })));
                        }
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            items.push(InputItem::Item(Item::FunctionCallOutput(
                                FunctionCallOutputItemParam {
                                    call_id: tool_use_id.clone(),
                                    output: FunctionCallOutput::Text(content.clone()),
                                    id: None,
                                    status: None,
                                },
                            )));
                        }
                    }
                }
            }
        }
    }

    (instructions, items)
}
