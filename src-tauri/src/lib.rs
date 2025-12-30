pub mod commands;
pub mod core;
pub mod error;

use tauri::Manager;

use crate::core::{AppState, Config, ModelDownloadProgress, ModelStatus};

/// Initialize tracing/logging with the given directives
pub fn init_logging(directives: &[&str]) {
    let mut filter = tracing_subscriber::EnvFilter::from_default_env();
    for directive in directives {
        filter = filter.add_directive(directive.parse().unwrap());
    }
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging(&["insight=debug", "milli=debug"]);
    tracing::info!("Starting Insight");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // Load config
            let config = Config::load_or_default();
            config.ensure_dirs()?;

            // Initialize state using Tauri's async runtime (fast, ~100ms)
            let state = tauri::async_runtime::block_on(AppState::new(config))?;
            app.manage(state);

            // Load models in background (slow, 20-30s)
            let state = app.state::<AppState>();
            let state_clone = state.inner().clone();
            let app_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;

                let (status_tx, mut status_rx) = tokio::sync::mpsc::channel::<ModelStatus>(10);
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::channel::<ModelDownloadProgress>(100);

                // Forward status events to frontend
                let status_handle = app_handle.clone();
                tokio::spawn(async move {
                    while let Some(status) = status_rx.recv().await {
                        let _ = status_handle.emit("model-status-changed", &status);
                    }
                });

                // Forward progress events to frontend
                let progress_handle = app_handle.clone();
                tokio::spawn(async move {
                    while let Some(progress) = progress_rx.recv().await {
                        let _ = progress_handle.emit("model-download-progress", &progress);
                    }
                });

                state_clone
                    .load_models_if_configured(status_tx, progress_tx)
                    .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_collections,
            commands::create_collection,
            commands::delete_collection,
            commands::share_collection,
            commands::import_collection,
            commands::get_documents,
            commands::get_document,
            commands::get_document_text,
            commands::get_document_chunks,
            commands::start_import,
            commands::get_import_progress,
            commands::delete_document,
            // Conversation commands
            commands::list_conversations,
            commands::load_conversation,
            commands::start_chat,
            commands::send_message,
            commands::cancel_generation,
            // Model commands (unified)
            commands::get_available_models,
            commands::get_model_status,
            commands::get_provider_status,
            commands::download_model,
            commands::get_current_model,
            commands::configure_model,
            // Provider management
            commands::get_provider_families,
            commands::get_current_provider,
            commands::fetch_openai_models,
            commands::fetch_anthropic_models,
            commands::configure_openai_provider,
            commands::configure_anthropic_provider,
            commands::get_stored_api_keys,
            // Prediction commands (tab completion)
            commands::predict_next_message,
            commands::cancel_prediction,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
