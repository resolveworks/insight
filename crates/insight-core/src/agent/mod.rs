pub mod tools;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use mistralrs::{
    CalledFunction, ChatCompletionChunkResponse, Delta, GgufModelBuilder, Model, RequestBuilder,
    Response, TextMessageRole, Tool, ToolCallResponse, ToolCallType, ToolChoice,
};

pub use tools::{execute_tool, get_mistralrs_tools, ToolCall, ToolResult};

use crate::models::LanguageModelInfo;

/// System prompt for the agent
const SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists investigate document collections.

Be concise. Answer in 2-4 sentences unless the user asks for more detail. Cite document names so findings are verifiable.

When answering questions:
1. Search to find relevant documents
2. Read documents to extract specific details
3. Cite sources (document name)
4. Note any gaps or contradictions worth pursuing"#;

/// Wrapper around mistral.rs Model
pub struct AgentModel {
    model: Arc<Model>,
}

impl AgentModel {
    /// Load a GGUF model from local cache
    ///
    /// The model must be downloaded first using ModelManager::download().
    /// The model_path should point to the directory containing the GGUF file.
    pub async fn load(model_path: &Path, model_info: &LanguageModelInfo) -> Result<Self> {
        // GgufModelBuilder takes a local directory path and filename(s)
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
        })
    }

    /// Get a reference to the model
    pub fn model(&self) -> &Arc<Model> {
        &self.model
    }
}

/// Message role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    User,
    Assistant,
}

/// A content block within a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: Vec<ContentBlock>,
}

/// A conversation with message history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<Message>,
    pub created_at: String,
    pub updated_at: String,
}

impl Conversation {
    pub fn new(id: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            title: "New conversation".to_string(),
            messages: vec![Message {
                role: MessageRole::System,
                content: vec![ContentBlock::Text {
                    text: SYSTEM_PROMPT.to_string(),
                }],
            }],
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Generate title from first user message (truncated to 50 chars)
    pub fn generate_title(&mut self) {
        if let Some(first_user_msg) = self.messages.iter().find(|m| m.role == MessageRole::User) {
            let text: String = first_user_msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            let mut title = text;
            if title.len() > 50 {
                title.truncate(47);
                title.push_str("...");
            }
            self.title = title;
        }
    }

    pub fn touch(&mut self) {
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn add_user_message(&mut self, text: String) {
        self.messages.push(Message {
            role: MessageRole::User,
            content: vec![ContentBlock::Text { text }],
        });
        self.touch();
    }

    /// Add an assistant message with content blocks
    pub fn add_assistant_message(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
        });
        self.touch();
    }
}

/// Delta content for streaming blocks
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    Text { text: String },
}

/// Events emitted during agent execution
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A new block has started streaming
    ContentBlockStart { block: ContentBlock },
    /// Delta content for the current block
    ContentBlockDelta { delta: ContentDelta },
    /// Current block streaming is complete
    ContentBlockStop,
    /// Agent turn is complete
    Done,
    /// An error occurred
    Error { message: String },
}

/// Maximum number of tool call iterations
const MAX_ITERATIONS: usize = 10;

/// Run the agent loop with structured tool calling
pub async fn run_agent_loop(
    model: &Arc<Model>,
    conversation: &mut Conversation,
    user_message: String,
    state: &crate::AppState,
    event_tx: mpsc::Sender<AgentEvent>,
    cancel_token: CancellationToken,
) -> Result<()> {
    info!(
        conversation_id = %conversation.id,
        message_len = user_message.len(),
        "Starting agent loop"
    );
    conversation.add_user_message(user_message);

    // Get tools for this session
    let tools = get_mistralrs_tools();
    debug!(tool_count = tools.len(), "Loaded tools");

    for iteration in 0..MAX_ITERATIONS {
        if cancel_token.is_cancelled() {
            info!(conversation_id = %conversation.id, "Agent loop cancelled");
            return Ok(());
        }

        debug!(
            iteration = iteration + 1,
            max_iterations = MAX_ITERATIONS,
            message_count = conversation.messages.len(),
            "Starting iteration"
        );

        // Build request from conversation with tools
        let request = build_request_from_conversation(conversation, &tools);

        // Use streaming request for responsive UI
        debug!("Sending streaming request to model");
        let mut stream = match model.stream_chat_request(request).await {
            Ok(s) => s,
            Err(e) => {
                tracing::error!(
                    conversation_id = %conversation.id,
                    iteration = iteration + 1,
                    error = %e,
                    error_debug = ?e,
                    "Model request failed"
                );
                return Err(e);
            }
        };

        // Streaming state
        let mut tool_calls: Vec<ToolCallResponse> = Vec::new();
        let mut text_started = false;
        let mut text_content = String::new();

        // Stream response chunks
        while let Some(chunk) = stream.next().await {
            if cancel_token.is_cancelled() {
                info!(conversation_id = %conversation.id, "Agent loop cancelled during streaming");
                return Ok(());
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
                                if !text_started {
                                    let _ = event_tx
                                        .send(AgentEvent::ContentBlockStart {
                                            block: ContentBlock::Text {
                                                text: String::new(),
                                            },
                                        })
                                        .await;
                                    text_started = true;
                                }
                                let _ = event_tx
                                    .send(AgentEvent::ContentBlockDelta {
                                        delta: ContentDelta::Text { text: text.clone() },
                                    })
                                    .await;
                                text_content.push_str(text);
                            }
                        }

                        // Accumulate tool calls (streamed as deltas)
                        if let Some(calls) = delta_tool_calls {
                            for call in calls {
                                if let Some(existing) =
                                    tool_calls.iter_mut().find(|tc| tc.index == call.index)
                                {
                                    existing
                                        .function
                                        .arguments
                                        .push_str(&call.function.arguments);
                                } else {
                                    tool_calls.push(call.clone());
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
                    tracing::error!(error = %msg, "Model error during streaming");
                    let _ = event_tx
                        .send(AgentEvent::Error {
                            message: msg.clone(),
                        })
                        .await;
                    return Err(anyhow::anyhow!("Model error: {}", msg));
                }
                _ => {}
            }
        }

        // Finalize text block
        let mut content_blocks = Vec::new();
        if text_started {
            let _ = event_tx.send(AgentEvent::ContentBlockStop).await;
            if !text_content.is_empty() {
                content_blocks.push(ContentBlock::Text { text: text_content });
            }
        }
        let tool_call_count = tool_calls.len();
        debug!(
            block_count = content_blocks.len(),
            tool_calls = tool_call_count,
            "Received model response"
        );

        // Check for tool calls
        if !tool_calls.is_empty() {
            info!(tool_count = tool_calls.len(), "Model requested tool calls");

            // Process each tool call
            for called in &tool_calls {
                let arguments: serde_json::Value = serde_json::from_str(&called.function.arguments)
                    .unwrap_or_else(|e| {
                        warn!(
                            tool_name = %called.function.name,
                            raw_args = %called.function.arguments,
                            error = %e,
                            "Failed to parse tool arguments, using empty object"
                        );
                        serde_json::json!({})
                    });

                info!(
                    tool_name = %called.function.name,
                    tool_id = %called.id,
                    "Executing tool"
                );
                debug!(arguments = %arguments, "Tool arguments");

                // Emit ToolUse block
                let tool_use_block = ContentBlock::ToolUse {
                    id: called.id.clone(),
                    name: called.function.name.clone(),
                    arguments: arguments.clone(),
                };
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStart {
                        block: tool_use_block.clone(),
                    })
                    .await;
                let _ = event_tx.send(AgentEvent::ContentBlockStop).await;
                content_blocks.push(tool_use_block);

                // Execute tool
                let tool_call = ToolCall {
                    id: called.id.clone(),
                    name: called.function.name.clone(),
                    arguments,
                };
                let result = execute_tool(&tool_call, state).await;

                if result.is_error {
                    warn!(
                        tool_name = %tool_call.name,
                        error = %result.content,
                        "Tool execution failed"
                    );
                } else {
                    debug!(
                        tool_name = %tool_call.name,
                        result_len = result.content.len(),
                        "Tool execution succeeded"
                    );
                }

                // Emit ToolResult block
                let tool_result_block = ContentBlock::ToolResult {
                    tool_use_id: called.id.clone(),
                    content: result.content,
                    is_error: result.is_error,
                };
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStart {
                        block: tool_result_block.clone(),
                    })
                    .await;
                let _ = event_tx.send(AgentEvent::ContentBlockStop).await;
                content_blocks.push(tool_result_block);
            }

            // Store assistant message with all content blocks (including tool uses with results)
            conversation.add_assistant_message(content_blocks);

            // Continue loop to let model process tool results
            debug!("Continuing to next iteration for tool result processing");
            continue;
        }

        // No tool calls - store and we're done
        info!(
            conversation_id = %conversation.id,
            iterations = iteration + 1,
            "Agent loop completed"
        );

        // Ensure we have at least one block
        if content_blocks.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: String::new(),
            });
        }

        conversation.add_assistant_message(content_blocks);
        let _ = event_tx.send(AgentEvent::Done).await;
        return Ok(());
    }

    // Max iterations reached
    warn!(
        conversation_id = %conversation.id,
        max_iterations = MAX_ITERATIONS,
        "Agent loop reached maximum iterations"
    );
    let _ = event_tx
        .send(AgentEvent::Error {
            message: "Maximum iterations reached".to_string(),
        })
        .await;

    Ok(())
}

/// Build a RequestBuilder from the conversation history
fn build_request_from_conversation(conversation: &Conversation, tools: &[Tool]) -> RequestBuilder {
    let mut request = RequestBuilder::new()
        .set_tools(tools.to_vec())
        .set_tool_choice(ToolChoice::Auto)
        .enable_thinking(false);

    for msg in &conversation.messages {
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

                    // Add tool result messages from ToolResult blocks
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
