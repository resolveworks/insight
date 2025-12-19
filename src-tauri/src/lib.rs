pub mod cli;
pub mod commands;
pub mod core;
pub mod headless;

use tauri::Manager;

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
            };

            // Initialize storage and search in background
            tauri::async_runtime::spawn(async move {
                if let Err(e) = state_clone.initialize().await {
                    tracing::error!("Failed to initialize: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_collections,
            commands::create_collection,
            commands::delete_collection,
            commands::get_documents,
            commands::import_pdf,
            commands::import_pdfs_batch,
            commands::delete_document,
            commands::search,
            commands::get_node_id,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

pub fn run_headless() {
    headless::run();
}
