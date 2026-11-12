//! Conversation commands: lifecycle (list/load/start/delete) and message
//! sending with streamed agent events. Each conversation belongs to exactly
//! one collection.

use serde::Serialize;
use tauri::{ipc::Channel, State};
use tokio_util::sync::CancellationToken;

use super::CollectionId;
use crate::core::{
    from_messages, providers, runtime, AppState, ChatEvent, ConversationSummary, Settings,
    TranscriptItem, DEFAULT_TITLE,
};
use crate::error::{CommandError, CommandResult, ResultExt};

/// A conversation as seen by the frontend: its collection, title, and full
/// normalized transcript.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationView {
    pub id: String,
    pub collection_id: String,
    pub title: String,
    pub transcript: Vec<TranscriptItem>,
}

/// Load a conversation's transcript into a [`ConversationView`].
async fn conversation_view(
    state: &AppState,
    collection_id: &str,
    conversation_id: &str,
    title: String,
) -> Result<ConversationView, CommandError> {
    let messages = state
        .memory
        .load(collection_id, conversation_id)
        .await
        .map_err(|e| CommandError::internal(format!("failed to load messages: {e}")))?;
    Ok(ConversationView {
        id: conversation_id.to_string(),
        collection_id: collection_id.to_string(),
        title: if title.is_empty() {
            DEFAULT_TITLE.to_string()
        } else {
            title
        },
        transcript: from_messages(&messages, 0),
    })
}

/// List all saved conversations in a collection, most recently updated first.
#[tauri::command]
pub async fn list_conversations(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<Vec<ConversationSummary>> {
    state
        .memory
        .list_summaries(&collection_id.namespace().to_string())
        .await
        .storage_err()
}

/// Load a conversation by ID (404-style error when missing).
#[tauri::command]
pub async fn load_conversation(
    collection_id: CollectionId,
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<ConversationView> {
    let collection_id = collection_id.namespace().to_string();
    let meta = state
        .memory
        .load_meta(&collection_id, &conversation_id)
        .await
        .storage_err()?
        .ok_or_else(CommandError::conversation_not_found)?;
    conversation_view(&state, &collection_id, &conversation_id, meta.title).await
}

/// Whether the configured chat provider passes validation.
async fn provider_ready(state: &AppState) -> CommandResult<bool> {
    let settings = Settings::load(&state.config.settings_file).storage_err()?;
    Ok(settings
        .provider
        .as_ref()
        .is_some_and(|p| providers::build_model(p).is_ok()))
}

/// Start a new chat conversation in a collection.
#[tauri::command]
pub async fn start_chat(
    collection_id: CollectionId,
    state: State<'_, AppState>,
) -> CommandResult<ConversationView> {
    if !provider_ready(&state).await? {
        return Err(CommandError::provider_not_configured());
    }

    let namespace = collection_id.namespace();
    {
        let storage = state.storage.read().await;
        if !storage.collection_exists(namespace).await.storage_err()? {
            return Err(CommandError::collection_not_found());
        }
    }

    let collection_id = namespace.to_string();
    let id = uuid::Uuid::new_v4().to_string();
    let meta = state
        .memory
        .create(&collection_id, &id)
        .await
        .storage_err()?;

    tracing::info!("Started new chat conversation: {}/{}", collection_id, id);
    conversation_view(&state, &collection_id, &id, meta.title).await
}

/// Delete a conversation: cancel any in-flight generation, then remove it
/// from disk. Idempotent.
#[tauri::command]
pub async fn delete_conversation(
    collection_id: CollectionId,
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(
        "Deleting conversation {}/{}",
        collection_id.namespace(),
        conversation_id
    );

    if let Some(token) = state
        .active_generations
        .write()
        .await
        .remove(&conversation_id)
    {
        token.cancel();
    }

    state
        .memory
        .delete(&collection_id.namespace().to_string(), &conversation_id)
        .await
        .storage_err()?;
    Ok(())
}

/// Send a message to a conversation and stream the agent's response through
/// the caller's channel.
///
/// Each message is one app-owned [`ChatEvent`]. A stream error surfaces as a
/// final [`ChatEvent::RunFailed`] on the same channel; a successful or
/// cancelled run ends with [`ChatEvent::RunFinished`]. The command returns
/// immediately; the run continues in a spawned task until it completes,
/// errors, or is cancelled via [`cancel_generation`].
#[tauri::command]
pub async fn send_message(
    collection_id: CollectionId,
    conversation_id: String,
    message: String,
    on_event: Channel<ChatEvent>,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(
        "Sending message to conversation {}/{}: {}",
        collection_id.namespace(),
        conversation_id,
        message
    );

    let cancel_token = CancellationToken::new();
    state
        .active_generations
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    let state_clone = state.inner().clone();
    let collection_id = collection_id.namespace().to_string();
    let conv_id = conversation_id.clone();
    let error_channel = on_event.clone();

    tokio::spawn(async move {
        if let Err(e) = runtime::send_message(
            &state_clone,
            &collection_id,
            &conv_id,
            message,
            move |value| {
                let _ = on_event.send(value);
            },
            cancel_token,
        )
        .await
        {
            tracing::error!(
                conversation_id = %conv_id,
                error = %e,
                error_chain = ?e,
                "Agent run error"
            );
            // The run failed before or outside the stream: the frontend is
            // waiting on the channel, so surface the failure there instead of
            // leaving the chat stuck in its generating state.
            let _ = error_channel.send(ChatEvent::RunFailed {
                message: format!("Agent run failed: {e}"),
            });
        }

        // Cleanup: drop the cancel token so a later generation can register
        // a fresh one (and `delete_conversation` stops cancelling a dead run).
        state_clone
            .active_generations
            .write()
            .await
            .remove(&conv_id);
    });

    Ok(())
}

/// Cancel an in-progress generation.
#[tauri::command]
pub async fn cancel_generation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let generations = state.active_generations.read().await;
    if let Some(token) = generations.get(&conversation_id) {
        token.cancel();
        tracing::info!("Cancelled generation for conversation {}", conversation_id);
    }
    Ok(())
}
