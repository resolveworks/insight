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
            let (state, mut pipeline_progress_rx) =
                tauri::async_runtime::block_on(AppState::new(config))?;
            app.manage(state);

            // Forward pipeline progress events to frontend
            let pipeline_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                while let Some(progress) = pipeline_progress_rx.recv().await {
                    let _ = pipeline_handle.emit("pipeline-progress", &progress);
                }
            });

            // Subscribe the frontend to the manager's status broadcast so
            // lazy-load transitions (loading → ready/failed on first use)
            // surface as `model-status-changed` events.
            let state = app.state::<AppState>();
            let mut status_rx = state.models.subscribe_status();
            let status_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                while let Ok(status) = status_rx.recv().await {
                    let _ = status_handle.emit("model-status-changed", &status);
                }
            });

            // Restore provider configs (no weights loaded yet) and start
            // the idle reaper. Both need a Tokio runtime, so we do them
            // from inside async_runtime::spawn.
            let state_clone = state.inner().clone();
            let app_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;

                state_clone.models.spawn_idle_reaper();

                let (status_tx, mut status_rx) = tokio::sync::mpsc::channel::<ModelStatus>(10);
                let (progress_tx, mut progress_rx) =
                    tokio::sync::mpsc::channel::<ModelDownloadProgress>(100);

                // Forward download-sourced status events (distinct from the
                // manager's broadcast channel — this covers the one-shot
                // "downloading" transition during initial setup).
                let status_handle = app_handle.clone();
                tokio::spawn(async move {
                    while let Some(status) = status_rx.recv().await {
                        let _ = status_handle.emit("model-status-changed", &status);
                    }
                });

                let progress_handle = app_handle.clone();
                tokio::spawn(async move {
                    while let Some(progress) = progress_rx.recv().await {
                        let _ = progress_handle.emit("model-download-progress", &progress);
                    }
                });

                state_clone
                    .restore_configs_from_settings(status_tx, progress_tx)
                    .await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::collections::get_collections,
            commands::collections::create_collection,
            commands::collections::delete_collection,
            commands::collections::share_collection,
            commands::collections::import_collection,
            commands::documents::get_documents,
            commands::documents::get_document,
            commands::documents::get_document_text,
            commands::documents::get_document_chunks,
            commands::documents::start_import,
            commands::documents::get_pipeline_progress,
            commands::documents::get_collection_pipeline_progress,
            commands::documents::delete_document,
            // Conversation commands
            commands::conversations::list_conversations,
            commands::conversations::load_conversation,
            commands::conversations::start_chat,
            commands::conversations::send_message,
            commands::conversations::cancel_generation,
            commands::conversations::set_conversation_collections,
            commands::conversations::delete_conversation,
            // Model commands (unified)
            commands::models::get_available_models,
            commands::models::get_model_status,
            commands::models::get_provider_status,
            commands::models::download_model,
            commands::models::get_current_model,
            commands::models::configure_model,
            // Provider management
            commands::providers::get_provider_families,
            commands::providers::get_current_provider,
            commands::providers::fetch_openai_models,
            commands::providers::fetch_anthropic_models,
            commands::providers::configure_openai_provider,
            commands::providers::configure_anthropic_provider,
            commands::providers::get_stored_api_keys,
            commands::providers::get_lifecycle_config,
            commands::providers::set_lifecycle_config,
            commands::providers::research_focus_enter,
            commands::providers::research_focus_leave,
            // Prediction commands (tab completion)
            commands::conversations::predict_next_message,
            commands::conversations::cancel_prediction,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
