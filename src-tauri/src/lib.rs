pub mod commands;
pub mod core;
pub mod error;

use tauri::Manager;

use crate::commands::documents::CollectionInvalidated;
use crate::core::{AppEvent, AppState, Config};

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
            let (state, mut app_events_rx) = tauri::async_runtime::block_on(AppState::new(config))?;
            app.manage(state);
            let state = app.state::<AppState>();

            // Forward pipeline events to the frontend
            let pipeline_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tauri::Emitter;
                while let Some(event) = app_events_rx.recv().await {
                    match event {
                        AppEvent::Progress(progress) => {
                            let _ = pipeline_handle.emit("pipeline-progress", &progress);
                        }
                        AppEvent::CollectionInvalidated { namespace } => {
                            let _ = pipeline_handle.emit(
                                "collection-invalidated",
                                CollectionInvalidated {
                                    collection_id: namespace.to_string(),
                                },
                            );
                        }
                    }
                }
            });

            // Restore the chat provider config from settings (no remote
            // connections opened yet) once the async runtime is up.
            let state_clone = state.inner().clone();

            tauri::async_runtime::spawn(async move {
                if let Err(e) = state_clone.restore_configs_from_settings().await {
                    tracing::error!(
                        error = %e,
                        "Failed to restore provider configuration from settings"
                    );
                }
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
            commands::conversations::delete_conversation,
            // Chat provider status
            commands::models::get_provider_status,
            // Provider management
            commands::providers::list_providers,
            commands::providers::list_provider_models,
            commands::providers::configure_provider,
            commands::providers::disable_provider,
            commands::providers::get_current_provider,
            // Prediction commands (tab completion)
            commands::conversations::predict_next_message,
            commands::conversations::cancel_prediction,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {});
}
