pub mod tools;

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use mistralrs::{
    Model, PagedAttentionMetaBuilder, RequestBuilder, Response, TextMessageRole, TextModelBuilder,
};

pub use tools::{execute_tool, get_tool_definitions, ToolCall, ToolDefinition, ToolResult};

/// Default model to use
const DEFAULT_MODEL_ID: &str = "microsoft/Phi-3.5-mini-instruct";

/// System prompt for the agent
const SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists analyze documents.
You have access to a document collection that you can search and read.

Available tools:
- search: Search for documents by query. Returns document names, IDs, and snippets.
- read_document: Read the full text of a document by its ID.

Always use the search tool first to find relevant documents, then read specific
documents to get detailed information. Cite document names when providing information.

When you want to call a tool, respond with a JSON object in this exact format:
{"tool": "tool_name", "arguments": {"arg1": "value1"}}

Only output one tool call at a time. After receiving tool results, you can call another tool or provide your final answer."#;

/// Wrapper around mistral.rs Model
pub struct AgentModel {
    model: Arc<Model>,
}

impl AgentModel {
    /// Load a model from Hugging Face Hub
    pub async fn load(_cache_dir: &Path) -> Result<Self> {
        let model = TextModelBuilder::new(DEFAULT_MODEL_ID)
            .with_logging()
            .with_paged_attn(|| PagedAttentionMetaBuilder::default().build())?
            .build()
            .await?;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
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
            }],
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub fn add_user_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::User,
            content,
            tool_call_id: None,
        });
    }

    pub fn add_assistant_message(&mut self, content: String) {
        self.messages.push(Message {
            role: MessageRole::Assistant,
            content,
            tool_call_id: None,
        });
    }

    pub fn add_tool_result(&mut self, tool_call_id: String, content: String) {
        self.messages.push(Message {
            role: MessageRole::Tool,
            content,
            tool_call_id: Some(tool_call_id),
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

/// Run the agent loop
pub async fn run_agent_loop(
    model: &Arc<Model>,
    conversation: &mut Conversation,
    user_message: String,
    state: &crate::core::AppState,
    event_tx: mpsc::Sender<AgentEvent>,
    cancel_token: CancellationToken,
) -> Result<()> {
    conversation.add_user_message(user_message);

    for _iteration in 0..MAX_ITERATIONS {
        if cancel_token.is_cancelled() {
            return Ok(());
        }

        // Build request from conversation
        let mut request = RequestBuilder::new();

        for msg in &conversation.messages {
            let role = match msg.role {
                MessageRole::System => TextMessageRole::System,
                MessageRole::User => TextMessageRole::User,
                MessageRole::Assistant => TextMessageRole::Assistant,
                MessageRole::Tool => TextMessageRole::User, // Tool results sent as user messages
            };
            request = request.add_message(role, &msg.content);
        }

        // Send streaming request
        let mut stream = model.stream_chat_request(request).await?;

        // Collect response
        let mut full_response = String::new();

        while let Some(response) = stream.next().await {
            if cancel_token.is_cancelled() {
                return Ok(());
            }

            match response {
                Response::Chunk(chunk) => {
                    for choice in chunk.choices {
                        if let Some(delta) = choice.delta.content.as_ref() {
                            full_response.push_str(delta);
                            let _ = event_tx
                                .send(AgentEvent::TextDelta {
                                    content: delta.clone(),
                                })
                                .await;
                        }
                    }
                }
                Response::Done(_) => break,
                Response::InternalError(e) => {
                    let _ = event_tx
                        .send(AgentEvent::Error {
                            message: e.to_string(),
                        })
                        .await;
                    return Err(anyhow::anyhow!("Model error: {}", e));
                }
                Response::ValidationError(e) => {
                    let _ = event_tx
                        .send(AgentEvent::Error {
                            message: e.to_string(),
                        })
                        .await;
                    return Err(anyhow::anyhow!("Validation error: {}", e));
                }
                Response::ModelError(msg, _) | Response::CompletionModelError(msg, _) => {
                    let _ = event_tx
                        .send(AgentEvent::Error {
                            message: msg.clone(),
                        })
                        .await;
                    return Err(anyhow::anyhow!("Model error: {}", msg));
                }
                // Ignore other response types (completions, embeddings, speech, etc.)
                _ => {}
            }
        }

        conversation.add_assistant_message(full_response.clone());

        // Check if the response contains a tool call
        if let Some(tool_call) = parse_tool_call(&full_response) {
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
        } else {
            // No tool call, we're done
            let _ = event_tx.send(AgentEvent::Done).await;
            return Ok(());
        }
    }

    // Max iterations reached
    let _ = event_tx
        .send(AgentEvent::Error {
            message: "Maximum iterations reached".to_string(),
        })
        .await;

    Ok(())
}

/// Parse a tool call from the model's response
fn parse_tool_call(response: &str) -> Option<ToolCall> {
    // Look for JSON object with "tool" field
    // Try to find a JSON object in the response
    let trimmed = response.trim();

    // Try parsing the entire response as JSON first
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        return extract_tool_call_from_json(&v);
    }

    // Try to find JSON embedded in the response
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            if end > start {
                let json_str = &trimmed[start..=end];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    return extract_tool_call_from_json(&v);
                }
            }
        }
    }

    None
}

fn extract_tool_call_from_json(v: &serde_json::Value) -> Option<ToolCall> {
    let tool_name = v.get("tool")?.as_str()?;
    let arguments = v.get("arguments").cloned().unwrap_or(serde_json::json!({}));

    Some(ToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        name: tool_name.to_string(),
        arguments,
    })
}
