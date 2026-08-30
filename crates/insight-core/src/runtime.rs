//! Rig-based agent runtime.
//!
//! [`send_message`] drives a rig agent over the document tools
//! ([`crate::tools`]) with [`crate::memory::FileConversationMemory`] persistence,
//! streaming every [`MultiTurnStreamItem`](rig::agent::MultiTurnStreamItem)
//! to the caller as JSON. The conversation's
//! collection scope is injected per turn as an `extra_context` document via
//! an [`AgentHook`] — non-sticky, so a mid-conversation scope change takes
//! effect on the very next model call without touching stored history.
//!
//! [`predict_next_message`] is a bare-model completion (no tools, no agent loop)
//! over the same provider settings.

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use futures::StreamExt;
use rig::agent::{
    AgentBuilder, AgentHook, CompletionCallAction, HookContext, ModelHandle, RequestPatch,
};
use rig::completion::{CompletionModel, Document, Message};
use rig::memory::ConversationMemory;
use rig::streaming::StreamingPrompt;
use tokio_util::sync::CancellationToken;

use crate::config::Settings;
use crate::memory::ConversationMeta;
use crate::providers;
use crate::tools::{CollectionTermsTool, ListDocumentsTool, ReadChunkTool, SearchTool, ToolDeps};
use crate::{AppState, CollectionInfo};

/// System prompt for the research agent. Stable across the life of the app —
/// the per-conversation collection scope arrives as an `extra_context`
/// document on every turn, not baked into the preamble.
pub const BASE_SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists investigate document collections.

Your search and list tools are filtered to a user-selected set of collections — the "active collections" described in the context documents. Results outside those collections do not exist for you.

Be concise. Answer in 2-4 sentences unless the user asks for more detail. Cite document names so findings are verifiable.

When answering questions:
1. Search to find relevant documents
2. Read documents to extract specific details
3. Cite sources (document name)
4. Note any gaps or contradictions worth pursuing"#;

/// The title a conversation starts with; still having it means no title has
/// been generated yet.
const DEFAULT_TITLE: &str = "New conversation";

/// Prompt used to summarize a conversation into a short title.
const TITLE_PROMPT: &str = "Summarize this conversation in a short title of at most 50 characters. Output ONLY the title, nothing else.";

/// Prompt used for tab-completion style next-message prediction.
const PREDICTION_PROMPT: &str = r#"Based on the conversation above, predict what the user is most likely to ask or say next.

Rules:
- Output ONLY the predicted message, nothing else
- Keep it concise (1-2 sentences max)
- Make it a natural follow-up question or statement
- If the assistant just answered a question, predict a likely follow-up
- If unsure, output nothing"#;

/// Maximum tool-call iterations per user message.
const MAX_TURNS: usize = 100;

/// Maximum characters of a generated title.
const TITLE_MAX_CHARS: usize = 50;

/// Maximum characters of a predicted next message.
const PREDICTION_MAX_CHARS: usize = 150;

/// How long a prediction may run before being abandoned.
const PREDICTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Render a collection line like "Climate Reports (10 documents, 250 pages)".
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

/// The text of the per-turn scope document injected by [`ScopeHook`].
fn format_scope_document(collections: &[CollectionInfo]) -> String {
    let list = collections
        .iter()
        .map(format_collection_entry)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Active collections: {}. Your search and list tools only cover these collections.",
        list
    )
}

/// Injects the conversation's active collection scope into every model call.
///
/// Registered before any other run logic so it applies to every turn of the
/// multi-turn loop. The patch is per-request and non-sticky: the scope
/// document never lands in stored memory, so a scope change mid-conversation
/// is picked up by the next call without contradicting earlier turns.
struct ScopeHook {
    collections: Vec<CollectionInfo>,
}

impl AgentHook for ScopeHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        _event: rig::agent::hook::CompletionCall<'_>,
    ) -> CompletionCallAction {
        CompletionCallAction::patch(RequestPatch::new().extra_context([Document {
            id: "active_collections".to_string(),
            text: format_scope_document(&self.collections),
            additional_props: Default::default(),
        }]))
    }
}

/// Extract the concatenated text of an assistant response's text parts.
fn response_text(choice: &[rig::completion::AssistantContent]) -> String {
    choice
        .iter()
        .filter_map(|c| match c {
            rig::completion::AssistantContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Truncate a string to `max` characters, appending "..." when cut.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 3).collect();
        format!("{}...", cut)
    }
}

/// Send a user message to a conversation and stream the agent's response.
///
/// Every stream item is forwarded to `on_event` as the serialized
/// [`MultiTurnStreamItem`](rig::agent::MultiTurnStreamItem); a stream error is surfaced as
/// `{"type":"streamError","message":...}` through the same channel and ends
/// the run. Cancelling `cancel` aborts the run (dropping the stream) and
/// returns `Ok(())`.
pub async fn send_message(
    state: &AppState,
    conversation_id: &str,
    message: String,
    on_event: impl Fn(serde_json::Value) + Send + 'static,
    cancel: CancellationToken,
) -> Result<()> {
    let meta = state
        .memory
        .load_meta(conversation_id)
        .await
        .map_err(|e| anyhow!("failed to load conversation meta: {e}"))?
        .ok_or_else(|| anyhow!("conversation not found: {conversation_id}"))?;
    if meta.collections.is_empty() {
        bail!("conversation has no collection scope: {conversation_id}");
    }

    let model = build_configured_model(state)?;
    run_agent(
        state,
        conversation_id,
        &meta,
        message,
        model,
        on_event,
        cancel,
    )
    .await
}

/// Read provider settings and construct the configured model handle.
fn build_configured_model(state: &AppState) -> Result<ModelHandle> {
    let settings = Settings::load(&state.config.settings_file);
    let provider = settings
        .provider
        .ok_or_else(|| anyhow!("no chat provider configured"))?;
    providers::build_model(&provider).map_err(|e| anyhow!("failed to build chat model: {e}"))
}

/// Build the fully-configured agent for one conversation run.
fn build_agent(state: &AppState, meta: &ConversationMeta, model: ModelHandle) -> rig::Agent {
    let deps = std::sync::Arc::new(ToolDeps::from_state(state));
    let scope = Some(meta.collections.clone());
    AgentBuilder::from_model_handle(model)
        .preamble(BASE_SYSTEM_PROMPT)
        .tool(SearchTool::new(deps.clone(), scope.clone()))
        .tool(ReadChunkTool::new(deps.clone()))
        .tool(ListDocumentsTool::new(deps.clone(), scope.clone()))
        .tool(CollectionTermsTool::new(deps, scope))
        .memory(state.memory.clone())
        .default_max_turns(MAX_TURNS)
        .add_hook(ScopeHook {
            collections: meta.collections.clone(),
        })
        .build()
}

/// Drive the agent stream for one user message.
///
/// Split from [`send_message`] so tests can inject a mock model.
async fn run_agent(
    state: &AppState,
    conversation_id: &str,
    meta: &ConversationMeta,
    message: String,
    model: ModelHandle,
    on_event: impl Fn(serde_json::Value) + Send + 'static,
    cancel: CancellationToken,
) -> Result<()> {
    let agent = build_agent(state, meta, model.clone());
    if cancel.is_cancelled() {
        return Ok(());
    }
    let mut stream = agent
        .stream_prompt(message)
        .conversation(conversation_id.to_string())
        .await;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                // Dropping the stream aborts the run.
                return Ok(());
            }
            item = stream.next() => {
                let Some(item) = item else { break };
                match item {
                    Ok(item) => {
                        on_event(serde_json::to_value(&item).context("serializing stream item")?)
                    }
                    Err(e) => {
                        // A stream error is terminal for the run; surface it
                        // through the event channel and stop.
                        on_event(serde_json::json!({
                            "type": "streamError",
                            "message": e.to_string(),
                        }));
                        return Ok(());
                    }
                }
            }
        }
    }

    state
        .memory
        .touch(conversation_id)
        .await
        .map_err(|e| anyhow!("failed to touch conversation: {e}"))?;

    // First real turn: replace the default title with a generated one.
    if meta.title == DEFAULT_TITLE {
        let history = load_history(state, conversation_id).await?;
        if history.len() >= 2 {
            if let Err(e) = generate_title_with(state, conversation_id, &history, &model).await {
                tracing::warn!(conversation_id, error = %e, "title generation failed");
            }
        }
    }

    Ok(())
}

/// Load the message history for a conversation from memory.
async fn load_history(state: &AppState, conversation_id: &str) -> Result<Vec<Message>> {
    state
        .memory
        .load(conversation_id)
        .await
        .map_err(|e| anyhow!("failed to load conversation history: {e}"))
}

/// Title generation over an explicit model, so the agent run can reuse its
/// handle and tests can inject a mock.
async fn generate_title_with(
    state: &AppState,
    conversation_id: &str,
    history: &[Message],
    model: &ModelHandle,
) -> Result<()> {
    let mut meta = state
        .memory
        .load_meta(conversation_id)
        .await
        .map_err(|e| anyhow!("failed to load conversation meta: {e}"))?
        .ok_or_else(|| anyhow!("conversation not found: {conversation_id}"))?;

    let generated = model
        .completion_request(Message::user(TITLE_PROMPT))
        .messages(history.iter().cloned())
        .send()
        .await
        .ok()
        .map(|response| response_text(&response.choice))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());

    let title = match generated {
        Some(t) => t,
        None => first_user_text(history).unwrap_or_default(),
    };
    if title.is_empty() {
        return Ok(());
    }
    meta.title = truncate_chars(&title, TITLE_MAX_CHARS);
    state
        .memory
        .update_meta(conversation_id, meta)
        .await
        .map_err(|e| anyhow!("failed to update conversation meta: {e}"))
}

/// Concatenated text of the first user message in `history`.
fn first_user_text(history: &[Message]) -> Option<String> {
    history.iter().find_map(|m| match m {
        Message::User { content } => Some(
            content
                .iter()
                .filter_map(|c| match c {
                    rig::message::UserContent::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        ),
        _ => None,
    })
}

/// Predict what the user might say next, for inline tab completion.
///
/// Returns `None` when there is not enough history, no provider is
/// configured, the request is cancelled, or it exceeds the 5s budget.
/// Non-fatal by design: a failed prediction simply completes nothing.
pub async fn predict_next_message(
    state: &AppState,
    conversation_id: &str,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    let history = load_history(state, conversation_id).await?;
    // Need at least one user + one assistant message for a meaningful
    // prediction.
    let has_assistant = history
        .iter()
        .any(|m| matches!(m, Message::Assistant { .. }));
    if history.len() < 2 || !has_assistant {
        return Ok(None);
    }
    let Ok(model) = build_configured_model(state) else {
        return Ok(None);
    };
    predict_with(&model, history, cancel).await
}

/// Prediction over an explicit model and history.
async fn predict_with(
    model: &ModelHandle,
    history: Vec<Message>,
    cancel: CancellationToken,
) -> Result<Option<String>> {
    let request = model
        .completion_request(Message::user(PREDICTION_PROMPT))
        .messages(history);

    let response = tokio::select! {
        _ = cancel.cancelled() => return Ok(None),
        result = tokio::time::timeout(PREDICTION_TIMEOUT, request.send()) => match result {
            Ok(Ok(response)) => response,
            Ok(Err(e)) => {
                tracing::debug!(error = %e, "prediction completion failed");
                return Ok(None);
            }
            Err(_) => {
                tracing::debug!("prediction timed out");
                return Ok(None);
            }
        },
    };

    let prediction = response_text(&response.choice).trim().to_string();
    if prediction.is_empty() {
        return Ok(None);
    }
    Ok(Some(truncate_chars(&prediction, PREDICTION_MAX_CHARS)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::ConversationMeta;
    use crate::providers::ProviderSettings;
    use rig::completion::Usage;
    use rig::test_utils::{mock_final, MockCompletionModel, MockStreamEvent};
    use std::sync::Mutex;

    /// A minimal AppState over a temp directory (real storage/search, no
    /// network).
    async fn test_state() -> (tempfile::TempDir, AppState) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            conversations_dir: temp_dir.path().join("conversations"),
            settings_file: temp_dir.path().join("settings.json"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _rx) = AppState::new(config).await.unwrap();
        (temp_dir, state)
    }

    /// Persist provider settings so `build_configured_model` succeeds offline.
    fn configure_dummy_provider(state: &AppState) {
        let settings = Settings {
            provider: Some(ProviderSettings {
                family: "openai".into(),
                model: "gpt-test".into(),
                api_key: Some("dummy".into()),
                base_url: Some("https://example.invalid".into()),
            }),
            api_keys: Default::default(),
        };
        settings.save(&state.config.settings_file).unwrap();
    }

    fn test_meta(id: &str) -> ConversationMeta {
        ConversationMeta {
            id: id.into(),
            title: DEFAULT_TITLE.into(),
            collections: vec![CollectionInfo {
                id: "abcdef1234567890".into(),
                name: "Climate Reports".into(),
                document_count: 3,
                total_pages: 42,
                created_at: None,
            }],
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    /// A streaming mock turn that answers with plain text.
    fn text_stream_turn(text: &str) -> Vec<MockStreamEvent> {
        vec![
            MockStreamEvent::text(text),
            MockStreamEvent::FinalResponse(mock_final(Usage::new())),
        ]
    }

    fn handle(model: MockCompletionModel) -> ModelHandle {
        ModelHandle::new(model)
    }

    #[tokio::test]
    async fn send_message_happy_path_persisters_and_emits() {
        let (_tmp, state) = test_state().await;
        let id = "conv-happy";
        state.memory.create(test_meta(id)).await.unwrap();

        let events: std::sync::Arc<Mutex<Vec<serde_json::Value>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([text_stream_turn(
            "The reports mention rising sea levels.",
        )]));

        run_agent(
            &state,
            id,
            &test_meta(id),
            "What do the reports say?".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        // Stream items were forwarded, ending with a final response.
        let events = events.lock().unwrap().clone();
        assert!(!events.is_empty());
        assert_eq!(
            events.last().unwrap().get("type").and_then(|t| t.as_str()),
            Some("finalResponse")
        );

        // Memory recorded both sides of the turn.
        let history = load_history(&state, id).await.unwrap();
        assert!(
            history.len() >= 2,
            "expected user + assistant, got {history:?}"
        );
        assert!(matches!(history.first(), Some(Message::User { .. })));
        assert!(matches!(history.last(), Some(Message::Assistant { .. })));

        // Meta was touched and the default title replaced (fallback: the
        // first user message, truncated).
        let meta = state.memory.load_meta(id).await.unwrap().unwrap();
        assert_ne!(meta.title, DEFAULT_TITLE);
        assert!(meta.title.starts_with("What do the reports say?"));
    }

    #[tokio::test]
    async fn send_message_cancelled_before_first_poll_records_nothing() {
        let (_tmp, state) = test_state().await;
        let id = "conv-cancel";
        state.memory.create(test_meta(id)).await.unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel();

        let events: std::sync::Arc<Mutex<Vec<serde_json::Value>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([text_stream_turn(
            "too late",
        )]));

        run_agent(
            &state,
            id,
            &test_meta(id),
            "hello".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            cancel,
        )
        .await
        .unwrap();

        assert!(events.lock().unwrap().is_empty());
        let history = load_history(&state, id).await.unwrap();
        assert!(history.is_empty(), "cancelled run must not record messages");
    }

    #[tokio::test]
    async fn scope_hook_injects_collection_document() {
        let (_tmp, state) = test_state().await;
        let id = "conv-scope";
        state.memory.create(test_meta(id)).await.unwrap();

        let mock = MockCompletionModel::from_stream_turns([text_stream_turn("ok")]);
        let probe = mock.clone();

        run_agent(
            &state,
            id,
            &test_meta(id),
            "summarize".into(),
            handle(mock),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let requests = probe.requests();
        let request = requests.first().expect("one completion request");
        let scope_doc = request
            .documents
            .iter()
            .find(|d| d.id == "active_collections")
            .expect("scope document injected via extra_context");
        assert!(
            scope_doc
                .text
                .contains("Climate Reports (3 documents, 42 pages)"),
            "scope text describes the active collections: {}",
            scope_doc.text
        );
    }

    #[tokio::test]
    async fn send_message_requires_scope_and_conversation() {
        let (_tmp, state) = test_state().await;
        configure_dummy_provider(&state);

        // Missing conversation.
        let err = send_message(
            &state,
            "nope",
            "hi".into(),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("not found"));

        // Empty scope.
        let mut meta = test_meta("conv-empty");
        meta.collections.clear();
        state.memory.create(meta).await.unwrap();
        let err = send_message(
            &state,
            "conv-empty",
            "hi".into(),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("scope"));
    }

    #[tokio::test]
    async fn title_generation_uses_model_output() {
        let (_tmp, state) = test_state().await;
        let id = "conv-title";
        state.memory.create(test_meta(id)).await.unwrap();

        let history = vec![Message::user("What is the total carbon budget?")];
        let model = handle(MockCompletionModel::text("Carbon Budget Question"));

        generate_title_with(&state, id, &history, &model)
            .await
            .unwrap();

        let meta = state.memory.load_meta(id).await.unwrap().unwrap();
        assert_eq!(meta.title, "Carbon Budget Question");
    }

    #[tokio::test]
    async fn title_generation_falls_back_to_first_user_message() {
        let (_tmp, state) = test_state().await;
        let id = "conv-title-fallback";
        state.memory.create(test_meta(id)).await.unwrap();

        let long_question = "Why did the agency reject the permit application for the coastal "
            .to_string()
            + "development project in the northern sector despite the environmental ";
        let history = vec![Message::user(long_question.clone())];
        // A mock with no scripted turn makes the completion fail → fallback.
        let model = handle(MockCompletionModel::from_turns([]));

        generate_title_with(&state, id, &history, &model)
            .await
            .unwrap();

        let meta = state.memory.load_meta(id).await.unwrap().unwrap();
        assert_ne!(meta.title, DEFAULT_TITLE);
        assert_eq!(meta.title.chars().count(), TITLE_MAX_CHARS);
        assert!(meta.title.ends_with("..."));
        assert!(long_question.starts_with(&meta.title[..TITLE_MAX_CHARS - 3]));
    }

    #[tokio::test]
    async fn predict_truncates_long_output() {
        let long = "x".repeat(PREDICTION_MAX_CHARS + 50);
        let model = handle(MockCompletionModel::text(long));
        let history = vec![Message::user("hi"), Message::assistant("hello")];

        let prediction = predict_with(&model, history, CancellationToken::new())
            .await
            .unwrap()
            .expect("prediction returned");
        assert_eq!(prediction.chars().count(), PREDICTION_MAX_CHARS);
        assert!(prediction.ends_with("..."));
    }

    #[tokio::test]
    async fn predict_requires_assistant_history() {
        let (_tmp, state) = test_state().await;
        configure_dummy_provider(&state);
        let id = "conv-predict";
        state.memory.create(test_meta(id)).await.unwrap();

        // Only a user message: no prediction.
        state
            .memory
            .append(id, vec![Message::user("hello")])
            .await
            .unwrap();
        assert_eq!(
            predict_next_message(&state, id, CancellationToken::new())
                .await
                .unwrap(),
            None
        );
    }
}
