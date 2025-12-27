pub mod provider;
pub mod tools;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub use provider::{
    get_tool_definitions, ChatProvider, CompletedToolCall, CompletionResult, ProviderEvent,
    ToolDefinition,
};
pub use tools::{execute_tool, ToolCall, ToolResult};

/// Info about a collection for agent context
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    /// Number of documents in this collection
    #[serde(default)]
    pub document_count: usize,
    /// Total pages across all documents
    #[serde(default)]
    pub total_pages: usize,
}

/// Context for agent execution - holds state and per-request configuration
#[derive(Clone)]
pub struct AgentContext {
    pub state: crate::AppState,
    /// Collections to filter searches to (None = search all)
    pub collections: Option<Vec<CollectionInfo>>,
}

impl AgentContext {
    /// Get collection IDs for search filtering
    pub fn collection_ids(&self) -> Option<Vec<String>> {
        self.collections
            .as_ref()
            .map(|cols| cols.iter().map(|c| c.id.clone()).collect())
    }

    /// Get collection names for system prompt
    pub fn collection_names(&self) -> Option<Vec<String>> {
        self.collections
            .as_ref()
            .map(|cols| cols.iter().map(|c| c.name.clone()).collect())
    }
}

/// Base system prompt for the agent
const BASE_SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists investigate document collections.

Be concise. Answer in 2-4 sentences unless the user asks for more detail. Cite document names so findings are verifiable.

When answering questions:
1. Search to find relevant documents
2. Read documents to extract specific details
3. Cite sources (document name)
4. Note any gaps or contradictions worth pursuing"#;

/// Build system prompt with optional collection context
fn build_system_prompt(collections: Option<&[CollectionInfo]>) -> String {
    let mut prompt = BASE_SYSTEM_PROMPT.to_string();

    if let Some(cols) = collections {
        if !cols.is_empty() {
            prompt.push_str("\n\nYou are searching documents in:\n");
            for col in cols {
                // Format: "- Collection Name (X documents, Y pages)"
                let stats = if col.document_count > 0 || col.total_pages > 0 {
                    let doc_word = if col.document_count == 1 {
                        "document"
                    } else {
                        "documents"
                    };
                    let page_word = if col.total_pages == 1 {
                        "page"
                    } else {
                        "pages"
                    };
                    format!(
                        " ({} {}, {} {})",
                        col.document_count, doc_word, col.total_pages, page_word
                    )
                } else {
                    String::new()
                };
                prompt.push_str(&format!("- {}{}\n", col.name, stats));
            }
        }
    }

    prompt
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
        Self::with_system_prompt(id, BASE_SYSTEM_PROMPT.to_string())
    }

    /// Create a new conversation with a custom system prompt
    pub fn with_system_prompt(id: String, system_prompt: String) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            id,
            title: "New conversation".to_string(),
            messages: vec![Message {
                role: MessageRole::System,
                content: vec![ContentBlock::Text {
                    text: system_prompt,
                }],
            }],
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Create a new conversation with collection context
    pub fn with_collection_context(id: String, collections: Option<&[CollectionInfo]>) -> Self {
        let system_prompt = build_system_prompt(collections);
        Self::with_system_prompt(id, system_prompt)
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
const MAX_ITERATIONS: usize = 100;

/// Run the agent loop with structured tool calling
///
/// Uses the ChatProvider trait for LLM inference, allowing local or remote models.
pub async fn run_agent_loop(
    provider: &dyn ChatProvider,
    conversation: &mut Conversation,
    user_message: String,
    ctx: &AgentContext,
    event_tx: mpsc::Sender<AgentEvent>,
    cancel_token: CancellationToken,
) -> Result<()> {
    info!(
        conversation_id = %conversation.id,
        message_len = user_message.len(),
        provider = provider.provider_name(),
        model = provider.model_id(),
        "Starting agent loop"
    );
    conversation.add_user_message(user_message);

    // Get tool definitions
    let tools = get_tool_definitions();
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

        // Create channel for provider events
        let (provider_tx, mut provider_rx) = mpsc::channel::<ProviderEvent>(100);

        // Clone what we need for the provider call
        let messages = conversation.messages.clone();
        let tools_clone = tools.clone();
        let cancel_clone = cancel_token.clone();

        // Spawn provider streaming in background
        let provider_handle = {
            // We need to handle the provider lifetime carefully
            // Since we can't move provider into the spawn, we'll run it inline
            provider.stream_completion(&messages, &tools_clone, provider_tx, cancel_clone)
        };

        // Forward provider events to agent events while streaming
        let event_tx_clone = event_tx.clone();
        let forward_handle = tokio::spawn(async move {
            let mut text_started = false;
            while let Some(event) = provider_rx.recv().await {
                match event {
                    ProviderEvent::TextDelta(text) => {
                        if !text_started {
                            let _ = event_tx_clone
                                .send(AgentEvent::ContentBlockStart {
                                    block: ContentBlock::Text {
                                        text: String::new(),
                                    },
                                })
                                .await;
                            text_started = true;
                        }
                        let _ = event_tx_clone
                            .send(AgentEvent::ContentBlockDelta {
                                delta: ContentDelta::Text { text },
                            })
                            .await;
                    }
                    ProviderEvent::ToolCallStart { .. } => {
                        // Tool calls are emitted after completion
                    }
                    ProviderEvent::ToolCallDelta { .. } => {
                        // Tool call deltas are accumulated by the provider
                    }
                    ProviderEvent::ToolCallComplete { .. } => {
                        // Will be processed after completion
                    }
                    ProviderEvent::Done => {
                        if text_started {
                            let _ = event_tx_clone.send(AgentEvent::ContentBlockStop).await;
                        }
                    }
                    ProviderEvent::Error(msg) => {
                        let _ = event_tx_clone
                            .send(AgentEvent::Error { message: msg })
                            .await;
                    }
                }
            }
            text_started
        });

        // Wait for provider to complete
        let result = provider_handle.await?;
        let _ = forward_handle.await;

        debug!(
            text_len = result.text.len(),
            tool_calls = result.tool_calls.len(),
            "Received model response"
        );

        // Build content blocks from result
        let mut content_blocks = Vec::new();
        if !result.text.is_empty() {
            content_blocks.push(ContentBlock::Text { text: result.text });
        }

        // Check for tool calls
        if !result.tool_calls.is_empty() {
            info!(
                tool_count = result.tool_calls.len(),
                "Model requested tool calls"
            );

            // Process each tool call
            for tc in &result.tool_calls {
                info!(
                    tool_name = %tc.name,
                    tool_id = %tc.id,
                    "Executing tool"
                );
                debug!(arguments = %tc.arguments, "Tool arguments");

                // Emit ToolUse block
                let tool_use_block = ContentBlock::ToolUse {
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
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
                    id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                };
                let tool_result = execute_tool(&tool_call, ctx).await;

                if tool_result.is_error {
                    warn!(
                        tool_name = %tool_call.name,
                        error = %tool_result.content,
                        "Tool execution failed"
                    );
                } else {
                    debug!(
                        tool_name = %tool_call.name,
                        result_len = tool_result.content.len(),
                        "Tool execution succeeded"
                    );
                }

                // Emit ToolResult block
                let tool_result_block = ContentBlock::ToolResult {
                    tool_use_id: tc.id.clone(),
                    content: tool_result.content,
                    is_error: tool_result.is_error,
                };
                let _ = event_tx
                    .send(AgentEvent::ContentBlockStart {
                        block: tool_result_block.clone(),
                    })
                    .await;
                let _ = event_tx.send(AgentEvent::ContentBlockStop).await;
                content_blocks.push(tool_result_block);
            }

            // Store assistant message with all content blocks
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
