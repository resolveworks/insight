pub mod commands;
pub mod core;
pub mod error;

use tauri::Manager;
use tauri_plugin_log::{log::LevelFilter, Target, TargetKind};

use crate::commands::collections::SharingStatusChanged;
use crate::commands::documents::CollectionInvalidated;
use crate::core::{AppEvent, AppState, Config};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([
                    Target::new(TargetKind::Stdout),
                    Target::new(TargetKind::LogDir { file_name: None }),
                ])
                .max_file_size(10 * 1024 * 1024)
                .level(LevelFilter::Info)
                .level_for("insight_lib", LevelFilter::Debug)
                .level_for("insight_core", LevelFilter::Debug)
                .level_for("milli", LevelFilter::Debug)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // The log plugin installs its logger while plugins are set up, so
            // this is the earliest line that reaches a target.
            tracing::info!("Starting Insight");

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
                        AppEvent::SharingStatusChanged { namespace } => {
                            let _ = pipeline_handle.emit(
                                "sharing-status-changed",
                                SharingStatusChanged {
                                    namespace: namespace.map(|namespace| namespace.to_string()),
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
            commands::collections::get_collection_sharing_status,
            commands::collections::import_collection,
            commands::documents::get_documents,
            commands::documents::get_document,
            commands::documents::get_document_text,
            commands::documents::get_document_source,
            commands::documents::start_import,
            commands::documents::get_pipeline_progress,
            commands::documents::delete_document,
            commands::documents::undelete_document,
            commands::documents::get_deleted_documents,
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {});
}
