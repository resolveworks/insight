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
use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub use embeddings::Embedder;

/// Boot phase events for frontend synchronization
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase")]
pub enum BootPhase {
    /// Storage and search initialized, ready for basic operations
    StorageReady {
        /// Whether an embedding model is configured in settings
        embedding_configured: bool,
        /// The configured model ID (if any)
        embedding_model_id: Option<String>,
    },
    /// Embedding model is being loaded (only if configured)
    EmbedderLoading { model_id: String, model_name: String },
    /// Embedding model loaded successfully
    EmbedderReady { model_id: String },
    /// Embedding model failed to load
    EmbedderFailed { model_id: String, error: String },
    /// Application fully ready
    AppReady,
}

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
    pub async fn initialize(&self, app_handle: &AppHandle) -> anyhow::Result<()> {
        // Phase 1: Initialize storage
        let storage = Storage::open(&self.config.iroh_dir).await?;
        *self.storage.write().await = Some(storage);

        // Initialize search index
        let index = search::open_index(&self.config.search_dir)?;

        // Read settings to determine if embedding is configured
        let settings = Settings::load(&self.config.settings_file);
        let embedding_configured = settings.embedding_model_id.is_some();
        let embedding_model_id = settings.embedding_model_id.clone();

        // Configure milli index metadata if model was previously set
        if let Some(ref model_id) = embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
                let indexer_config = self.indexer_config.lock().await;
                if let Err(e) =
                    search::configure_embedder(&index, &indexer_config, "default", model.dimensions)
                {
                    tracing::warn!("Failed to configure embedder in index on startup: {}", e);
                } else {
                    tracing::info!(
                        "Configured embedder '{}' ({}D) in search index",
                        model_id,
                        model.dimensions
                    );
                }
            }
        }

        // Log current embedder configs for debugging
        if let Err(e) = search::log_embedder_configs(&index) {
            tracing::warn!("Failed to log embedder configs: {}", e);
        }

        *self.search.write().await = Some(index);

        // Emit Phase 1: Storage ready
        if let Err(e) = app_handle.emit(
            "boot-phase",
            BootPhase::StorageReady {
                embedding_configured,
                embedding_model_id: embedding_model_id.clone(),
            },
        ) {
            tracing::warn!("Failed to emit StorageReady event: {}", e);
        }

        // Phase 2: Load embedder if configured
        if let Some(ref model_id) = embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
                // Emit loading start
                if let Err(e) = app_handle.emit(
                    "boot-phase",
                    BootPhase::EmbedderLoading {
                        model_id: model_id.clone(),
                        model_name: model.name.to_string(),
                    },
                ) {
                    tracing::warn!("Failed to emit EmbedderLoading event: {}", e);
                }

                // Load the embedder (this is the slow part, 20-30 seconds)
                tracing::info!("Loading embedding model '{}' on boot...", model_id);
                match Embedder::new(&model.hf_repo_id, model.dimensions).await {
                    Ok(embedder) => {
                        *self.embedder.write().await = Some(embedder);
                        *self.embedding_model_id.write().await = Some(model_id.clone());

                        tracing::info!("Embedding model '{}' loaded successfully", model_id);
                        if let Err(e) = app_handle.emit(
                            "boot-phase",
                            BootPhase::EmbedderReady {
                                model_id: model_id.clone(),
                            },
                        ) {
                            tracing::warn!("Failed to emit EmbedderReady event: {}", e);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to load embedder on boot: {}", e);
                        if let Err(emit_err) = app_handle.emit(
                            "boot-phase",
                            BootPhase::EmbedderFailed {
                                model_id: model_id.clone(),
                                error: e.to_string(),
                            },
                        ) {
                            tracing::warn!("Failed to emit EmbedderFailed event: {}", emit_err);
                        }
                    }
                }
            }
        }

        // Phase 3: App fully ready
        if let Err(e) = app_handle.emit("boot-phase", BootPhase::AppReady) {
            tracing::warn!("Failed to emit AppReady event: {}", e);
        }

        // Keep legacy event for backward compatibility
        if let Err(e) = app_handle.emit("backend-ready", ()) {
            tracing::warn!("Failed to emit backend-ready event: {}", e);
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
