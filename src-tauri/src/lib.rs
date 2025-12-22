pub mod cli;
pub mod commands;
pub mod core;

use tauri::Manager;

use crate::core::AppState;

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
    tracing::info!("Starting Insight in GUI mode");

    // Initialize AppState before Tauri starts (blocks briefly on storage init)
    let app_state = AppState::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(app_state)
        .setup(|app| {
            // Load embedder in background (this is the slow part, 20-30s)
            let state = app.state::<AppState>();
            let config = state.config.clone();
            let embedder = state.embedder.clone();
            let embedding_model_id = state.embedding_model_id.clone();
            let app_handle = app.handle().clone();

            tauri::async_runtime::spawn(async move {
                core::load_embedder_if_configured(
                    &config,
                    &embedder,
                    &embedding_model_id,
                    &app_handle,
                )
                .await;
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
