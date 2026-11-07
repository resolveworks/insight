//! Conversation commands: lifecycle (list/load/start/delete), message
//! sending with streamed agent events, and next-message prediction. Each
//! conversation belongs to exactly one collection.

use rig_core::completion::Message;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use super::CollectionId;
use crate::core::{providers, runtime, AppState, ConversationSummary, Settings, DEFAULT_TITLE};
use crate::error::{CommandError, CommandResult, ResultExt};

/// A conversation as seen by the frontend: its collection, title, and full
/// rig message history.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationView {
    pub id: String,
    pub collection_id: String,
    pub title: String,
    pub messages: Vec<Message>,
}

/// Load the messages of a conversation into a [`ConversationView`].
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
        messages,
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
        storage
            .get_collection_metadata(namespace)
            .await
            .storage_err()?
            .ok_or_else(CommandError::collection_not_found)?;
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

/// Delete a conversation: cancel any in-flight generation or prediction,
/// then remove it from disk. Idempotent.
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
    if let Some(token) = state
        .active_predictions
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

/// Send a message to a conversation and stream the agent's response as
/// `agent-event-{id}` Tauri events.
///
/// Each event payload is one serialized rig `MultiTurnStreamItem` (JSON,
/// `type`-tagged, camelCase). Stream errors surface as a final
/// `{"type":"streamError","message":...}` payload on the same channel. The
/// command returns immediately; the run continues in a spawned task until it
/// completes, errors, or is cancelled via [`cancel_generation`].
#[tauri::command]
pub async fn send_message(
    collection_id: CollectionId,
    conversation_id: String,
    message: String,
    app: AppHandle,
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
    let app_handle = app.clone();
    let app_handle_error = app.clone();

    tokio::spawn(async move {
        let event_name = format!("agent-event-{}", conv_id);
        let emitter = move |value: serde_json::Value| {
            if let Err(e) = app_handle.emit(&event_name, &value) {
                tracing::error!("Failed to emit agent event: {}", e);
            }
        };

        if let Err(e) = runtime::send_message(
            &state_clone,
            &collection_id,
            &conv_id,
            message,
            emitter,
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
            // waiting on the event channel, so surface the failure there
            // instead of leaving the chat stuck in its generating state.
            let event_name = format!("agent-event-{}", conv_id);
            let _ = app_handle_error.emit(
                &event_name,
                &serde_json::json!({
                    "type": "streamError",
                    "message": format!("Agent run failed: {e}"),
                }),
            );
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

/// Predict what the user might say next (tab completion). Supersedes any
/// in-flight prediction for the same conversation.
#[tauri::command]
pub async fn predict_next_message(
    collection_id: CollectionId,
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    // Supersede any in-flight prediction for this conversation.
    {
        let predictions = state.active_predictions.read().await;
        if let Some(token) = predictions.get(&conversation_id) {
            token.cancel();
        }
    }

    let cancel_token = CancellationToken::new();
    state
        .active_predictions
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    let state_clone = state.inner().clone();
    let collection_id = collection_id.namespace().to_string();
    let result =
        runtime::predict_next_message(&state_clone, &collection_id, &conversation_id, cancel_token)
            .await;

    state
        .active_predictions
        .write()
        .await
        .remove(&conversation_id);

    // A failed prediction completes nothing; surface it as `None` like the
    // other non-fatal outcomes.
    match result {
        Ok(prediction) => Ok(prediction),
        Err(e) => {
            tracing::debug!("Prediction failed: {}", e);
            Ok(None)
        }
    }
}

/// Cancel any pending prediction for a conversation.
#[tauri::command]
pub async fn cancel_prediction(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    let predictions = state.active_predictions.read().await;
    if let Some(token) = predictions.get(&conversation_id) {
        token.cancel();
    }
    Ok(())
}
