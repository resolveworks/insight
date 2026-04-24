pub mod tools;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

pub use crate::provider::{
    get_tool_definitions, ChatProvider, CompletedToolCall, CompletionResult, ProviderEvent,
    ToolDefinition,
};
pub use tools::{execute_tool, ToolCall, ToolResult};

// Re-export CollectionInfo from crate root for convenience
pub use crate::CollectionInfo;

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

/// Base system prompt for the agent. Stable across the life of the app — any
/// per-chat state (e.g. active collection scope) is carried as `Context`
/// messages in the transcript, not baked into the system prompt.
const BASE_SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists investigate document collections.

Your search and list tools are filtered to a user-selected set of collections — the "active collections." Messages tagged `[session]` are out-of-band updates about session state, not user instructions. When the active collections change mid-conversation, prior tool results reflect the previous set.

Be concise. Answer in 2-4 sentences unless the user asks for more detail. Cite document names so findings are verifiable.

When answering questions:
1. Search to find relevant documents
2. Read documents to extract specific details
3. Cite sources (document name)
4. Note any gaps or contradictions worth pursuing"#;

/// Render a single collection line like "Climate Reports (10 documents, 250 pages)"
/// — shared between breadcrumb formatting and any UI mirror.
fn format_collection_entry(c: &CollectionInfo) -> String {
    if c.document_count == 0 && c.total_pages == 0 {
        return c.name.clone();
    }
    let doc_word = if c.document_count == 1 {
        "document"
    } else {
        "documents"
    };
    let page_word = if c.total_pages == 1 { "page" } else { "pages" };
    format!(
        "{} ({} {}, {} {})",
        c.name, c.document_count, doc_word, c.total_pages, page_word
    )
}

/// Format the human-readable text of an active-collections breadcrumb.
fn format_scope_breadcrumb(collections: &[CollectionInfo], first_time: bool) -> String {
    if collections.is_empty() {
        return "Active collections cleared — searches will return nothing until one is selected."
            .to_string();
    }
    let list = collections
        .iter()
        .map(format_collection_entry)
        .collect::<Vec<_>>()
        .join(", ");
    if first_time {
        format!("Active collections: {}", list)
    } else {
        format!(
            "Active collections changed to: {}. Earlier tool results used the previous set.",
            list
        )
    }
}

/// Wrap a Context-role message's raw text for delivery to an LLM.
///
/// Providers generally only accept a single leading system message, so
/// breadcrumb messages are shipped as user-role notes. The `[session]` tag
/// makes it obvious to the model that the note is out-of-band metadata rather
/// than something the user typed.
pub(crate) fn render_context_message(text: &str) -> String {
    format!("[session] {}", text)
}

fn collections_equivalent(a: &[CollectionInfo], b: &[CollectionInfo]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let a_ids: std::collections::HashSet<&str> = a.iter().map(|c| c.id.as_str()).collect();
    b.iter().all(|c| a_ids.contains(c.id.as_str()))
}

/// Message role in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    System,
    /// Session-scoped state note (e.g. collection scope change) inserted into
    /// the transcript so both the model and the persisted conversation record
    /// when context shifted. Providers render it inline as a tagged note.
    Context,
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

impl Message {
    /// Concatenate all text blocks in this message. Used by provider adapters
    /// to flatten system and context messages into a single payload string.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let ContentBlock::Text { text } = block {
                out.push_str(text);
            }
        }
        out
    }
}

/// A conversation with message history
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub messages: Vec<Message>,
    pub created_at: String,
    pub updated_at: String,
    /// Active collection scope for this conversation. Mutated via
    /// [`Conversation::set_collections`], which also appends a breadcrumb
    /// message to the transcript so the model can see the change.
    #[serde(default)]
    pub collections: Vec<CollectionInfo>,
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
            collections: Vec::new(),
        }
    }

    /// Update the active collection scope. When the set actually changes AND
    /// the conversation has already started (i.e. the user has sent at least
    /// one message), records a `Context`-role breadcrumb so the model sees
    /// that scope shifted.
    ///
    /// Breadcrumbs collapse: if the last message in the transcript is already
    /// a session breadcrumb (a burst of filter changes with no user message
    /// between them), its text is replaced rather than appending a new one.
    ///
    /// Before the first user message, scope mutations are silent —
    /// [`Conversation::add_user_message`] attaches a one-shot prelude on the
    /// first message instead. Returns `true` when something changed.
    pub fn set_collections(&mut self, collections: Vec<CollectionInfo>) -> bool {
        if collections_equivalent(&self.collections, &collections) {
            return false;
        }

        let first_time = self.collections.is_empty();
        self.collections = collections;
        self.touch();

        if !self.has_user_message() {
            return true;
        }

        let text = format_scope_breadcrumb(&self.collections, first_time);

        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Context {
                last.content = vec![ContentBlock::Text { text }];
                return true;
            }
        }

        self.messages.push(Message {
            role: MessageRole::Context,
            content: vec![ContentBlock::Text { text }],
        });
        true
    }

    fn has_user_message(&self) -> bool {
        self.messages.iter().any(|m| m.role == MessageRole::User)
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
        // The first user message carries a one-shot scope prelude so the model
        // sees the active collection scope without a transcript full of
        // pre-conversation filter-fiddling breadcrumbs.
        if !self.has_user_message() && !self.collections.is_empty() {
            let scope_text = format_scope_breadcrumb(&self.collections, true);
            self.messages.push(Message {
                role: MessageRole::Context,
                content: vec![ContentBlock::Text { text: scope_text }],
            });
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== CollectionInfo Tests ====================

    #[test]
    fn test_collection_info_serialization() {
        let info = CollectionInfo {
            id: "col_123".to_string(),
            name: "Research Papers".to_string(),
            document_count: 42,
            total_pages: 500,
            created_at: Some("2024-01-15T10:30:00Z".to_string()),
        };

        let json = serde_json::to_string(&info).unwrap();
        let parsed: CollectionInfo = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "col_123");
        assert_eq!(parsed.name, "Research Papers");
        assert_eq!(parsed.document_count, 42);
        assert_eq!(parsed.total_pages, 500);
        assert_eq!(parsed.created_at, Some("2024-01-15T10:30:00Z".to_string()));
    }

    #[test]
    fn test_collection_info_default_counts() {
        // Test that document_count and total_pages default to 0
        let json = r#"{"id": "col_1", "name": "Test"}"#;
        let parsed: CollectionInfo = serde_json::from_str(json).unwrap();

        assert_eq!(parsed.document_count, 0);
        assert_eq!(parsed.total_pages, 0);
    }

    // ==================== AgentContext Tests ====================

    #[test]
    fn test_agent_context_collection_ids_none() {
        // We can't easily create AppState in sync tests, so test the logic directly
        let collections: Option<Vec<CollectionInfo>> = None;
        let ids = collections
            .as_ref()
            .map(|cols| cols.iter().map(|c| c.id.clone()).collect::<Vec<_>>());

        assert!(ids.is_none());
    }

    #[test]
    fn test_agent_context_collection_ids_some() {
        let collections = Some(vec![
            CollectionInfo {
                id: "col_1".to_string(),
                name: "First".to_string(),
                document_count: 0,
                total_pages: 0,
                created_at: None,
            },
            CollectionInfo {
                id: "col_2".to_string(),
                name: "Second".to_string(),
                document_count: 0,
                total_pages: 0,
                created_at: None,
            },
        ]);

        let ids: Option<Vec<String>> = collections
            .as_ref()
            .map(|cols| cols.iter().map(|c| c.id.clone()).collect());

        assert_eq!(ids, Some(vec!["col_1".to_string(), "col_2".to_string()]));
    }

    #[test]
    fn test_agent_context_collection_names() {
        let collections = Some(vec![
            CollectionInfo {
                id: "col_1".to_string(),
                name: "Research".to_string(),
                document_count: 0,
                total_pages: 0,
                created_at: None,
            },
            CollectionInfo {
                id: "col_2".to_string(),
                name: "Finance".to_string(),
                document_count: 0,
                total_pages: 0,
                created_at: None,
            },
        ]);

        let names: Option<Vec<String>> = collections
            .as_ref()
            .map(|cols| cols.iter().map(|c| c.name.clone()).collect());

        assert_eq!(
            names,
            Some(vec!["Research".to_string(), "Finance".to_string()])
        );
    }

    // ==================== Scope breadcrumb Tests ====================

    #[test]
    fn test_format_scope_breadcrumb_first_time_with_stats() {
        let collections = vec![
            CollectionInfo {
                id: "col_1".to_string(),
                name: "Climate Reports".to_string(),
                document_count: 10,
                total_pages: 250,
                created_at: None,
            },
            CollectionInfo {
                id: "col_2".to_string(),
                name: "Financial Data".to_string(),
                document_count: 5,
                total_pages: 100,
                created_at: None,
            },
        ];
        let text = format_scope_breadcrumb(&collections, true);

        assert!(text.starts_with("Active collections:"));
        assert!(text.contains("Climate Reports (10 documents, 250 pages)"));
        assert!(text.contains("Financial Data (5 documents, 100 pages)"));
    }

    #[test]
    fn test_format_scope_breadcrumb_update_warns_about_previous_scope() {
        let collections = vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Energy".to_string(),
            document_count: 3,
            total_pages: 40,
            created_at: None,
        }];
        let text = format_scope_breadcrumb(&collections, false);

        assert!(text.starts_with("Active collections changed to:"));
        assert!(text.contains("Earlier tool results used the previous set."));
    }

    #[test]
    fn test_format_scope_breadcrumb_singular_counts() {
        let collections = vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Solo".to_string(),
            document_count: 1,
            total_pages: 1,
            created_at: None,
        }];
        let text = format_scope_breadcrumb(&collections, true);

        assert!(text.contains("1 document,"));
        assert!(text.contains("1 page"));
        assert!(!text.contains("1 documents"));
        assert!(!text.contains("1 pages"));
    }

    #[test]
    fn test_format_scope_breadcrumb_drops_zero_stats() {
        let collections = vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Empty".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }];
        let text = format_scope_breadcrumb(&collections, true);

        assert!(text.contains("Empty"));
        assert!(!text.contains("0 documents"));
    }

    #[test]
    fn test_format_scope_breadcrumb_cleared_scope() {
        let text = format_scope_breadcrumb(&[], false);
        assert!(text.starts_with("Active collections cleared"));
    }

    // ==================== MessageRole Tests ====================

    #[test]
    fn test_message_role_serialization() {
        assert_eq!(
            serde_json::to_string(&MessageRole::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Context).unwrap(),
            "\"context\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&MessageRole::Assistant).unwrap(),
            "\"assistant\""
        );
    }

    #[test]
    fn test_message_role_deserialization() {
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"system\"").unwrap(),
            MessageRole::System
        );
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"context\"").unwrap(),
            MessageRole::Context
        );
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"user\"").unwrap(),
            MessageRole::User
        );
        assert_eq!(
            serde_json::from_str::<MessageRole>("\"assistant\"").unwrap(),
            MessageRole::Assistant
        );
    }

    // ==================== ContentBlock Tests ====================

    #[test]
    fn test_content_block_text_serialization() {
        let block = ContentBlock::Text {
            text: "Hello world".to_string(),
        };
        let json = serde_json::to_string(&block).unwrap();

        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"Hello world\""));

        let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
        match parsed {
            ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("Expected Text block"),
        }
    }

    #[test]
    fn test_content_block_tool_use_serialization() {
        let block = ContentBlock::ToolUse {
            id: "call_123".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "climate"}),
        };
        let json = serde_json::to_string(&block).unwrap();

        assert!(json.contains("\"type\":\"tool_use\""));
        assert!(json.contains("\"id\":\"call_123\""));
        assert!(json.contains("\"name\":\"search\""));

        let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
        match parsed {
            ContentBlock::ToolUse {
                id,
                name,
                arguments,
            } => {
                assert_eq!(id, "call_123");
                assert_eq!(name, "search");
                assert_eq!(arguments["query"], "climate");
            }
            _ => panic!("Expected ToolUse block"),
        }
    }

    #[test]
    fn test_content_block_tool_result_serialization() {
        let block = ContentBlock::ToolResult {
            tool_use_id: "call_123".to_string(),
            content: "Found 5 documents".to_string(),
            is_error: false,
        };
        let json = serde_json::to_string(&block).unwrap();

        assert!(json.contains("\"type\":\"tool_result\""));
        assert!(json.contains("\"tool_use_id\":\"call_123\""));
        assert!(json.contains("\"is_error\":false"));

        let parsed: ContentBlock = serde_json::from_str(&json).unwrap();
        match parsed {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "call_123");
                assert_eq!(content, "Found 5 documents");
                assert!(!is_error);
            }
            _ => panic!("Expected ToolResult block"),
        }
    }

    // ==================== Message Tests ====================

    #[test]
    fn test_message_serialization() {
        let message = Message {
            role: MessageRole::User,
            content: vec![ContentBlock::Text {
                text: "Hello".to_string(),
            }],
        };
        let json = serde_json::to_string(&message).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.role, MessageRole::User);
        assert_eq!(parsed.content.len(), 1);
    }

    #[test]
    fn test_message_multiple_blocks() {
        let message = Message {
            role: MessageRole::Assistant,
            content: vec![
                ContentBlock::Text {
                    text: "Searching...".to_string(),
                },
                ContentBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: serde_json::json!({}),
                },
            ],
        };
        let json = serde_json::to_string(&message).unwrap();
        let parsed: Message = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.content.len(), 2);
    }

    // ==================== Conversation Tests ====================

    #[test]
    fn test_conversation_new() {
        let conv = Conversation::new("conv_123".to_string());

        assert_eq!(conv.id, "conv_123");
        assert_eq!(conv.title, "New conversation");
        assert_eq!(conv.messages.len(), 1);
        assert_eq!(conv.messages[0].role, MessageRole::System);

        // Check system message contains base prompt
        match &conv.messages[0].content[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("research assistant"));
            }
            _ => panic!("Expected text block"),
        }
    }

    #[test]
    fn test_conversation_with_system_prompt() {
        let conv =
            Conversation::with_system_prompt("conv_1".to_string(), "Custom prompt".to_string());

        assert_eq!(conv.messages.len(), 1);
        match &conv.messages[0].content[0] {
            ContentBlock::Text { text } => {
                assert_eq!(text, "Custom prompt");
            }
            _ => panic!("Expected text block"),
        }
    }

    #[test]
    fn test_conversation_set_collections_before_first_message_is_silent() {
        let mut conv = Conversation::new("conv_1".to_string());
        let starting_len = conv.messages.len();

        let changed = conv.set_collections(vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Test Collection".to_string(),
            document_count: 5,
            total_pages: 50,
            created_at: None,
        }]);

        // Scope was updated, but no breadcrumb yet — the first user message
        // will carry an implicit "Active collections:" prelude.
        assert!(changed);
        assert_eq!(conv.collections.len(), 1);
        assert_eq!(conv.messages.len(), starting_len);
    }

    #[test]
    fn test_conversation_first_user_message_injects_scope_prelude() {
        let mut conv = Conversation::new("conv_1".to_string());
        conv.set_collections(vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Test Collection".to_string(),
            document_count: 5,
            total_pages: 50,
            created_at: None,
        }]);

        let len_before = conv.messages.len();
        conv.add_user_message("What's in these docs?".to_string());

        // Prelude + user message
        assert_eq!(conv.messages.len(), len_before + 2);

        let prelude = &conv.messages[len_before];
        assert_eq!(prelude.role, MessageRole::Context);
        match &prelude.content[0] {
            ContentBlock::Text { text } => {
                assert!(text.starts_with("Active collections:"));
                assert!(text.contains("Test Collection"));
            }
            _ => panic!("Expected text block"),
        }

        let user_msg = conv.messages.last().unwrap();
        assert_eq!(user_msg.role, MessageRole::User);
    }

    #[test]
    fn test_conversation_multiple_scope_changes_before_first_message_leave_single_prelude() {
        let mut conv = Conversation::new("conv_1".to_string());

        conv.set_collections(vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "First".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.set_collections(vec![CollectionInfo {
            id: "col_2".to_string(),
            name: "Second".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.set_collections(vec![CollectionInfo {
            id: "col_3".to_string(),
            name: "Third".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);

        // No breadcrumbs should have been added yet.
        let context_count = conv
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Context)
            .count();
        assert_eq!(context_count, 0);

        conv.add_user_message("Hello".to_string());

        // Exactly one prelude, naming only the final selection.
        let context_msgs: Vec<_> = conv
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::Context)
            .collect();
        assert_eq!(context_msgs.len(), 1);
        match &context_msgs[0].content[0] {
            ContentBlock::Text { text } => {
                assert!(text.starts_with("Active collections:"));
                assert!(text.contains("Third"));
                assert!(!text.contains("First"));
                assert!(!text.contains("Second"));
            }
            _ => panic!("Expected text block"),
        }
    }

    #[test]
    fn test_conversation_set_collections_after_user_message_emits_breadcrumb() {
        let mut conv = Conversation::new("conv_1".to_string());
        conv.set_collections(vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "First".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.add_user_message("Hello".to_string());

        let changed = conv.set_collections(vec![CollectionInfo {
            id: "col_2".to_string(),
            name: "Second".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);

        assert!(changed);
        let last = conv.messages.last().unwrap();
        assert_eq!(last.role, MessageRole::Context);
        match &last.content[0] {
            ContentBlock::Text { text } => {
                assert!(text.starts_with("Active collections changed to:"));
                assert!(text.contains("Second"));
                assert!(text.contains("Earlier tool results used the previous set."));
            }
            _ => panic!("Expected text block"),
        }
    }

    #[test]
    fn test_conversation_consecutive_scope_changes_collapse_into_one_breadcrumb() {
        let mut conv = Conversation::new("conv_1".to_string());
        conv.set_collections(vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Alpha".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.add_user_message("Hello".to_string());
        conv.add_assistant_message(vec![ContentBlock::Text {
            text: "Hi there".to_string(),
        }]);

        // Three scope changes with no intervening user message.
        conv.set_collections(vec![CollectionInfo {
            id: "col_2".to_string(),
            name: "Bravo".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.set_collections(vec![CollectionInfo {
            id: "col_3".to_string(),
            name: "Charlie".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        conv.set_collections(vec![CollectionInfo {
            id: "col_4".to_string(),
            name: "Delta".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);

        // Only one breadcrumb trails the assistant turn, naming Delta.
        let tail_contexts: Vec<_> = conv
            .messages
            .iter()
            .rev()
            .take_while(|m| m.role == MessageRole::Context)
            .collect();
        assert_eq!(tail_contexts.len(), 1);
        match &tail_contexts[0].content[0] {
            ContentBlock::Text { text } => {
                assert!(text.contains("Delta"));
                assert!(!text.contains("Bravo"));
                assert!(!text.contains("Charlie"));
            }
            _ => panic!("Expected text block"),
        }

        // And a subsequent user message separates this breadcrumb from future
        // ones, so the next scope change appends rather than collapsing.
        conv.add_user_message("Another question".to_string());
        conv.add_assistant_message(vec![ContentBlock::Text {
            text: "Another answer".to_string(),
        }]);
        let len_before_next = conv.messages.len();
        conv.set_collections(vec![CollectionInfo {
            id: "col_5".to_string(),
            name: "Echo".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }]);
        assert_eq!(conv.messages.len(), len_before_next + 1);
    }

    #[test]
    fn test_conversation_set_collections_noop_when_unchanged() {
        let mut conv = Conversation::new("conv_1".to_string());
        let cols = vec![CollectionInfo {
            id: "col_1".to_string(),
            name: "Same".to_string(),
            document_count: 0,
            total_pages: 0,
            created_at: None,
        }];

        assert!(conv.set_collections(cols.clone()));
        conv.add_user_message("Hello".to_string());
        let len_after_first = conv.messages.len();

        // Re-setting with identical IDs should not emit a breadcrumb.
        assert!(!conv.set_collections(cols));
        assert_eq!(conv.messages.len(), len_after_first);
    }

    #[test]
    fn test_conversation_add_user_message() {
        let mut conv = Conversation::new("conv_1".to_string());
        let initial_updated = conv.updated_at.clone();

        // Small delay to ensure timestamp differs
        std::thread::sleep(std::time::Duration::from_millis(10));

        conv.add_user_message("Hello!".to_string());

        assert_eq!(conv.messages.len(), 2);
        assert_eq!(conv.messages[1].role, MessageRole::User);
        match &conv.messages[1].content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello!"),
            _ => panic!("Expected text block"),
        }

        // updated_at should have changed
        assert_ne!(conv.updated_at, initial_updated);
    }

    #[test]
    fn test_conversation_add_assistant_message() {
        let mut conv = Conversation::new("conv_1".to_string());

        conv.add_assistant_message(vec![
            ContentBlock::Text {
                text: "Response".to_string(),
            },
            ContentBlock::ToolUse {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({}),
            },
        ]);

        assert_eq!(conv.messages.len(), 2);
        assert_eq!(conv.messages[1].role, MessageRole::Assistant);
        assert_eq!(conv.messages[1].content.len(), 2);
    }

    #[test]
    fn test_conversation_generate_title_short() {
        let mut conv = Conversation::new("conv_1".to_string());
        conv.add_user_message("What is climate change?".to_string());

        conv.generate_title();

        assert_eq!(conv.title, "What is climate change?");
    }

    #[test]
    fn test_conversation_generate_title_long() {
        let mut conv = Conversation::new("conv_1".to_string());
        conv.add_user_message(
            "This is a very long message that exceeds fifty characters and should be truncated"
                .to_string(),
        );

        conv.generate_title();

        assert_eq!(conv.title.len(), 50);
        assert!(conv.title.ends_with("..."));
    }

    #[test]
    fn test_conversation_generate_title_no_user_message() {
        let mut conv = Conversation::new("conv_1".to_string());

        conv.generate_title();

        // Title should remain unchanged
        assert_eq!(conv.title, "New conversation");
    }

    #[test]
    fn test_conversation_serialization_roundtrip() {
        let mut conv = Conversation::new("conv_123".to_string());
        conv.add_user_message("Test message".to_string());
        conv.add_assistant_message(vec![ContentBlock::Text {
            text: "Response".to_string(),
        }]);

        let json = serde_json::to_string(&conv).unwrap();
        let parsed: Conversation = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "conv_123");
        assert_eq!(parsed.messages.len(), 3);
    }

    // ==================== ContentDelta Tests ====================

    #[test]
    fn test_content_delta_serialization() {
        let delta = ContentDelta::Text {
            text: "chunk".to_string(),
        };
        let json = serde_json::to_string(&delta).unwrap();

        assert!(json.contains("\"type\":\"text\""));
        assert!(json.contains("\"text\":\"chunk\""));
    }

    // ==================== AgentEvent Tests ====================

    #[test]
    fn test_agent_event_content_block_start() {
        let event = AgentEvent::ContentBlockStart {
            block: ContentBlock::Text {
                text: "Hi".to_string(),
            },
        };
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"content_block_start\""));
        assert!(json.contains("\"data\""));
    }

    #[test]
    fn test_agent_event_content_block_delta() {
        let event = AgentEvent::ContentBlockDelta {
            delta: ContentDelta::Text {
                text: "chunk".to_string(),
            },
        };
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"content_block_delta\""));
    }

    #[test]
    fn test_agent_event_content_block_stop() {
        let event = AgentEvent::ContentBlockStop;
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"content_block_stop\""));
    }

    #[test]
    fn test_agent_event_done() {
        let event = AgentEvent::Done;
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"done\""));
    }

    #[test]
    fn test_agent_event_error() {
        let event = AgentEvent::Error {
            message: "Something went wrong".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();

        assert!(json.contains("\"type\":\"error\""));
        assert!(json.contains("Something went wrong"));
    }

    // ==================== run_agent_loop Tests ====================

    /// Mock provider for testing the agent loop
    struct MockProvider {
        responses: std::sync::Mutex<Vec<CompletionResult>>,
    }

    impl MockProvider {
        fn new(responses: Vec<CompletionResult>) -> Self {
            Self {
                responses: std::sync::Mutex::new(responses),
            }
        }
    }

    impl crate::provider::Provider for MockProvider {
        fn provider_name(&self) -> &'static str {
            "mock"
        }

        fn model_id(&self) -> &str {
            "mock-model"
        }
    }

    #[async_trait::async_trait]
    impl ChatProvider for MockProvider {
        async fn stream_completion(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            event_tx: mpsc::Sender<ProviderEvent>,
            _cancel_token: CancellationToken,
        ) -> Result<CompletionResult> {
            // Get result while holding lock, then release before await
            let result = {
                let mut responses = self.responses.lock().unwrap();
                if responses.is_empty() {
                    CompletionResult::default()
                } else {
                    responses.remove(0)
                }
            };

            // Stream text if present
            if !result.text.is_empty() {
                let _ = event_tx
                    .send(ProviderEvent::TextDelta(result.text.clone()))
                    .await;
            }
            let _ = event_tx.send(ProviderEvent::Done).await;

            Ok(result)
        }
    }

    #[tokio::test]
    async fn test_run_agent_loop_simple_response() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _progress_rx) = crate::AppState::new(config).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let provider = MockProvider::new(vec![CompletionResult {
            text: "Hello! I can help with that.".to_string(),
            tool_calls: vec![],
        }]);

        let mut conversation = Conversation::new("test_conv".to_string());
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let cancel_token = CancellationToken::new();

        run_agent_loop(
            &provider,
            &mut conversation,
            "Hello".to_string(),
            &ctx,
            event_tx,
            cancel_token,
        )
        .await
        .unwrap();

        // Check conversation was updated
        assert_eq!(conversation.messages.len(), 3); // system + user + assistant

        // Check events were emitted
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }

        // Should have ContentBlockStart, ContentBlockDelta, ContentBlockStop, Done
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done)));
    }

    #[tokio::test]
    async fn test_run_agent_loop_cancellation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _progress_rx) = crate::AppState::new(config).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let provider = MockProvider::new(vec![]);
        let mut conversation = Conversation::new("test_conv".to_string());
        let (event_tx, _event_rx) = mpsc::channel(100);
        let cancel_token = CancellationToken::new();

        // Cancel before running
        cancel_token.cancel();

        let result = run_agent_loop(
            &provider,
            &mut conversation,
            "Hello".to_string(),
            &ctx,
            event_tx,
            cancel_token,
        )
        .await;

        // Should complete without error (cancellation is graceful)
        assert!(result.is_ok());

        // Only user message should be added (cancelled before provider call completes)
        assert_eq!(conversation.messages.len(), 2); // system + user
    }

    #[tokio::test]
    async fn test_run_agent_loop_with_tool_call() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _progress_rx) = crate::AppState::new(config).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        // First response: tool call, second response: final text
        let provider = MockProvider::new(vec![
            CompletionResult {
                text: String::new(),
                tool_calls: vec![CompletedToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: serde_json::json!({"query": "test"}),
                }],
            },
            CompletionResult {
                text: "Based on my search, I found no results.".to_string(),
                tool_calls: vec![],
            },
        ]);

        let mut conversation = Conversation::new("test_conv".to_string());
        let (event_tx, mut event_rx) = mpsc::channel(100);
        let cancel_token = CancellationToken::new();

        run_agent_loop(
            &provider,
            &mut conversation,
            "Search for test".to_string(),
            &ctx,
            event_tx,
            cancel_token,
        )
        .await
        .unwrap();

        // Should have: system + user + assistant(tool call + result) + assistant(final)
        assert!(conversation.messages.len() >= 3);

        // Check that tool use and result blocks exist
        let has_tool_use = conversation.messages.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
        });
        let has_tool_result = conversation.messages.iter().any(|m| {
            m.content
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult { .. }))
        });

        assert!(has_tool_use, "Should have a ToolUse block");
        assert!(has_tool_result, "Should have a ToolResult block");

        // Collect events
        let mut events = Vec::new();
        while let Ok(event) = event_rx.try_recv() {
            events.push(event);
        }
        assert!(events.iter().any(|e| matches!(e, AgentEvent::Done)));
    }
}
