//! Rig-based agent runtime.
//!
//! [`send_message`] drives a rig agent over the document tools
//! ([`crate::tools`]) with [`crate::memory::FileConversationMemory`] persistence,
//! streaming every [`MultiTurnStreamItem`](rig::agent::MultiTurnStreamItem)
//! to the caller as JSON. A conversation belongs to one collection, fixed at
//! creation; that collection's name is attached as a run document on every
//! turn so the model knows what its tools cover.

use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use iroh_docs::NamespaceId;
use rig::agent::AgentBuilder;
use rig::completion::{CompletionRequest, Document, Message};
use rig_core::{operation::Completion, DynModel};
use tokio_util::sync::CancellationToken;

use crate::config::Settings;
use crate::providers;
use crate::tools::{self, ToolState};
use crate::AppState;

/// System prompt for the research agent. Stable across the life of the app —
/// the conversation's collection arrives as a run document on every turn,
/// not baked into the preamble.
pub const BASE_SYSTEM_PROMPT: &str = r#"You are a research assistant helping journalists investigate document collections.

Your search and list tools are filtered to the conversation's collection — the "active collection" described in the context documents. Results outside that collection do not exist for you.

Be concise. Answer in 2-4 sentences unless the user asks for more detail. Cite document names so findings are verifiable.

When answering questions:
1. Search to find relevant documents
2. Read documents to extract specific details
3. Cite sources (document name)
4. Note any gaps or contradictions worth pursuing"#;

/// Prompt used to summarize a conversation into a short title.
const TITLE_PROMPT: &str = "Summarize this conversation in a short title of at most 50 characters. Output ONLY the title, nothing else.";

/// Maximum tool-call iterations per user message.
const MAX_TURNS: usize = 100;

/// The text of the scope document attached to every agent run.
fn format_scope_document(collection_name: &str) -> String {
    format!(
        "You are working in collection \"{collection_name}\". Your search and list tools only cover this collection."
    )
}

/// Send a user message to a conversation and stream the agent's response.
///
/// Every stream item is forwarded to `on_event` as the serialized
/// [`MultiTurnStreamItem`](rig::agent::MultiTurnStreamItem). A stream error
/// ends the run and is returned as `Err`, so the caller owns the single
/// error event on the channel. Cancelling `cancel` aborts the run (dropping
/// the stream) and returns `Ok(())`.
pub async fn send_message(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    message: String,
    on_event: impl Fn(serde_json::Value) + Send + 'static,
    cancel: CancellationToken,
) -> Result<()> {
    let meta = state
        .memory
        .load_meta(collection_id, conversation_id)
        .await
        .map_err(|e| anyhow!("failed to load conversation meta: {e}"))?
        .ok_or_else(|| anyhow!("conversation not found: {collection_id}/{conversation_id}"))?;

    let namespace_id = NamespaceId::from_str(collection_id)
        .map_err(|_| anyhow!("invalid collection id: {collection_id}"))?;
    let collection_name = state
        .storage
        .read()
        .await
        .get_collection_metadata(namespace_id)
        .await
        .context("failed to read collection metadata")?
        .map(|metadata| metadata.name)
        .ok_or_else(|| anyhow!("collection not found: {collection_id}"))?;

    let model = build_configured_model(state)?;
    run_agent(
        state,
        collection_id,
        conversation_id,
        &collection_name,
        meta.title.is_empty(),
        message,
        model,
        on_event,
        cancel,
    )
    .await
}

/// Read provider settings and construct the configured model handle.
fn build_configured_model(state: &AppState) -> Result<DynModel<Completion>> {
    let settings =
        Settings::load(&state.config.settings_file).context("failed to load settings")?;
    let provider = settings
        .provider
        .ok_or_else(|| anyhow!("no chat provider configured"))?;
    providers::build_model(&provider).map_err(|e| anyhow!("failed to build chat model: {e}"))
}

/// Build the fully-configured agent for one conversation run.
///
/// The tools own live handles to the index and storage plus the run's
/// collection.
fn build_agent(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    model: DynModel<Completion>,
) -> Result<rig::Agent> {
    let tool_state = ToolState::new(state, collection_id.to_string());
    Ok(AgentBuilder::new(model)
        .preamble(BASE_SYSTEM_PROMPT)
        .tool(tools::Search::new(tool_state.clone()))
        .tool(tools::ReadChunk::new(tool_state.clone()))
        .tool(tools::ListDocuments::new(tool_state.clone()))
        .tool(tools::GetCollectionTerms::new(tool_state))
        .memory(state.memory.store(collection_id, conversation_id)?)
        .conversation(conversation_id)
        .default_max_turns(MAX_TURNS)
        .build())
}

/// Drive the agent stream for one user message.
///
/// Split from [`send_message`] so tests can inject a mock model.
#[allow(clippy::too_many_arguments)]
async fn run_agent(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    collection_name: &str,
    needs_title: bool,
    message: String,
    model: DynModel<Completion>,
    on_event: impl Fn(serde_json::Value) + Send + 'static,
    cancel: CancellationToken,
) -> Result<()> {
    let agent = build_agent(state, collection_id, conversation_id, model.clone())?;
    let scope_document = Document {
        id: "active_collection".to_string(),
        text: format_scope_document(collection_name),
        additional_props: Default::default(),
    };
    let mut stream = agent.prompt(message).document(scope_document).stream();

    loop {
        tokio::select! {
            biased;
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
                    Err(e) => return Err(anyhow!("agent stream failed: {e}")),
                }
            }
        }
    }

    state
        .memory
        .touch(collection_id, conversation_id)
        .await
        .map_err(|e| anyhow!("failed to touch conversation: {e}"))?;

    // First real turn: replace the empty title with a generated one.
    if needs_title {
        let history = load_history(state, collection_id, conversation_id).await?;
        if history.len() >= 2 {
            if let Err(e) =
                generate_title_with(state, collection_id, conversation_id, &history, &model).await
            {
                tracing::warn!(conversation_id, error = %e, "title generation failed");
            }
        }
    }

    Ok(())
}

/// Load the message history for a conversation from memory.
async fn load_history(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
) -> Result<Vec<Message>> {
    state
        .memory
        .load(collection_id, conversation_id)
        .await
        .map_err(|e| anyhow!("failed to load conversation history: {e}"))
}

/// Title generation over an explicit model, so the agent run can reuse its
/// handle and tests can inject a mock.
async fn generate_title_with(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    history: &[Message],
    model: &DynModel<Completion>,
) -> Result<()> {
    let generated = model
        .call(CompletionRequest::new(Message::user(TITLE_PROMPT)).messages(history.iter().cloned()))
        .await
        .ok()
        .map(|response| response.text())
        .filter(|t| !t.trim().is_empty());

    let title = match generated {
        Some(t) => rig_memory::title_of(&t),
        None => first_user_text(history)
            .map(|t| rig_memory::title_of(&t))
            .unwrap_or_default(),
    };
    if title.is_empty() {
        return Ok(());
    }
    state
        .memory
        .set_title(collection_id, conversation_id, &title)
        .await
        .map_err(|e| anyhow!("failed to set conversation title: {e}"))
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
#[cfg(test)]
mod tests {
    use super::*;
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

    /// Create a real collection and a conversation belonging to it.
    /// Returns the collection id and name.
    async fn setup_conversation(
        state: &AppState,
        conversation_id: &str,
        collection_name: &str,
    ) -> (String, String) {
        let (namespace_id, metadata) = state
            .storage
            .write()
            .await
            .create_collection(collection_name)
            .await
            .unwrap();
        let collection_id = namespace_id.to_string();
        state
            .memory
            .create(&collection_id, conversation_id)
            .await
            .unwrap();
        (collection_id, metadata.name)
    }

    /// A streaming mock turn that answers with plain text.
    fn text_stream_turn(text: &str) -> Vec<MockStreamEvent> {
        vec![
            MockStreamEvent::text(text),
            MockStreamEvent::FinalResponse(mock_final(Usage::new())),
        ]
    }

    fn handle(model: MockCompletionModel) -> DynModel<Completion> {
        model.erase()
    }

    /// The text of the `active_collection` run document sent with the probe's
    /// first completion request.
    fn first_scope_text(probe: &MockCompletionModel) -> String {
        let requests = probe.requests();
        let request = requests.first().expect("one completion request");
        rig::test_utils::sent_documents(request)
            .into_iter()
            .find(|(id, _)| id == "active_collection")
            .map(|(_, text)| text)
            .expect("scope document injected as run context")
    }

    #[tokio::test]
    async fn send_message_happy_path_persists_and_emits() {
        let (_tmp, state) = test_state().await;
        let id = "conv-happy";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let events: std::sync::Arc<Mutex<Vec<serde_json::Value>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([text_stream_turn(
            "The reports mention rising sea levels.",
        )]));

        run_agent(
            &state,
            &collection_id,
            id,
            &name,
            true,
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
        let history = load_history(&state, &collection_id, id).await.unwrap();
        assert!(
            history.len() >= 2,
            "expected user + assistant, got {history:?}"
        );
        assert!(matches!(history.first(), Some(Message::User { .. })));
        assert!(matches!(history.last(), Some(Message::Assistant(_))));

        // Meta was touched and the empty title replaced (fallback: the first
        // user message).
        let meta = state
            .memory
            .load_meta(&collection_id, id)
            .await
            .unwrap()
            .unwrap();
        assert!(!meta.title.is_empty());
        assert!(meta.title.starts_with("What do the reports say?"));
    }

    #[tokio::test]
    async fn cancelled_before_first_poll_is_deterministic() {
        let (_tmp, state) = test_state().await;
        let id = "conv-cancel";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

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
            &collection_id,
            id,
            &name,
            true,
            "hello".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            cancel,
        )
        .await
        .unwrap();

        assert!(events.lock().unwrap().is_empty());
        let history = load_history(&state, &collection_id, id).await.unwrap();
        assert!(history.is_empty(), "cancelled run must not record messages");
    }

    #[tokio::test]
    async fn stream_error_returns_err_without_emitting() {
        let (_tmp, state) = test_state().await;
        let id = "conv-stream-error";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let events: std::sync::Arc<Mutex<Vec<serde_json::Value>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([vec![
            MockStreamEvent::error("provider exploded"),
        ]]));

        let err = run_agent(
            &state,
            &collection_id,
            id,
            &name,
            true,
            "hello".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("provider exploded"));
        assert!(events.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn run_injects_scope_document_naming_the_collection() {
        let (_tmp, state) = test_state().await;
        let id = "conv-scope";
        let (collection_id, name) = setup_conversation(&state, id, "Budget Filings").await;

        let probe = MockCompletionModel::from_stream_turns([text_stream_turn("ok")]);
        let first_probe = probe.clone();
        run_agent(
            &state,
            &collection_id,
            id,
            &name,
            true,
            "summarize".into(),
            handle(probe),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let scope = first_scope_text(&first_probe);
        assert!(
            scope.contains(&name),
            "scope text names the conversation's collection: {scope}"
        );
    }

    #[tokio::test]
    async fn send_message_errors_for_unknown_conversation_and_collection() {
        let (_tmp, state) = test_state().await;
        // A valid collection id that storage does not know.
        let collection_id = iroh_docs::NamespaceSecret::from_bytes(&[7; 32])
            .id()
            .to_string();

        // Missing conversation.
        let err = send_message(
            &state,
            &collection_id,
            "nope",
            "hi".into(),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("conversation not found"));

        // A conversation whose collection is gone.
        let id = "conv-orphan";
        state.memory.create(&collection_id, id).await.unwrap();
        let err = send_message(
            &state,
            &collection_id,
            id,
            "hi".into(),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
        // Storage reports a missing namespace rather than `Ok(None)`, so the
        // lookup fails instead of reaching the explicit not-found branch.
        assert!(err.to_string().contains("collection metadata"), "{err:#}");
    }

    #[tokio::test]
    async fn title_generation_uses_model_output() {
        let (_tmp, state) = test_state().await;
        let id = "conv-title";
        let (collection_id, _) = setup_conversation(&state, id, "Climate Reports").await;

        let history = vec![Message::user("What is the total carbon budget?")];
        let model = handle(MockCompletionModel::text("Carbon Budget Question"));

        generate_title_with(&state, &collection_id, id, &history, &model)
            .await
            .unwrap();

        let meta = state
            .memory
            .load_meta(&collection_id, id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(meta.title, "Carbon Budget Question");
    }

    #[tokio::test]
    async fn title_generation_falls_back_to_first_user_message() {
        let (_tmp, state) = test_state().await;
        let id = "conv-title-fallback";
        let (collection_id, _) = setup_conversation(&state, id, "Climate Reports").await;

        let long_question = "Why did the agency reject the permit application for the coastal "
            .to_string()
            + "development project in the northern sector despite the environmental ";
        let history = vec![Message::user(long_question.clone())];
        // A mock with no scripted turn makes the completion fail → fallback.
        let model = handle(MockCompletionModel::from_turns([]));

        generate_title_with(&state, &collection_id, id, &history, &model)
            .await
            .unwrap();

        let meta = state
            .memory
            .load_meta(&collection_id, id)
            .await
            .unwrap()
            .unwrap();
        assert!(!meta.title.is_empty());
        assert_eq!(meta.title.chars().count(), 60);
        assert!(meta.title.ends_with('…'));
        assert!(long_question.starts_with(meta.title.trim_end_matches('…')));
    }
}
