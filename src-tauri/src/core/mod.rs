pub mod config;
pub mod pdf;
pub mod search;
pub mod storage;

use std::sync::Arc;
use tokio::sync::RwLock;

use milli::Index;

pub use config::Config;
pub use storage::Storage;

/// Application state shared across Tauri commands
pub struct AppState {
    pub config: Config,
    pub storage: Arc<RwLock<Option<Storage>>>,
    pub search: Arc<RwLock<Option<Index>>>,
}

impl AppState {
    pub fn new() -> Self {
        let config = Config::load_or_default();
        config.ensure_dirs().expect("Failed to create data directories");

        Self {
            config,
            storage: Arc::new(RwLock::new(None)),
            search: Arc::new(RwLock::new(None)),
        }
    }

    /// Initialize storage and search (call once at startup)
    pub async fn initialize(&self) -> anyhow::Result<()> {
        // Initialize storage
        let storage = Storage::open(&self.config.iroh_dir).await?;
        *self.storage.write().await = Some(storage);

        // Initialize search index
        let index = search::open_index(&self.config.search_dir)?;
        *self.search.write().await = Some(index);

        tracing::info!("AppState initialized");
        Ok(())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
