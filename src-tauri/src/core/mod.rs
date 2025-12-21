pub mod agent;
pub mod config;
pub mod conversations;
pub mod embeddings;
pub mod models;
pub mod pdf;
pub mod search;
pub mod storage;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;

use milli::update::IndexerConfig;
use milli::Index;

pub use embeddings::Embedder;

pub use agent::{AgentEvent, AgentModel, Conversation};
pub use config::{Config, Settings};
pub use storage::Storage;

/// Application state shared across Tauri commands
pub struct AppState {
    pub config: Config,
    pub storage: Arc<RwLock<Option<Storage>>>,
    pub search: Arc<RwLock<Option<Index>>>,
    /// Shared indexer config with thread pool - use Mutex to serialize indexing operations
    pub indexer_config: Arc<Mutex<IndexerConfig>>,
    /// Custom embedder for semantic search (None = full-text only)
    pub embedder: Arc<RwLock<Option<Embedder>>>,
    /// Currently configured embedding model ID (None = no embeddings)
    pub embedding_model_id: Arc<RwLock<Option<String>>>,
    /// Loaded LLM model for agent
    pub agent_model: Arc<RwLock<Option<AgentModel>>>,
    /// Active conversations
    pub conversations: Arc<RwLock<HashMap<String, Conversation>>>,
    /// Cancellation tokens for active generations
    pub active_generations: Arc<RwLock<HashMap<String, CancellationToken>>>,
}

impl AppState {
    pub fn new() -> Self {
        let config = Config::load_or_default();
        config
            .ensure_dirs()
            .expect("Failed to create data directories");

        // Create a shared IndexerConfig with thread pool for all indexing operations
        let indexer_config = IndexerConfig::default();

        Self {
            config,
            storage: Arc::new(RwLock::new(None)),
            search: Arc::new(RwLock::new(None)),
            indexer_config: Arc::new(Mutex::new(indexer_config)),
            embedder: Arc::new(RwLock::new(None)), // Configured later via settings
            embedding_model_id: Arc::new(RwLock::new(None)),
            agent_model: Arc::new(RwLock::new(None)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
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

        // Note: Embedding model is configured lazily on first use via configure_embedding_model command
        // This avoids blocking startup with slow model loading
        let settings = Settings::load(&self.config.settings_file);
        if settings.embedding_model_id.is_some() {
            tracing::info!("Embedding model configured in settings, will load on first use");
        }

        tracing::info!("AppState initialized");
        Ok(())
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
