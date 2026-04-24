use iroh_docs::NamespaceId;
use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;

use crate::core::{agent, conversations, AppState, ProviderEvent};
use crate::error::{CommandError, CommandResult, ResultExt};

/// List all saved conversations
#[tauri::command]
pub async fn list_conversations(
    state: State<'_, AppState>,
) -> CommandResult<Vec<conversations::ConversationSummary>> {
    conversations::list_conversations(&state.config.conversations_dir).storage_err()
}

/// Load a conversation by ID
#[tauri::command]
pub async fn load_conversation(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<agent::Conversation> {
    let path = conversations::conversation_path(&state.config.conversations_dir, &conversation_id);
    let conversation = conversations::load_conversation(&path).storage_err()?;

    // Add to in-memory cache
    state
        .conversations
        .write()
        .await
        .insert(conversation_id, conversation.clone());

    Ok(conversation)
}

/// Enrich bare [`CollectionInfo`] values (which the frontend only knows a
/// subset of) with storage-derived `total_pages` counts. Collections whose IDs
/// don't parse as valid namespaces are passed through untouched so the call
/// never silently drops the user's selection.
async fn enrich_collections(
    state: &AppState,
    collections: Vec<agent::CollectionInfo>,
) -> Vec<agent::CollectionInfo> {
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
        enriched.push(agent::CollectionInfo {
            id: col.id,
            name: col.name,
            document_count: documents.len(),
            total_pages: documents.iter().map(|d| d.page_count).sum(),
            created_at: None,
        });
    }
    enriched
}

/// Start a new chat conversation
///
/// Requires a chat provider to be configured first. If `collections` is
/// provided and non-empty, the conversation's initial scope is recorded via
/// `Conversation::set_collections` so a breadcrumb is added to the transcript.
#[tauri::command]
pub async fn start_chat(
    collections: Option<Vec<agent::CollectionInfo>>,
    state: State<'_, AppState>,
) -> CommandResult<agent::Conversation> {
    if !state.models.chat_ready().await {
        return Err(CommandError::provider_not_configured());
    }

    let conversation_id = uuid::Uuid::new_v4().to_string();
    let mut conversation = agent::Conversation::new(conversation_id.clone());

    if let Some(cols) = collections {
        if !cols.is_empty() {
            let enriched = enrich_collections(&state, cols).await;
            conversation.set_collections(enriched);
        }
    }

    conversations::save_conversation(&state.config.conversations_dir, &conversation)
        .storage_err()?;

    state
        .conversations
        .write()
        .await
        .insert(conversation_id.clone(), conversation.clone());

    tracing::info!("Started new chat conversation: {}", conversation_id);
    Ok(conversation)
}

/// Replace the active collection scope for a conversation.
///
/// Appends a scope-change breadcrumb to the transcript (when the selection
/// actually differs) so the model knows earlier tool results were run against
/// the previous scope.
#[tauri::command]
pub async fn set_conversation_collections(
    conversation_id: String,
    collections: Vec<agent::CollectionInfo>,
    state: State<'_, AppState>,
) -> CommandResult<agent::Conversation> {
    let mut conversation = state
        .conversations
        .read()
        .await
        .get(&conversation_id)
        .cloned()
        .ok_or(CommandError::conversation_not_found())?;

    // Skip storage work when the selection is unchanged — set_collections
    // would no-op anyway, but enrichment isn't free for multi-collection
    // workspaces.
    let existing_ids: std::collections::HashSet<&str> = conversation
        .collections
        .iter()
        .map(|c| c.id.as_str())
        .collect();
    let unchanged = collections.len() == existing_ids.len()
        && collections
            .iter()
            .all(|c| existing_ids.contains(c.id.as_str()));
    if unchanged {
        return Ok(conversation);
    }

    let enriched = enrich_collections(&state, collections).await;
    let changed = conversation.set_collections(enriched);

    if changed {
        conversations::save_conversation(&state.config.conversations_dir, &conversation)
            .storage_err()?;
        state
            .conversations
            .write()
            .await
            .insert(conversation_id, conversation.clone());
    }

    Ok(conversation)
}

/// Delete a conversation. Cancels any in-flight generation or prediction,
/// drops the in-memory entry, and removes the JSON file from disk.
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

    state.conversations.write().await.remove(&conversation_id);

    conversations::delete_conversation(&state.config.conversations_dir, &conversation_id)
        .storage_err()?;

    Ok(())
}

/// Send a message to a conversation and stream the response.
///
/// The search tools are scoped to the conversation's stored collection set;
/// callers change that scope via [`set_conversation_collections`] ahead of
/// time, not here.
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

    let conversation = state
        .conversations
        .read()
        .await
        .get(&conversation_id)
        .ok_or(CommandError::conversation_not_found())?
        .clone();

    if !state.models.chat_ready().await {
        return Err(CommandError::provider_not_configured());
    }

    if conversation.collections.is_empty() {
        return Err(CommandError::no_collection_scope());
    }

    let cancel_token = CancellationToken::new();
    state
        .active_generations
        .write()
        .await
        .insert(conversation_id.clone(), cancel_token.clone());

    let (tx, mut rx) = tokio::sync::mpsc::channel::<agent::AgentEvent>(100);

    let app_handle = app.clone();
    let conv_id = conversation_id.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let event_name = format!("agent-event-{}", conv_id);
            if let Err(e) = app_handle.emit(&event_name, &event) {
                tracing::error!("Failed to emit agent event: {}", e);
            }
        }
    });

    let conversations_dir = state.config.conversations_dir.clone();
    let state_clone = state.inner().clone();

    let conv_id = conversation_id.clone();
    let mut conversation = conversation;
    let scope_collections = Some(conversation.collections.clone());

    tokio::spawn(async move {
        let ctx = agent::AgentContext {
            state: state_clone.clone(),
            collections: scope_collections,
        };

        let lease = match state_clone.models.acquire_chat().await {
            Ok(Some(l)) => l,
            Ok(None) => {
                tracing::error!("Provider not configured when running agent loop");
                return;
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to load chat provider");
                return;
            }
        };

        if let Err(e) = agent::run_agent_loop(
            lease.provider(),
            &mut conversation,
            message,
            &ctx,
            tx,
            cancel_token,
        )
        .await
        {
            tracing::error!(
                conversation_id = %conv_id,
                error = %e,
                error_chain = ?e,
                "Agent loop error"
            );
        }
        drop(lease);

        let user_count = conversation
            .messages
            .iter()
            .filter(|m| m.role == agent::MessageRole::User)
            .count();
        if user_count == 1 {
            conversation.generate_title();
        }

        if let Err(e) = conversations::save_conversation(&conversations_dir, &conversation) {
            tracing::error!("Failed to save conversation: {}", e);
            return;
        }

        // Delete may race with the save: if the cache entry is gone, the JSON
        // we just wrote would resurrect a deleted conversation. Remove the
        // orphan so delete stays durable. The cache write is kept short so
        // other conversation commands aren't blocked by disk I/O.
        let orphaned = {
            let mut map = state_clone.conversations.write().await;
            if map.contains_key(&conv_id) {
                map.insert(conv_id.clone(), conversation);
                false
            } else {
                true
            }
        };
        if orphaned {
            if let Err(e) = conversations::delete_conversation(&conversations_dir, &conv_id) {
                tracing::warn!("Failed to clean up orphaned conversation file: {}", e);
            }
        }
    });

    Ok(())
}

/// Cancel an in-progress generation
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

const PREDICTION_PROMPT: &str = r#"Based on the conversation above, predict what the user is most likely to ask or say next.

Rules:
- Output ONLY the predicted message, nothing else
- Keep it concise (1-2 sentences max)
- Make it a natural follow-up question or statement
- If the assistant just answered a question, predict a likely follow-up
- If unsure, output nothing"#;

/// Predict what the user might say next in a conversation
#[tauri::command]
pub async fn predict_next_message(
    conversation_id: String,
    state: State<'_, AppState>,
) -> CommandResult<Option<String>> {
    let conversation = match state.conversations.read().await.get(&conversation_id) {
        Some(c) => c.clone(),
        None => return Ok(None),
    };

    // Need at least one user + one assistant message for a meaningful prediction.
    let non_system_messages = conversation
        .messages
        .iter()
        .filter(|m| m.role != agent::MessageRole::System)
        .count();
    if non_system_messages < 2 {
        return Ok(None);
    }

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

    let lease = match state.models.acquire_chat().await {
        Ok(Some(l)) => l,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!(error = %e, "Failed to load chat provider for prediction");
            return Ok(None);
        }
    };

    let mut prediction_messages = conversation.messages.clone();
    prediction_messages.push(agent::Message {
        role: agent::MessageRole::User,
        content: vec![agent::ContentBlock::Text {
            text: PREDICTION_PROMPT.to_string(),
        }],
    });

    let (tx, mut rx) = tokio::sync::mpsc::channel::<ProviderEvent>(50);

    let collect_task = tokio::spawn(async move {
        let mut result = String::new();
        while let Some(event) = rx.recv().await {
            if let ProviderEvent::TextDelta(text) = event {
                result.push_str(&text);
            }
        }
        result
    });

    let completion_result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        lease.stream_completion(&prediction_messages, &[], tx, cancel_token.clone()),
    )
    .await;

    state
        .active_predictions
        .write()
        .await
        .remove(&conversation_id);

    let collected_text = collect_task.await.unwrap_or_default();

    match completion_result {
        Ok(Ok(_)) => {
            let prediction = collected_text.trim().to_string();
            let prediction = if prediction.len() > 150 {
                format!("{}...", &prediction[..147])
            } else {
                prediction
            };
            if prediction.is_empty() {
                Ok(None)
            } else {
                Ok(Some(prediction))
            }
        }
        Ok(Err(e)) => {
            tracing::debug!("Prediction failed: {}", e);
            Ok(None)
        }
        Err(_) => {
            tracing::debug!("Prediction timed out");
            Ok(None)
        }
    }
}

/// Cancel any pending prediction for a conversation
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
