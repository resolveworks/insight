use serde::{Deserialize, Serialize};
use tauri::State;

use crate::core::{
    get_provider_families as core_get_provider_families, AnthropicProvider, AppState,
    OpenAIProvider, ProviderConfig, ProviderFamily, RemoteModelInfo,
};
use crate::error::{CommandResult, ResultExt};

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
    let config = state.provider_config.read().await;
    Ok(config.clone())
}

/// Fetch available models from OpenAI API
#[tauri::command]
pub async fn fetch_openai_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    OpenAIProvider::fetch_models(&api_key).await.external_err()
}

/// Fetch available models for Anthropic (verifies API key)
#[tauri::command]
pub async fn fetch_anthropic_models(api_key: String) -> CommandResult<Vec<RemoteModelInfo>> {
    AnthropicProvider::verify_api_key(&api_key)
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

    let provider = OpenAIProvider::new(&api_key, &model);
    let config = ProviderConfig::OpenAI {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    // Update state
    *state.chat_provider.write().await = Some(Box::new(provider));
    *state.provider_config.write().await = Some(config.clone());

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

    let provider = AnthropicProvider::new(&api_key, &model);
    let config = ProviderConfig::Anthropic {
        api_key: api_key.clone(),
        model: model.clone(),
    };

    // Update state
    *state.chat_provider.write().await = Some(Box::new(provider));
    *state.provider_config.write().await = Some(config.clone());

    // Persist setting and store API key separately for easy switching
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
