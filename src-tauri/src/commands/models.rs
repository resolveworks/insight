//! Chat provider status commands.
//!
//! Local inference (model downloads, embeddings, OCR) has been removed;
//! the only model surface is the remote chat provider configured via the
//! provider commands (see `providers.rs`).

use serde::Serialize;
use tauri::State;

use crate::core::{AppState, Settings};
use crate::error::{CommandResult, ResultExt};

/// Snapshot of the chat provider's persistent state (unconfigured or ready).
///
/// Transient states (loading, failed) are carried by `model-status-changed`
/// events, not by this query.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub provider_type: Option<String>,
    pub model_id: Option<String>,
    pub ready: bool,
}

/// Get the current chat provider status.
#[tauri::command]
pub async fn get_provider_status(state: State<'_, AppState>) -> CommandResult<ProviderStatus> {
    let config = state.models.chat_config().await;
    let ready = state.models.chat_ready().await;
    Ok(ProviderStatus {
        provider_type: config.as_ref().map(|c| c.provider_type().to_string()),
        model_id: config.map(|c| c.model_id().to_string()),
        ready,
    })
}

/// Disable the chat provider: clear it from the manager and persist
/// `provider = None` in settings.
///
/// Emits no event; the frontend updates its own state after invoking.
#[tauri::command]
pub async fn disable_chat_provider(state: State<'_, AppState>) -> CommandResult<()> {
    tracing::info!("Disabling chat provider");
    state.models.clear_chat().await;

    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = None;
    settings.save(&state.config.settings_file).storage_err()?;

    Ok(())
}
