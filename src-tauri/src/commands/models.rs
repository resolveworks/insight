//! Chat provider status command.
//!
//! The provider is read from persisted settings and validated live by
//! constructing the model handle (`build_model`); there is no long-lived
//! model state in the app.

use serde::Serialize;
use tauri::State;

use crate::core::{providers, AppState, Settings};
use crate::error::{CommandResult, ResultExt};

/// Snapshot of the chat provider's state for the UI.
///
/// Chat is enabled when `family` is set and `error` is not; `error` says why
/// a configured provider cannot be used.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub family: Option<String>,
    pub model: Option<String>,
    pub error: Option<String>,
}

/// Get the current chat provider status.
#[tauri::command]
pub async fn get_provider_status(state: State<'_, AppState>) -> CommandResult<ProviderStatus> {
    let settings = Settings::load(&state.config.settings_file).storage_err()?;
    let status = match settings.provider {
        Some(provider) => ProviderStatus {
            family: Some(provider.family.clone()),
            model: Some(provider.model.clone()),
            error: providers::build_model(&provider)
                .err()
                .map(|e| e.to_string()),
        },
        None => ProviderStatus {
            family: None,
            model: None,
            error: None,
        },
    };
    Ok(status)
}
