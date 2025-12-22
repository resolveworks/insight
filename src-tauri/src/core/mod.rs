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
    /// Storage and search index initialized, ready to check model configuration
    StorageReady {
        embedding_configured: bool,
        embedding_model_id: Option<String>,
    },
    /// Embedding model is being loaded (only if configured)
    EmbedderLoading {
        model_id: String,
        model_name: String,
    },
    /// Embedding model loaded successfully
    EmbedderReady { model_id: String },
    /// Embedding model failed to load
    EmbedderFailed { model_id: String, error: String },
    /// All models loaded, app is ready
    AppReady,
}

pub use agent::{AgentEvent, AgentModel, Conversation};
pub use config::{Config, Settings};
pub use storage::Storage;

/// Application state shared across Tauri commands
pub struct AppState {
    pub config: Config,
    /// Storage - initialized in setup(), always available to commands
    pub storage: Arc<RwLock<Storage>>,
    /// Search index - initialized in setup(), always available to commands
    pub search: Arc<RwLock<Index>>,
    /// Shared indexer config with thread pool
    pub indexer_config: Arc<Mutex<IndexerConfig>>,
    /// Custom embedder for semantic search (None = full-text only, loaded async)
    pub embedder: Arc<RwLock<Option<Embedder>>>,
    /// Currently configured embedding model ID
    pub embedding_model_id: Arc<RwLock<Option<String>>>,
    /// Loaded language model for agent
    pub agent_model: Arc<RwLock<Option<AgentModel>>>,
    /// Currently configured language model ID
    pub language_model_id: Arc<RwLock<Option<String>>>,
    /// Active conversations
    pub conversations: Arc<RwLock<HashMap<String, Conversation>>>,
    /// Cancellation tokens for active generations
    pub active_generations: Arc<RwLock<HashMap<String, CancellationToken>>>,
}

impl AppState {
    /// Create AppState with initialized storage and search.
    /// Called from setup() where Tauri's async runtime is available.
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        // Fast async init - just opens files
        let storage = Storage::open(&config.iroh_dir).await?;

        // Sync init
        let index = search::open_index(&config.search_dir)?;
        let indexer_config = IndexerConfig::default();

        // Configure embedder in search index if previously set
        let settings = Settings::load(&config.settings_file);
        if let Some(ref model_id) = settings.embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
                if let Err(e) =
                    search::configure_embedder(&index, &indexer_config, "default", model.dimensions)
                {
                    tracing::warn!("Failed to configure embedder in index: {}", e);
                } else {
                    tracing::info!(
                        "Configured embedder '{}' ({}D) in search index",
                        model_id,
                        model.dimensions
                    );
                }
            }
        }

        Ok(Self {
            config,
            storage: Arc::new(RwLock::new(storage)),
            search: Arc::new(RwLock::new(index)),
            indexer_config: Arc::new(Mutex::new(indexer_config)),
            embedder: Arc::new(RwLock::new(None)),
            embedding_model_id: Arc::new(RwLock::new(None)),
            agent_model: Arc::new(RwLock::new(None)),
            language_model_id: Arc::new(RwLock::new(None)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Load configured models on startup.
    /// Called in background after setup() completes.
    pub async fn load_models_if_configured(&self, app_handle: &AppHandle) {
        let settings = Settings::load(&self.config.settings_file);

        // Emit StorageReady so frontend knows initialization is complete
        let _ = app_handle.emit(
            "boot-phase",
            BootPhase::StorageReady {
                embedding_configured: settings.embedding_model_id.is_some(),
                embedding_model_id: settings.embedding_model_id.clone(),
            },
        );

        // Language model is loaded lazily when chat is opened (see ensure_language_model_loaded)

        // Load embedding model if configured
        if let Some(ref model_id) = settings.embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
                let _ = app_handle.emit(
                    "boot-phase",
                    BootPhase::EmbedderLoading {
                        model_id: model_id.clone(),
                        model_name: model.name.clone(),
                    },
                );

                tracing::info!("Loading embedding model '{}'...", model_id);
                match Embedder::new(&model.hf_repo_id, model.dimensions).await {
                    Ok(emb) => {
                        *self.embedder.write().await = Some(emb);
                        *self.embedding_model_id.write().await = Some(model_id.clone());

                        tracing::info!("Embedding model '{}' loaded", model_id);
                        let _ = app_handle.emit(
                            "boot-phase",
                            BootPhase::EmbedderReady {
                                model_id: model_id.clone(),
                            },
                        );
                    }
                    Err(e) => {
                        tracing::error!("Failed to load embedder: {}", e);
                        let _ = app_handle.emit(
                            "boot-phase",
                            BootPhase::EmbedderFailed {
                                model_id: model_id.clone(),
                                error: e.to_string(),
                            },
                        );
                    }
                }
            }
        }

        let _ = app_handle.emit("boot-phase", BootPhase::AppReady);
        tracing::info!("Backend ready");
    }
}
