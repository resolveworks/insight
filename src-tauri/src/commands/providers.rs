use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::core::{
    get_provider_families as core_get_provider_families, AnthropicChatProvider, AppState,
    LifecycleConfig, OpenAIChatProvider, ProviderConfig, ProviderFamily, RemoteModelInfo,
};
use crate::error::{CommandError, CommandResult, ResultExt};

/// Get available provider families
#[tauri::command]
pub async fn get_provider_families() -> Vec<ProviderFamily> {
    core_get_provider_families()
}

/// Get current provider configuration
#[tauri::command]
pub async fn get_current_provider(
    state: State<'_, AppState>,
) -> CommandResult<Option<ProviderConfig>> {
    Ok(state.models.chat_config().await)
}

/// Fetch available models from OpenAI API
#[tauri::command]
pub async fn fetch_openai_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    OpenAIChatProvider::fetch_models(&api_key)
        .await
        .external_err()
}

/// Fetch available models for Anthropic (verifies API key)
#[tauri::command]
pub async fn fetch_anthropic_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    AnthropicChatProvider::verify_api_key(&api_key)
        .await
        .external_err()
}

/// Configure OpenAI as the chat provider
#[tauri::command]
pub async fn configure_openai_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::Settings;

    tracing::info!("Configuring OpenAI provider with model: {}", model);

    let provider = OpenAIChatProvider::new(&api_key, &model);
    let config = ProviderConfig::OpenAI {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    state
        .models
        .set_chat(Arc::new(provider), config.clone())
        .await
        .map_err(|e| CommandError::internal(format!("Failed to install provider: {}", e)))?;

    // Persist setting and store API key separately for easy switching
    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = Some(config);
    settings.openai_api_key = Some(api_key);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!("OpenAI provider configured successfully");
    Ok(())
}

/// Configure Anthropic as the chat provider
#[tauri::command]
pub async fn configure_anthropic_provider(
    api_key: String,
    model: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::Settings;

    tracing::info!("Configuring Anthropic provider with model: {}", model);

    let provider = AnthropicChatProvider::new(&api_key, &model);
    let config = ProviderConfig::Anthropic {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    state
        .models
        .set_chat(Arc::new(provider), config.clone())
        .await
        .map_err(|e| CommandError::internal(format!("Failed to install provider: {}", e)))?;

    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = Some(config);
    settings.anthropic_api_key = Some(api_key);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!("Anthropic provider configured successfully");
    Ok(())
}

/// Get stored API keys (for auto-populating when switching providers)
#[tauri::command]
pub async fn get_stored_api_keys(state: State<'_, AppState>) -> CommandResult<StoredApiKeys> {
    use crate::core::Settings;

    let settings = Settings::load(&state.config.settings_file);
    Ok(StoredApiKeys {
        openai: settings.openai_api_key,
        anthropic: settings.anthropic_api_key,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredApiKeys {
    pub openai: Option<String>,
    pub anthropic: Option<String>,
}

/// Get the current lifecycle config (coexist flags).
#[tauri::command]
pub async fn get_lifecycle_config(state: State<'_, AppState>) -> CommandResult<LifecycleConfig> {
    Ok(state.models.lifecycle_config().await)
}

/// Update the lifecycle config. Propagates coexist flags to any currently
/// installed providers without triggering a load.
#[tauri::command]
pub async fn set_lifecycle_config(
    config: LifecycleConfig,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    use crate::core::Settings;

    state.models.set_lifecycle_config(config.clone()).await;

    let mut settings = Settings::load(&state.config.settings_file);
    settings.lifecycle = config;
    settings.save(&state.config.settings_file).storage_err()?;

    Ok(())
}

/// Mark the research surface as focused. Background workers (embed, OCR)
/// yield at their next job boundary so chat has priority access to the
/// local models.
#[tauri::command]
pub async fn research_focus_enter(state: State<'_, AppState>) -> CommandResult<()> {
    state.models.set_research_focused(true);
    Ok(())
}

/// Mark the research surface as no longer focused. Background workers
/// resume on their next poll.
#[tauri::command]
pub async fn research_focus_leave(state: State<'_, AppState>) -> CommandResult<()> {
    state.models.set_research_focused(false);
    Ok(())
}
