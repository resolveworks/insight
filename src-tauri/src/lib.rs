pub mod cli;
pub mod commands;
pub mod core;
pub mod headless;

use tauri::{Emitter, Manager};

use crate::core::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("insight=debug".parse().unwrap()),
        )
        .init();

    tracing::info!("Starting Insight in GUI mode");

    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .setup(|app| {
            let state = app.state::<AppState>();
            let state_clone = AppState {
                config: state.config.clone(),
                storage: state.storage.clone(),
                search: state.search.clone(),
                indexer_config: state.indexer_config.clone(),
                embedder: state.embedder.clone(),
                embedding_model_id: state.embedding_model_id.clone(),
                agent_model: state.agent_model.clone(),
                conversations: state.conversations.clone(),
                active_generations: state.active_generations.clone(),
            };

            let app_handle = app.handle().clone();

            // Initialize storage and search in background
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state_clone.initialize().await {
                    tracing::error!("Failed to initialize: {}", e);
                } else {
                    // Notify frontend that backend is ready
                    if let Err(e) = app_handle.emit("backend-ready", ()) {
                        tracing::error!("Failed to emit backend-ready event: {}", e);
                    }
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_collections,
            commands::create_collection,
            commands::delete_collection,
            commands::get_documents,
            commands::get_document,
            commands::get_document_text,
            commands::import_pdfs_batch,
            commands::delete_document,
            commands::search,
            commands::get_node_id,
            // Conversation commands
            commands::list_conversations,
            commands::load_conversation,
            commands::start_chat,
            commands::send_message,
            commands::cancel_generation,
            commands::unload_model,
            // Model management commands
            commands::get_available_models,
            commands::get_model_status,
            commands::download_model,
            // Embedding model commands
            commands::get_available_embedding_models,
            commands::get_current_embedding_model,
            commands::get_embedding_model_status,
            commands::download_embedding_model,
            commands::configure_embedding_model,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub fn run_headless() {
    headless::run();
}
