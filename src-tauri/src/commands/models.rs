//! Chat provider status commands.
//!
//! The provider is read from persisted settings and validated live by
//! constructing the model handle (`build_model`); there is no long-lived
//! model state in the app anymore.

use serde::Serialize;
use tauri::State;

use crate::core::{providers, AppState, Settings};
use crate::error::CommandResult;

/// Snapshot of the chat provider's state for the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatus {
    pub family: Option<String>,
    pub model: Option<String>,
    pub ready: bool,
}

/// Get the current chat provider status. `ready` means a provider is
/// configured AND its settings pass `build_model` validation.
#[tauri::command]
pub async fn get_provider_status(state: State<'_, AppState>) -> CommandResult<ProviderStatus> {
    let settings = Settings::load(&state.config.settings_file);
    let (family, model, ready) = match settings.provider {
        Some(provider) => {
            let ready = providers::build_model(&provider).is_ok();
            (Some(provider.family), Some(provider.model), ready)
        }
        None => (None, None, false),
    };
    Ok(ProviderStatus {
        family,
        model,
        ready,
    })
}
