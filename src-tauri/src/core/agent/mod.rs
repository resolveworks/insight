pub mod tools;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use mistralrs::{
    CalledFunction, GgufModelBuilder, Model, PagedAttentionMetaBuilder, RequestBuilder,
    TextMessageRole, Tool, ToolCallResponse, ToolCallType, ToolChoice,
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
        .with_paged_attn(|| PagedAttentionMetaBuilder::default().build())?
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
    Tool,
}

/// A tool call made by the assistant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageToolCall {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl From<&ToolCallResponse> for MessageToolCall {
    fn from(tc: &ToolCallResponse) -> Self {
        Self {
            index: tc.index,
            id: tc.id.clone(),
            name: tc.function.name.clone(),
            arguments: tc.function.arguments.clone(),
        }
    }
}

impl MessageToolCall {
    /// Convert to mistralrs ToolCallResponse for request building
    pub fn to_tool_call_response(&self) -> ToolCallResponse {
        ToolCallResponse {
            index: self.index,
            id: self.id.clone(),
            tp: ToolCallType::Function,
            function: CalledFunction {
                name: self.name.clone(),
                arguments: self.arguments.clone(),
            },
        }
    }
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    /// For Tool role: the ID of the tool call this is a response to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For Assistant role: tool calls made by the assistant
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<MessageToolCall>>,
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
                content: SYSTEM_PROMPT.to_string(),
                tool_call_id: None,
                tool_calls: None,
            }],
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Generate title from first user message (truncated to 50 chars)
    pub fn generate_title(&mut self) {
        if let Some(first_user_msg) = self.messages.iter().find(|m| m.role == MessageRole::User) {
            let mut title = first_user_msg.content.clone();
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

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::User,
            content,
            tool_call_id: None,
            tool_calls: None,
        });
        self.touch();
    }

    pub fn add_assistant_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
            tool_call_id: None,
            tool_calls: None,
        });
        self.touch();
    }

    pub fn add_assistant_message_with_tool_calls(
        &mut self,
        content: String,
        tool_calls: Vec<ToolCallResponse>,
    ) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
            tool_call_id: None,
            tool_calls: Some(tool_calls.iter().map(MessageToolCall::from).collect()),
        });
        self.touch();
    }

    pub fn add_tool_result(&mut self, tool_call_id: String, content: String) {
        self.messages.push(Message {
            role: MessageRole::Tool,
            content,
            tool_call_id: Some(tool_call_id),
            tool_calls: None,
        });
        self.touch();
    }
}

/// Events emitted during agent execution
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", content = "data")]
pub enum AgentEvent {
    TextDelta {
        content: String,
    },
    ToolCallStart {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    ToolCallResult {
        id: String,
        content: String,
        is_error: bool,
    },
    Done,
    Error {
        message: String,
    },
}

/// Maximum number of tool call iterations
const MAX_ITERATIONS: usize = 10;

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

        // Use non-streaming request for tool calling (more reliable)
        // Tool calls need to be complete before execution
        debug!("Sending request to model");
        let response = match model.send_chat_request(request).await {
            Ok(r) => r,
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

        let choice = response
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("No response from model"))?;

        let message = &choice.message;
        let content = message.content.clone().unwrap_or_default();
        let tool_call_count = message.tool_calls.as_ref().map(|tc| tc.len()).unwrap_or(0);

        debug!(
            content_len = content.len(),
            tool_calls = tool_call_count,
            "Received model response"
        );

        // Emit text content if present
        if !content.is_empty() {
            debug!(content_len = content.len(), "Emitting text delta");
            let _ = event_tx
                .send(AgentEvent::TextDelta {
                    content: content.clone(),
                })
                .await;
        }

        // Check for tool calls
        if let Some(ref tool_calls) = message.tool_calls {
            if !tool_calls.is_empty() {
                info!(tool_count = tool_calls.len(), "Model requested tool calls");

                // Store assistant message with tool calls for conversation history
                conversation
                    .add_assistant_message_with_tool_calls(content.clone(), tool_calls.clone());

                // Execute each tool call
                for called in tool_calls {
                    let tool_call = ToolCall {
                        id: called.id.clone(),
                        name: called.function.name.clone(),
                        arguments: serde_json::from_str(&called.function.arguments)
                            .unwrap_or(serde_json::json!({})),
                    };

                    info!(
                        tool_name = %tool_call.name,
                        tool_id = %tool_call.id,
                        "Executing tool"
                    );
                    debug!(arguments = %tool_call.arguments, "Tool arguments");

                    // Emit tool call start
                    let _ = event_tx
                        .send(AgentEvent::ToolCallStart {
                            id: tool_call.id.clone(),
                            name: tool_call.name.clone(),
                            arguments: tool_call.arguments.clone(),
                        })
                        .await;

                    // Execute tool
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

                    // Emit tool result
                    let _ = event_tx
                        .send(AgentEvent::ToolCallResult {
                            id: result.tool_call_id.clone(),
                            content: result.content.clone(),
                            is_error: result.is_error,
                        })
                        .await;

                    // Add tool result to conversation
                    conversation.add_tool_result(result.tool_call_id, result.content);
                }

                // Continue loop to let model process tool results
                debug!("Continuing to next iteration for tool result processing");
                continue;
            }
        }

        // No tool calls - add regular assistant message and we're done
        info!(
            conversation_id = %conversation.id,
            iterations = iteration + 1,
            "Agent loop completed"
        );
        conversation.add_assistant_message(content);
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
        match msg.role {
            MessageRole::System => {
                request = request.add_message(TextMessageRole::System, &msg.content);
            }
            MessageRole::User => {
                request = request.add_message(TextMessageRole::User, &msg.content);
            }
            MessageRole::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    // Convert to mistralrs format
                    let responses: Vec<ToolCallResponse> = tool_calls
                        .iter()
                        .map(|tc| tc.to_tool_call_response())
                        .collect();
                    request = request.add_message_with_tool_call(
                        TextMessageRole::Assistant,
                        msg.content.clone(),
                        responses,
                    );
                } else {
                    request = request.add_message(TextMessageRole::Assistant, &msg.content);
                }
            }
            MessageRole::Tool => {
                if let Some(ref id) = msg.tool_call_id {
                    request = request.add_tool_message(msg.content.clone(), id.clone());
                }
            }
        }
    }

    request
}
