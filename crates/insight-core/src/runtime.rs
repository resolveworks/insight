//! Rig-based agent runtime.
//!
//! [`send_message`] drives a rig agent over the document tools
//! ([`crate::tools`]) with [`crate::memory::FileConversationMemory`] persistence,
//! streaming app-owned [`ChatEvent`]s built by [`RunTranscript`]. A conversation
//! belongs to one collection, fixed at creation; that collection's name and
//! most common terms are attached as run documents on every turn so the model
//! knows what its tools cover.

use std::collections::BTreeMap;
use std::str::FromStr;

use anyhow::{anyhow, Context, Result};
use futures::StreamExt;
use iroh_docs::NamespaceId;
use rig::agent::{AgentBuilder, MultiTurnStreamItem};
use rig::completion::{CompletionRequest, Document, Message};
use rig_core::{operation::Completion, DynModel};
use tokio_util::sync::CancellationToken;

use crate::chat::{from_messages, ChatEvent, RunTranscript, TranscriptItem, TranscriptKind};
use crate::config::Settings;
use crate::providers;
use crate::search::TermFrequency;
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

/// How many of the collection's most common terms the context document shows.
const COLLECTION_TERMS_LIMIT: usize = 50;

/// The text of the scope document attached to every agent run.
fn format_scope_document(collection_name: &str) -> String {
    format!(
        "You are working in collection \"{collection_name}\". Your search and list tools only cover this collection."
    )
}

/// The collection's term context, re-derived only after its documents change.
fn collection_terms_document(state: &AppState, collection_id: &str) -> Result<Document> {
    let terms_cache = state.index_worker.terms_cache();
    let mut cache = terms_cache.lock().expect("terms cache lock poisoned");
    let text = match cache.get(collection_id) {
        Some(text) => text.clone(),
        None => {
            // Refresh under the lock so a concurrent index write's invalidation
            // cannot be followed by inserting the snapshot it just invalidated.
            let terms = crate::search::get_collection_terms(
                &state.search,
                collection_id,
                COLLECTION_TERMS_LIMIT,
            )
            .context("failed to read collection terms")?;
            let text = format_collection_terms(&terms);
            cache.insert(collection_id.to_string(), text.clone());
            text
        }
    };
    Ok(Document {
        id: "collection_terms".to_string(),
        text,
        additional_props: Default::default(),
    })
}

/// Term counts grouped like `count: term term`, most frequent group first.
fn format_collection_terms(terms: &[TermFrequency]) -> String {
    if terms.is_empty() {
        return "The collection has no indexed terms yet.".to_string();
    }
    let mut groups = BTreeMap::<u64, Vec<&str>>::new();
    for term in terms {
        groups.entry(term.doc_count).or_default().push(&term.term);
    }
    let lines = groups
        .iter()
        .rev()
        .map(|(count, terms)| format!("{count}: {}", terms.join(" ")))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "The most common terms in the collection by document frequency, each count followed by the terms found in that many documents. Use them to judge what the collection covers; this is a sample, not the full index.\n\n{lines}"
    )
}

/// Send a user message to a conversation and stream the agent's response.
///
/// The run emits the user's item, then [`ChatEvent`]s as the agent produces
/// them, and ends with [`ChatEvent::RunFinished`] carrying the items this run
/// appended. That event is the commit: the turn's messages, its `updated`
/// time, and its generated title are all persisted before it is sent. A stream
/// error ends the run and is returned as `Err`, so the caller owns the single
/// failure event on the channel. Cancelling `cancel` aborts the run (dropping
/// the stream and any title generation), records nothing, and returns `Ok(())`
/// after an empty [`ChatEvent::RunFinished`].
pub async fn send_message(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    message: String,
    on_event: impl Fn(ChatEvent) + Send + 'static,
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
        .tool(tools::ReadDocument::new(tool_state.clone()))
        .tool(tools::ListDocuments::new(tool_state))
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
    on_event: impl Fn(ChatEvent) + Send + 'static,
    cancel: CancellationToken,
) -> Result<()> {
    let initial_history = load_history(state, collection_id, conversation_id).await?;
    let base = initial_history.len();
    on_event(ChatEvent::ItemUpserted {
        item: user_item(base, &message),
    });

    let agent = build_agent(state, collection_id, conversation_id, model.clone())?;
    let scope_document = Document {
        id: "active_collection".to_string(),
        text: format_scope_document(collection_name),
        additional_props: Default::default(),
    };
    let terms_document = collection_terms_document(state, collection_id)?;
    let mut stream = agent
        .prompt(message)
        .documents([scope_document, terms_document])
        .stream();
    let mut transcript = RunTranscript::new(base);
    let mut final_messages = None;

    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                // Dropping the stream aborts the run.
                on_event(ChatEvent::RunFinished { items: Vec::new() });
                return Ok(());
            }
            item = stream.next() => {
                let Some(item) = item else { break };
                match item {
                    Ok(item) => {
                        if let MultiTurnStreamItem::FinalResponse(response) = &item {
                            final_messages = Some(response.messages.clone());
                        }
                        for event in transcript.apply(item)? {
                            on_event(event);
                        }
                        if final_messages.is_some() {
                            break;
                        }
                    }
                    Err(e) => return Err(anyhow!("agent stream failed: {e}")),
                }
            }
        }
    }

    let final_messages =
        final_messages.ok_or_else(|| anyhow!("agent stream ended without a final response"))?;
    let items = from_messages(&final_messages, base);

    // Everything this turn persisted is durable before the commit event: the
    // client re-reads the conversation list when it arrives.
    state
        .memory
        .touch(collection_id, conversation_id)
        .await
        .map_err(|e| anyhow!("failed to touch conversation: {e}"))?;

    // First real turn: replace the empty title with a generated one. Cancelling
    // here leaves the title empty, which the next turn generates.
    if needs_title {
        let history = initial_history
            .into_iter()
            .chain(final_messages)
            .collect::<Vec<_>>();
        if history.len() >= 2 {
            let generated = tokio::select! {
                biased;
                _ = cancel.cancelled() => None,
                result = generate_title_with(
                    state,
                    collection_id,
                    conversation_id,
                    &history,
                    &model,
                ) => Some(result),
            };
            if let Some(Err(e)) = generated {
                tracing::warn!(conversation_id, error = %e, "title generation failed");
            }
        }
    }

    on_event(ChatEvent::RunFinished { items });

    Ok(())
}

/// The transcript item for a user message about to be sent.
fn user_item(base: usize, text: &str) -> TranscriptItem {
    TranscriptItem {
        key: format!("{base}-0"),
        kind: TranscriptKind::UserText,
        name: String::new(),
        text: text.to_string(),
        provisional: true,
        is_error: false,
        executing: false,
        streaming: false,
    }
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
    use crate::memory::{ConversationSummary, DEFAULT_TITLE};
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

    /// A streaming mock turn with reasoning and a tool call whose arguments
    /// arrive over several deltas.
    fn tool_call_stream_turn() -> Vec<MockStreamEvent> {
        vec![
            MockStreamEvent::reasoning_delta("I should search the collection."),
            MockStreamEvent::tool_call_name_delta("call-1", "search"),
            MockStreamEvent::tool_call_arguments_delta("call-1", "{\"query\":"),
            MockStreamEvent::tool_call_arguments_delta("call-1", "\"climate\""),
            MockStreamEvent::tool_call_arguments_delta("call-1", "}"),
            MockStreamEvent::tool_call_end("call-1"),
            MockStreamEvent::FinalResponse(mock_final(Usage::new())),
        ]
    }

    fn handle(model: MockCompletionModel) -> DynModel<Completion> {
        model.erase()
    }

    /// The text of one run document sent with the probe's first completion
    /// request.
    fn first_document_text(probe: &MockCompletionModel, id: &str) -> String {
        let requests = probe.requests();
        let request = requests.first().expect("one completion request");
        rig::test_utils::sent_documents(request)
            .into_iter()
            .find(|(document_id, _)| document_id == id)
            .map(|(_, text)| text)
            .expect("document injected as run context")
    }

    /// The text of the `active_collection` run document sent with the probe's
    /// first completion request.
    fn first_scope_text(probe: &MockCompletionModel) -> String {
        first_document_text(probe, "active_collection")
    }

    /// Send one message and return the collection-term context the model saw.
    async fn terms_sent(
        state: &AppState,
        collection_id: &str,
        conversation_id: &str,
        collection_name: &str,
    ) -> String {
        let probe = MockCompletionModel::from_stream_turns([text_stream_turn("ok")]);
        let requests = probe.clone();
        run_agent(
            state,
            collection_id,
            conversation_id,
            collection_name,
            false,
            "summarize".into(),
            handle(probe),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap();
        first_document_text(&requests, "collection_terms")
    }

    /// Collections that existed before startup are watched and their live
    /// sync is resumed, so a previously shared or imported collection syncs
    /// again after a restart.
    #[tokio::test]
    async fn startup_resumes_live_sync_for_existing_collections() {
        let (_tmp, state) = test_state().await;
        let (collection_id, _) = state
            .storage
            .write()
            .await
            .create_collection("Shared Before Restart")
            .await
            .unwrap();

        assert!(
            !state
                .storage
                .read()
                .await
                .collection_sharing_status(collection_id)
                .await
                .unwrap()
                .sync_running
        );

        state.restore_configs_from_settings().await.unwrap();

        assert!(
            state
                .storage
                .read()
                .await
                .collection_sharing_status(collection_id)
                .await
                .unwrap()
                .sync_running
        );
    }

    #[tokio::test]
    async fn send_message_happy_path_persists_and_emits() {
        let (_tmp, state) = test_state().await;
        let id = "conv-happy";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let events: std::sync::Arc<Mutex<Vec<ChatEvent>>> =
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

        // The run ends with its normalized transcript.
        let events = events.lock().unwrap().clone();
        assert!(matches!(
            events.first(),
            Some(ChatEvent::ItemUpserted { item })
                if item.kind == TranscriptKind::UserText && item.provisional
        ));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::RunFinished { items })
                if items.iter().any(|item| item.kind == TranscriptKind::AssistantText)
        ));

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

    /// The commit event is what the client re-reads the conversation list on:
    /// by then the generated title must already be what the list reports.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_conversation_list_shows_the_new_title_when_the_run_commits() {
        let (_tmp, state) = test_state().await;
        let id = "conv-commit";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let at_commit: std::sync::Arc<Mutex<Option<Vec<ConversationSummary>>>> =
            std::sync::Arc::new(Mutex::new(None));
        let observed = at_commit.clone();
        let memory = state.memory.clone();
        let list_collection = collection_id.clone();
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
            move |event| {
                if matches!(event, ChatEvent::RunFinished { .. }) {
                    let listed = tokio::task::block_in_place(|| {
                        tokio::runtime::Handle::current()
                            .block_on(memory.list_summaries(&list_collection))
                    });
                    *observed.lock().unwrap() = Some(listed.unwrap());
                }
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let listed = at_commit.lock().unwrap().clone().expect("run committed");
        assert_eq!(listed.len(), 1);
        assert_ne!(
            listed[0].title, DEFAULT_TITLE,
            "the title must be durable before the commit event"
        );
    }

    #[tokio::test]
    async fn tool_call_run_commits_the_same_transcript_it_streamed() {
        let (_tmp, state) = test_state().await;
        let id = "conv-tools";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let events: std::sync::Arc<Mutex<Vec<ChatEvent>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([
            tool_call_stream_turn(),
            text_stream_turn("The collection has no matching passages."),
        ]));

        run_agent(
            &state,
            &collection_id,
            id,
            &name,
            false,
            "What do the reports say?".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let events = events.lock().unwrap().clone();
        assert!(matches!(
            events.first(),
            Some(ChatEvent::ItemUpserted { item })
                if item.key == "0-0"
                    && item.kind == TranscriptKind::UserText
                    && item.provisional
        ));
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::ItemUpserted { item }
                    if item.kind == TranscriptKind::AssistantText && item.streaming
            )),
            "assistant text streams while it receives deltas"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::ItemUpserted { item }
                    if item.kind == TranscriptKind::ToolCall && item.executing
            )),
            "a committed tool call is executing until its result arrives"
        );

        let run_finished = events
            .iter()
            .rev()
            .find_map(|event| match event {
                ChatEvent::RunFinished { items } => Some(items),
                _ => None,
            })
            .expect("the run finishes with its transcript");

        let history = load_history(&state, &collection_id, id).await.unwrap();
        let stored = from_messages(&history, 0);
        assert_eq!(run_finished, &stored);
        assert!(run_finished.iter().all(|item| !item.provisional));
        assert!(
            run_finished
                .iter()
                .all(|item| !item.executing && !item.streaming),
            "nothing stays live after the run commits: {run_finished:?}"
        );

        let reasoning = run_finished
            .iter()
            .find(|item| item.kind == TranscriptKind::Reasoning)
            .expect("reasoning item");
        assert_eq!(reasoning.text, "I should search the collection.");

        let call = run_finished
            .iter()
            .find(|item| item.kind == TranscriptKind::ToolCall)
            .expect("tool call item");
        assert_eq!(call.key, "1-1");
        assert_eq!(call.name, "search");
        assert_eq!(call.text, "{\n  \"query\": \"climate\"\n}");

        let result = run_finished
            .iter()
            .find(|item| item.kind == TranscriptKind::ToolResult)
            .expect("tool result item");
        assert_eq!(result.key, "2-0");
        assert_eq!(result.name, "search");
        assert!(!result.is_error);
        assert!(!result.text.is_empty());

        let answer = run_finished
            .iter()
            .find(|item| item.kind == TranscriptKind::AssistantText)
            .expect("assistant answer");
        assert_eq!(answer.key, "3-0");
        assert_eq!(answer.text, "The collection has no matching passages.");
    }

    #[tokio::test]
    async fn a_second_run_continues_the_committed_keys() {
        let (_tmp, state) = test_state().await;
        let id = "conv-second-run";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        run_agent(
            &state,
            &collection_id,
            id,
            &name,
            false,
            "First question".into(),
            handle(MockCompletionModel::from_stream_turns([text_stream_turn(
                "First answer",
            )])),
            |_| {},
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let prior = load_history(&state, &collection_id, id).await.unwrap();
        let prior_items = from_messages(&prior, 0);
        let base = prior.len();
        assert_eq!(base, 2);

        let events: std::sync::Arc<Mutex<Vec<ChatEvent>>> =
            std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let model = handle(MockCompletionModel::from_stream_turns([
            tool_call_stream_turn(),
            text_stream_turn("Second answer"),
        ]));
        run_agent(
            &state,
            &collection_id,
            id,
            &name,
            false,
            "Second question".into(),
            model,
            move |event| sink.lock().unwrap().push(event),
            CancellationToken::new(),
        )
        .await
        .unwrap();

        let events = events.lock().unwrap().clone();
        let run_finished = events
            .iter()
            .rev()
            .find_map(|event| match event {
                ChatEvent::RunFinished { items } => Some(items.clone()),
                _ => None,
            })
            .expect("the run finishes with its transcript");

        let all = load_history(&state, &collection_id, id).await.unwrap();
        let all_items = from_messages(&all, 0);

        let mut combined = prior_items.clone();
        combined.extend(run_finished.iter().cloned());
        assert_eq!(combined, all_items);
        assert_eq!(&all_items[..prior_items.len()], prior_items.as_slice());

        assert_eq!(run_finished.first().unwrap().key, format!("{base}-0"));
        assert!(run_finished.iter().all(|item| {
            item.key
                .split_once('-')
                .expect("transcript keys are message-block")
                .0
                .parse::<usize>()
                .expect("message index is a number")
                >= base
        }));
    }

    #[tokio::test]
    async fn cancelled_before_first_poll_is_deterministic() {
        let (_tmp, state) = test_state().await;
        let id = "conv-cancel";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let cancel = CancellationToken::new();
        cancel.cancel();

        let events: std::sync::Arc<Mutex<Vec<ChatEvent>>> =
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

        let events = events.lock().unwrap().clone();
        assert!(matches!(
            events.first(),
            Some(ChatEvent::ItemUpserted { item })
                if item.kind == TranscriptKind::UserText
        ));
        assert!(matches!(
            events.last(),
            Some(ChatEvent::RunFinished { items }) if items.is_empty()
        ));
        let history = load_history(&state, &collection_id, id).await.unwrap();
        assert!(history.is_empty(), "cancelled run must not record messages");
    }

    #[tokio::test]
    async fn stream_error_returns_err_without_terminal_event() {
        let (_tmp, state) = test_state().await;
        let id = "conv-stream-error";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let events: std::sync::Arc<Mutex<Vec<ChatEvent>>> =
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
        let events = events.lock().unwrap().clone();
        assert!(!events.iter().any(|event| matches!(
            event,
            ChatEvent::RunFinished { .. } | ChatEvent::RunFailed { .. }
        )));
    }

    #[tokio::test]
    async fn run_seeds_terms_and_refreshes_them_after_indexing() {
        let (_tmp, state) = test_state().await;
        let id = "conv-terms";
        let (collection_id, name) = setup_conversation(&state, id, "Climate Reports").await;

        let empty = terms_sent(&state, &collection_id, id, &name).await;
        assert_eq!(empty, "The collection has no indexed terms yet.");

        let document = |document_id: &str, content: &str| crate::search::DocumentToIndex {
            id: crate::search::document_external_id(&collection_id, document_id),
            document_id: document_id.to_string(),
            document_name: format!("{document_id}.pdf"),
            content: content.to_string(),
            collection_id: collection_id.clone(),
        };
        state
            .index_worker
            .index_documents(vec![document("doc1", "climate")])
            .await
            .unwrap();
        let initial = terms_sent(&state, &collection_id, id, &name).await;
        let climate = initial
            .lines()
            .find(|line| line.split(' ').any(|term| term == "climate"))
            .expect("climate seeded");
        assert!(
            climate.starts_with("1: "),
            "term and document frequency seeded: {initial}"
        );

        state
            .index_worker
            .index_documents(vec![document("doc2", "climate budget")])
            .await
            .unwrap();
        let refreshed = terms_sent(&state, &collection_id, id, &name).await;
        let climate = refreshed
            .lines()
            .find(|line| line.split(' ').any(|term| term == "climate"))
            .expect("climate remains seeded");
        let budget = refreshed
            .lines()
            .find(|line| line.split(' ').any(|term| term == "budget"))
            .expect("new term seeded");
        assert!(
            climate.starts_with("2: ") && budget.starts_with("1: "),
            "indexing invalidates the previous snapshot: {refreshed}"
        );
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
