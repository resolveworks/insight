//! Chat provider commands: rig's provider selections and their catalog
//! models, plus configuring and clearing the active provider.

use tauri::State;

use crate::core::{
    providers::{self, ModelChoice, ProviderSettings},
    AppState, Settings,
};
use crate::error::{CommandResult, ResultExt};

/// Every provider selection rig registers, canonically spelled.
#[tauri::command]
pub fn list_providers() -> Vec<String> {
    providers::selections()
}

/// The chat models rig's catalog lists for `family`'s vendor.
#[tauri::command]
pub fn list_provider_models(family: String) -> CommandResult<Vec<ModelChoice>> {
    providers::models(&family).external_err()
}

/// Configure the chat provider: validate by constructing the model, then
/// persist the settings.
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
        family,
        model,
        api_key,
        base_url,
    };
    // Construction performs no network I/O but validates key/URL inputs.
    providers::build_model(&provider).external_err()?;

    let mut settings = Settings::load(&state.config.settings_file).storage_err()?;
    settings.provider = Some(provider);
    settings.save(&state.config.settings_file).storage_err()?;

    tracing::info!("Chat provider configured");
    Ok(())
}

/// Disable the chat provider: persist `provider = None`.
#[tauri::command]
pub async fn disable_provider(state: State<'_, AppState>) -> CommandResult<()> {
    tracing::info!("Disabling chat provider");
    let mut settings = Settings::load(&state.config.settings_file).storage_err()?;
    settings.provider = None;
    settings.save(&state.config.settings_file).storage_err()?;
    Ok(())
}

/// Get the current provider configuration, if one is set.
#[tauri::command]
pub async fn get_current_provider(
    state: State<'_, AppState>,
) -> CommandResult<Option<ProviderSettings>> {
    let settings = Settings::load(&state.config.settings_file).storage_err()?;
    Ok(settings.provider)
}
