pub mod tools;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mistralrs::{
    GgufModelBuilder, Model, PagedAttentionMetaBuilder, RequestBuilder, TextMessageRole, Tool,
    ToolCallResponse, ToolChoice,
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

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    /// For Tool role: the ID of the tool call this is a response to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// For Assistant role: tool calls made by the assistant
    #[serde(skip)]
    pub tool_calls: Option<Vec<ToolCallResponse>>,
}

/// A conversation with message history
#[derive(Debug, Clone)]
pub struct Conversation {
    pub id: String,
    pub messages: Vec<Message>,
    pub created_at: String,
}

impl Conversation {
    pub fn new(id: String) -> Self {
        Self {
            id,
            messages: vec![Message {
                role: MessageRole::System,
                content: SYSTEM_PROMPT.to_string(),
                tool_call_id: None,
                tool_calls: None,
            }],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::User,
            content,
            tool_call_id: None,
            tool_calls: None,
        });
    }

    pub fn add_assistant_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
            tool_call_id: None,
            tool_calls: None,
        });
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
            tool_calls: Some(tool_calls),
        });
    }

    pub fn add_tool_result(&mut self, tool_call_id: String, content: String) {
        self.messages.push(Message {
            role: MessageRole::Tool,
            content,
            tool_call_id: Some(tool_call_id),
            tool_calls: None,
        });
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
    conversation.add_user_message(user_message);

    // Get tools for this session
    let tools = get_mistralrs_tools();

    for _iteration in 0..MAX_ITERATIONS {
        if cancel_token.is_cancelled() {
            return Ok(());
        }

        // Build request from conversation with tools
        let request = build_request_from_conversation(conversation, &tools);

        // Use non-streaming request for tool calling (more reliable)
        // Tool calls need to be complete before execution
        let response = model.send_chat_request(request).await?;

        let choice = response
            .choices
            .first()
            .ok_or_else(|| anyhow::anyhow!("No response from model"))?;

        let message = &choice.message;
        let content = message.content.clone().unwrap_or_default();

        // Emit text content if present
        if !content.is_empty() {
            let _ = event_tx
                .send(AgentEvent::TextDelta {
                    content: content.clone(),
                })
                .await;
        }

        // Check for tool calls
        if let Some(ref tool_calls) = message.tool_calls {
            if !tool_calls.is_empty() {
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
                continue;
            }
        }

        // No tool calls - add regular assistant message and we're done
        conversation.add_assistant_message(content);
        let _ = event_tx.send(AgentEvent::Done).await;
        return Ok(());
    }

    // Max iterations reached
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
                    // Assistant message with tool calls
                    request = request.add_message_with_tool_call(
                        TextMessageRole::Assistant,
                        msg.content.clone(),
                        tool_calls.clone(),
                    );
                } else {
                    // Regular assistant message
                    request = request.add_message(TextMessageRole::Assistant, &msg.content);
                }
            }
            MessageRole::Tool => {
                // Tool result message
                if let Some(ref id) = msg.tool_call_id {
                    request = request.add_tool_message(msg.content.clone(), id.clone());
                }
            }
        }
    }

    request
}
