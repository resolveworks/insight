pub mod commands;
pub mod core;

use tauri::Manager;

use crate::core::{AppState, Config};

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
            let state_clone = AppState {
                config: state.config.clone(),
                storage: state.storage.clone(),
                search: state.search.clone(),
                indexer_config: state.indexer_config.clone(),
                embedder: state.embedder.clone(),
                embedding_model_id: state.embedding_model_id.clone(),
                agent_model: state.agent_model.clone(),
                language_model_id: state.language_model_id.clone(),
                conversations: state.conversations.clone(),
                active_generations: state.active_generations.clone(),
                job_coordinator: state.job_coordinator.clone(),
            };
            let app_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                state_clone.load_models_if_configured(&app_handle).await;
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
            commands::import_pdfs_batch,
            commands::delete_document,
            commands::search,
            // Conversation commands
            commands::list_conversations,
            commands::load_conversation,
            commands::start_chat,
            commands::send_message,
            commands::cancel_generation,
            commands::unload_model,
            // Language model commands
            commands::get_available_language_models,
            commands::get_language_model_status,
            commands::download_language_model,
            commands::get_current_language_model,
            commands::configure_language_model,
            // Embedding model commands
            commands::get_available_embedding_models,
            commands::get_current_embedding_model,
            commands::get_embedding_model_status,
            commands::download_embedding_model,
            commands::configure_embedding_model,
            // Boot
            commands::get_boot_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
