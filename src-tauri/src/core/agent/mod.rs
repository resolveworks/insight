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

pub use tools::{
    execute_tool, get_mistralrs_tools, get_tool_definitions, ToolCall, ToolDefinition, ToolResult,
};

use super::models::ModelInfo;

/// System prompt for the agent
const SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists analyze documents.
You have access to a document collection that you can search and read.

Always use the search tool first to find relevant documents, then read specific documents to get detailed information. Cite document names when providing information."#;

/// Wrapper around mistral.rs Model
pub struct AgentModel {
    model: Arc<Model>,
}

impl AgentModel {
    /// Load a GGUF model from local cache
    ///
    /// The model must be downloaded first using ModelManager::download_model.
    /// The model_path should point to the directory containing the GGUF file.
    pub async fn load(model_path: &Path, model_info: &ModelInfo) -> Result<Self> {
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

/// Result of a tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUseResult {
    pub content: String,
    pub is_error: bool,
}

/// A content block within a message
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        arguments: serde_json::Value,
        /// Result of the tool execution, None while pending
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<ToolUseResult>,
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
            // Extract text from content blocks (excluding thinking)
            let text: String = first_user_msg
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    ContentBlock::Thinking { .. } => None,
                    ContentBlock::ToolUse { .. } => None,
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

    /// Add an assistant message with content blocks (text/thinking + tool uses)
    pub fn add_assistant_message(&mut self, content: Vec<ContentBlock>) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
        });
        self.touch();
    }

    /// Set the result on a ToolUse block in the last assistant message
    pub fn set_tool_result(&mut self, tool_use_id: &str, content: String, is_error: bool) {
        if let Some(last_msg) = self.messages.last_mut() {
            if last_msg.role == MessageRole::Assistant {
                for block in &mut last_msg.content {
                    if let ContentBlock::ToolUse { id, result, .. } = block {
                        if id == tool_use_id {
                            *result = Some(ToolUseResult { content, is_error });
                            break;
                        }
                    }
                }
            }
        }
        self.touch();
    }
}

/// Delta content for streaming blocks
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentDelta {
    Text { text: String },
    Thinking { thinking: String },
}

/// Events emitted during agent execution
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A new block has started streaming
    ContentBlockStart { index: usize, block: ContentBlock },
    /// Delta content for a block at index
    ContentBlockDelta { index: usize, delta: ContentDelta },
    /// Block streaming is complete
    ContentBlockStop { index: usize },
    /// Agent turn is complete
    Done,
    /// An error occurred
    Error { message: String },
}

/// Maximum number of tool call iterations
const MAX_ITERATIONS: usize = 10;

/// State for streaming text with think tag detection
#[derive(Debug)]
enum StreamState {
    /// Normal text mode
    Text,
    /// Inside <think>...</think> block
    Thinking,
}

/// Parses streaming text and emits block lifecycle events
///
/// Handles Qwen3-style `<think>...</think>` tags by detecting transitions
/// and emitting appropriate ContentBlockStart/Delta/Stop events.
struct StreamingBlockParser {
    state: StreamState,
    buffer: String,
    block_index: usize,
    block_started: bool,
    /// Accumulated content for each block (for building final ContentBlocks)
    blocks: Vec<ContentBlock>,
    /// Current block's accumulated content
    current_content: String,
}

impl StreamingBlockParser {
    fn new(starting_index: usize) -> Self {
        Self {
            state: StreamState::Text,
            buffer: String::new(),
            block_index: starting_index,
            block_started: false,
            blocks: Vec::new(),
            current_content: String::new(),
        }
    }

    /// Returns the next available block index (for tool calls after streaming)
    fn next_index(&self) -> usize {
        if self.block_started {
            self.block_index + 1
        } else {
            self.block_index
        }
    }

    /// Push new text and return any events to emit
    fn push(&mut self, text: &str) -> Vec<AgentEvent> {
        self.buffer.push_str(text);
        self.process_buffer()
    }

    /// Finish parsing and return any final events
    fn finish(&mut self) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        // Emit any remaining buffered content
        if !self.buffer.is_empty() {
            let remaining = std::mem::take(&mut self.buffer);
            events.extend(self.emit_content(&remaining));
        }

        // Close current block if open
        if self.block_started {
            self.finalize_current_block();
            events.push(AgentEvent::ContentBlockStop {
                index: self.block_index,
            });
        }

        events
    }

    /// Get the final content blocks
    fn into_blocks(self) -> Vec<ContentBlock> {
        self.blocks
    }

    fn process_buffer(&mut self) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        loop {
            match self.state {
                StreamState::Text => {
                    // Look for <think> tag
                    if let Some(pos) = self.buffer.find("<think>") {
                        // Emit text before the tag
                        if pos > 0 {
                            let before = self.buffer[..pos].to_string();
                            events.extend(self.emit_content(&before));
                        }

                        // Close text block if open, switch to thinking
                        if self.block_started {
                            self.finalize_current_block();
                            events.push(AgentEvent::ContentBlockStop {
                                index: self.block_index,
                            });
                            self.block_index += 1;
                            self.block_started = false;
                        }

                        self.state = StreamState::Thinking;
                        self.buffer = self.buffer[pos + 7..].to_string(); // skip "<think>"
                    } else if self.buffer.len() > 7 && !self.buffer.ends_with('<') {
                        // Safe to emit if we have content and it doesn't end with potential tag start
                        // Keep last 7 chars in case of partial "<think>"
                        let safe_len = self.buffer.len() - 7;
                        let to_emit = self.buffer[..safe_len].to_string();
                        self.buffer = self.buffer[safe_len..].to_string();
                        if !to_emit.is_empty() {
                            events.extend(self.emit_content(&to_emit));
                        }
                        break;
                    } else {
                        // Not enough content or might be partial tag, wait for more
                        break;
                    }
                }
                StreamState::Thinking => {
                    // Look for </think> tag
                    if let Some(pos) = self.buffer.find("</think>") {
                        // Emit thinking content before the tag
                        if pos > 0 {
                            let before = self.buffer[..pos].to_string();
                            events.extend(self.emit_content(&before));
                        }

                        // Close thinking block, switch to text
                        if self.block_started {
                            self.finalize_current_block();
                            events.push(AgentEvent::ContentBlockStop {
                                index: self.block_index,
                            });
                            self.block_index += 1;
                            self.block_started = false;
                        }

                        self.state = StreamState::Text;
                        self.buffer = self.buffer[pos + 8..].to_string(); // skip "</think>"
                    } else if self.buffer.len() > 8 && !self.buffer.ends_with('<') {
                        // Safe to emit thinking content
                        let safe_len = self.buffer.len() - 8;
                        let to_emit = self.buffer[..safe_len].to_string();
                        self.buffer = self.buffer[safe_len..].to_string();
                        if !to_emit.is_empty() {
                            events.extend(self.emit_content(&to_emit));
                        }
                        break;
                    } else {
                        break;
                    }
                }
            }
        }

        events
    }

    fn emit_content(&mut self, content: &str) -> Vec<AgentEvent> {
        let mut events = Vec::new();

        if !self.block_started {
            // Start a new block
            let block = match self.state {
                StreamState::Text => ContentBlock::Text {
                    text: String::new(),
                },
                StreamState::Thinking => ContentBlock::Thinking {
                    thinking: String::new(),
                },
            };
            events.push(AgentEvent::ContentBlockStart {
                index: self.block_index,
                block,
            });
            self.block_started = true;
            self.current_content.clear();
        }

        // Emit delta
        let delta = match self.state {
            StreamState::Text => ContentDelta::Text {
                text: content.to_string(),
            },
            StreamState::Thinking => ContentDelta::Thinking {
                thinking: content.to_string(),
            },
        };
        events.push(AgentEvent::ContentBlockDelta {
            index: self.block_index,
            delta,
        });

        self.current_content.push_str(content);
        events
    }

    fn finalize_current_block(&mut self) {
        if !self.current_content.is_empty() {
            let block = match self.state {
                StreamState::Text => ContentBlock::Text {
                    text: std::mem::take(&mut self.current_content),
                },
                StreamState::Thinking => ContentBlock::Thinking {
                    thinking: std::mem::take(&mut self.current_content),
                },
            };
            self.blocks.push(block);
        }
    }
}

/// Run the agent loop with structured tool calling
pub async fn run_agent_loop(
    model: &Arc<Model>,
    conversation: &mut Conversation,
    user_message: String,
    state: &crate::core::AppState,
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

    // Global block index across all iterations - frontend sees one flat stream
    let mut next_block_index = 0usize;

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
        let mut streamer = StreamingBlockParser::new(next_block_index);

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

                        // Process text content through streaming parser
                        if let Some(text) = delta_content {
                            if !text.is_empty() {
                                for event in streamer.push(text) {
                                    let _ = event_tx.send(event).await;
                                }
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

        // Finalize streaming (close any open blocks)
        let finish_events = streamer.finish();
        for event in &finish_events {
            let _ = event_tx.send(event.clone()).await;
        }

        // Update global index before consuming streamer
        next_block_index = streamer.next_index();
        let mut content_blocks = streamer.into_blocks();
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
                let arguments: serde_json::Value =
                    serde_json::from_str(&called.function.arguments)
                        .unwrap_or(serde_json::json!({}));

                info!(
                    tool_name = %called.function.name,
                    tool_id = %called.id,
                    "Executing tool"
                );
                debug!(arguments = %arguments, "Tool arguments");

                // Create ToolUse block (without result yet)
                let tool_use_block = ContentBlock::ToolUse {
                    id: called.id.clone(),
                    name: called.function.name.clone(),
                    arguments: arguments.clone(),
                    result: None,
                };

                // Emit block start (shows loading state in UI)
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStart {
                        index: next_block_index,
                        block: tool_use_block.clone(),
                    })
                    .await;

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

                // Create final block with result
                let final_block = ContentBlock::ToolUse {
                    id: called.id.clone(),
                    name: called.function.name.clone(),
                    arguments: tool_call.arguments.clone(),
                    result: Some(ToolUseResult {
                        content: result.content,
                        is_error: result.is_error,
                    }),
                };

                // Emit updated block with result, then stop
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStart {
                        index: next_block_index,
                        block: final_block.clone(),
                    })
                    .await;
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStop {
                        index: next_block_index,
                    })
                    .await;

                content_blocks.push(final_block);
                next_block_index += 1;
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
        .set_tool_choice(ToolChoice::Auto);

    for msg in &conversation.messages {
        // Extract text content from blocks (including thinking wrapped in tags for context)
        let text: String = msg
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.clone()),
                ContentBlock::Thinking { thinking } => {
                    // Re-wrap thinking in tags so model maintains context
                    Some(format!("<think>{}</think>", thinking))
                }
                ContentBlock::ToolUse { .. } => None,
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
                            ..
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
                    request = request
                        .add_message_with_tool_call(TextMessageRole::Assistant, text, tool_uses);

                    // Add tool result messages for each ToolUse that has a result
                    for block in &msg.content {
                        if let ContentBlock::ToolUse {
                            id,
                            result: Some(tool_result),
                            ..
                        } = block
                        {
                            request =
                                request.add_tool_message(tool_result.content.clone(), id.clone());
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
