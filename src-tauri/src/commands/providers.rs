//! Provider registry commands (family listing, model listing, configuration).

use std::collections::HashMap;

use tauri::State;

use crate::core::{
    providers::{self, ProviderFamily, ProviderModel, ProviderSettings},
    AppState, Settings,
};
use crate::error::{CommandResult, ResultExt};

/// List every provider family in the registry.
#[tauri::command]
pub fn list_provider_families() -> Vec<ProviderFamily> {
    providers::list_families()
}

/// List the models a family offers, querying the provider remotely.
#[tauri::command]
pub async fn list_provider_models(
    family: String,
    api_key: Option<String>,
    base_url: Option<String>,
) -> CommandResult<Vec<ProviderModel>> {
    providers::list_models(&family, api_key.as_deref(), base_url.as_deref())
        .await
        .external_err()
}

/// Configure the chat provider: validate by constructing the model, then
/// persist the provider settings and the family's API key.
#[tauri::command]
pub async fn configure_provider(
    family: String,
    api_key: Option<String>,
    base_url: Option<String>,
    model: String,
    state: State<'_, AppState>,
) -> CommandResult<()> {
    tracing::info!(family = %family, model = %model, "Configuring chat provider");

    let provider = ProviderSettings {
        family: family.clone(),
        model,
        api_key: api_key.clone(),
        base_url,
    };
    // Construction performs no network I/O but validates key/URL inputs.
    providers::build_model(&provider).external_err()?;

    let mut settings = Settings::load(&state.config.settings_file);
    if let Some(key) = api_key {
        settings.api_keys.insert(family.clone(), key);
    }
    settings.provider = Some(provider);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!(family = %family, "Chat provider configured");
    Ok(())
}

/// Disable the chat provider: persist `provider = None` (stored API keys are
/// kept for easy switching back).
#[tauri::command]
pub async fn disable_provider(state: State<'_, AppState>) -> CommandResult<()> {
    tracing::info!("Disabling chat provider");
    let mut settings = Settings::load(&state.config.settings_file);
    settings.provider = None;
    settings.save(&state.config.settings_file).storage_err()?;
    Ok(())
}

/// Get the current provider configuration, if one is set.
#[tauri::command]
pub async fn get_current_provider(
    state: State<'_, AppState>,
) -> CommandResult<Option<ProviderSettings>> {
    let settings = Settings::load(&state.config.settings_file);
    Ok(settings.provider)
}

/// Get stored API keys per family (for auto-populating the settings UI).
#[tauri::command]
pub async fn get_stored_api_keys(
    state: State<'_, AppState>,
) -> CommandResult<HashMap<String, String>> {
    let settings = Settings::load(&state.config.settings_file);
    Ok(settings.api_keys)
}
