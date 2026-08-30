//! Conversation commands: lifecycle (list/load/start/delete), scope
//! management, message sending with streamed agent events, and next-message
//! prediction.

use iroh_docs::NamespaceId;
use rig::completion::Message;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::core::{
    providers, runtime, AppState, CollectionInfo, ConversationMeta, ConversationSummary, Settings,
};
use crate::error::{CommandError, CommandResult, ResultExt};

/// A conversation as seen by the frontend: metadata sidecar plus the full
/// rig message history.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationView {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub collections: Vec<CollectionInfo>,
    pub messages: Vec<Message>,
}

/// Load meta + messages into a [`ConversationView`].
async fn conversation_view(
    state: &AppState,
    meta: ConversationMeta,
) -> Result<ConversationView, CommandError> {
    use rig::memory::ConversationMemory;

    let id: rig::id::ConversationId = meta.id.clone().into();
    let messages = state
        .memory
        .load(&id)
        .await
        .map_err(|e| CommandError::internal(format!("failed to load messages: {e}")))?;
    Ok(ConversationView {
        id: meta.id,
        title: meta.title,
        created_at: meta.created_at,
        updated_at: meta.updated_at,
        collections: meta.collections,
        messages,
    })
}

/// List all saved conversations, most recently updated first.
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> CommandResult<Vec<ConversationSummary>> {
    state.memory.list_summaries().await.storage_err()
}

/// Load a conversation by ID (404-style error when missing).
#[tauri::command]
pub async fn load_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<ConversationView> {
    let meta = state
        .memory
        .load_meta(&conversation_id)
        .await
        .storage_err()?
        .ok_or_else(CommandError::conversation_not_found)?;
    conversation_view(&state, meta).await
}

/// Enrich bare [`CollectionInfo`] values (which the frontend only knows a
/// subset of) with storage-derived counts. Collections whose IDs don't parse
/// as valid namespaces are passed through untouched so the call never
/// silently drops the user's selection.
async fn enrich_collections(
    state: &AppState,
    collections: Vec<CollectionInfo>,
) -> Vec<CollectionInfo> {
    let storage = state.storage.read().await;
    let mut enriched = Vec::with_capacity(collections.len());
    for col in collections {
        let Ok(namespace_id) = col.id.parse::<NamespaceId>() else {
            enriched.push(col);
            continue;
        };
        let documents = storage
            .list_documents(namespace_id)
            .await
            .unwrap_or_default();
        enriched.push(CollectionInfo {
            id: col.id,
            name: col.name,
            document_count: documents.len(),
            total_pages: documents.iter().map(|d| d.page_count).sum(),
            created_at: None,
        });
    }
    enriched
}

/// Whether the configured chat provider passes validation.
async fn provider_ready(state: &AppState) -> bool {
    let settings = Settings::load(&state.config.settings_file);
    settings
        .provider
        .as_ref()
        .is_some_and(|p| providers::build_model(p).is_ok())
}

/// Start a new chat conversation.
///
/// Requires a configured chat provider and a non-empty collection scope.
#[tauri::command]
pub async fn start_chat(
    collections: Option<Vec<CollectionInfo>>,
    state: State<'_, AppState>,
) -> CommandResult<ConversationView> {
    if !provider_ready(&state).await {
        return Err(CommandError::provider_not_configured());
    }

    let collections = match collections {
        Some(cols) if !cols.is_empty() => enrich_collections(&state, cols).await,
        _ => return Err(CommandError::no_collection_scope()),
    };

    let id = uuid::Uuid::new_v4().to_string();
    let meta = ConversationMeta {
        id: id.clone(),
        title: "New conversation".to_string(),
        collections,
        created_at: String::new(),
        updated_at: String::new(),
    };
    let meta = state.memory.create(meta).await.storage_err()?;

    tracing::info!("Started new chat conversation: {}", id);
    conversation_view(&state, meta).await
}

/// Replace the active collection scope for a conversation. Returns the
/// refreshed view.
#[tauri::command]
pub async fn set_conversation_collections(
    conversation_id: String,
    collections: Vec<CollectionInfo>,
    state: State<'_, AppState>,
) -> CommandResult<ConversationView> {
    let mut meta = state
        .memory
        .load_meta(&conversation_id)
        .await
        .storage_err()?
        .ok_or_else(CommandError::conversation_not_found)?;

    // Skip storage work when the selection is unchanged.
    let existing_ids: std::collections::HashSet<&str> =
        meta.collections.iter().map(|c| c.id.as_str()).collect();
    let unchanged = collections.len() == existing_ids.len()
        && collections
            .iter()
            .all(|c| existing_ids.contains(c.id.as_str()));
    if !unchanged {
        meta.collections = enrich_collections(&state, collections).await;
        state
            .memory
            .update_meta(&conversation_id, meta)
            .await
            .storage_err()?;
    }

    load_conversation(conversation_id, state).await
}

/// Delete a conversation: cancel any in-flight generation or prediction,
/// then remove it from disk. Idempotent.
#[tauri::command]
pub async fn delete_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!("Deleting conversation {}", conversation_id);

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

    state.memory.delete(&conversation_id).await.storage_err()?;
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
    conversation_id: String,
    message: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(
        "Sending message to conversation {}: {}",
        conversation_id,
        message
    );

    let meta = state
        .memory
        .load_meta(&conversation_id)
        .await
        .storage_err()?
        .ok_or_else(CommandError::conversation_not_found)?;

    if !provider_ready(&state).await {
        return Err(CommandError::provider_not_configured());
    }
    if meta.collections.is_empty() {
        return Err(CommandError::no_collection_scope());
    }

    let cancel_token = CancellationToken::new();
    state
        .active_generations
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    let state_clone = state.inner().clone();
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

        if let Err(e) =
            runtime::send_message(&state_clone, &conv_id, message, emitter, cancel_token).await
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
    let result = runtime::predict_next_message(&state_clone, &conversation_id, cancel_token).await;

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
