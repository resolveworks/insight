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
    /// Embedding model is being loaded (only if configured)
    EmbedderLoading { model_id: String, model_name: String },
    /// Embedding model loaded successfully
    EmbedderReady { model_id: String },
    /// Embedding model failed to load
    EmbedderFailed { model_id: String, error: String },
}

pub use agent::{AgentEvent, AgentModel, Conversation};
pub use config::{Config, Settings};
pub use storage::Storage;

/// Application state shared across Tauri commands
pub struct AppState {
    pub config: Config,
    /// Storage is always initialized before commands can be called
    pub storage: Arc<RwLock<Storage>>,
    /// Search index is always initialized before commands can be called
    pub search: Arc<RwLock<Index>>,
    /// Shared indexer config with thread pool - use Mutex to serialize indexing operations
    pub indexer_config: Arc<Mutex<IndexerConfig>>,
    /// Custom embedder for semantic search (None = full-text only, loaded async)
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
    /// Create and initialize AppState. Blocks on async storage initialization.
    pub fn new() -> Self {
        let config = Config::load_or_default();
        config
            .ensure_dirs()
            .expect("Failed to create data directories");

        // Create a temporary runtime to initialize storage
        let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");

        let storage = rt.block_on(async {
            Storage::open(&config.iroh_dir)
                .await
                .expect("Failed to open storage")
        });

        let index = search::open_index(&config.search_dir).expect("Failed to open search index");

        // Configure embedder in index if previously set
        let settings = Settings::load(&config.settings_file);
        let indexer_config = IndexerConfig::default();

        if let Some(ref model_id) = settings.embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
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

        Self {
            config,
            storage: Arc::new(RwLock::new(storage)),
            search: Arc::new(RwLock::new(index)),
            indexer_config: Arc::new(Mutex::new(indexer_config)),
            embedder: Arc::new(RwLock::new(None)),
            embedding_model_id: Arc::new(RwLock::new(None)),
            agent_model: Arc::new(RwLock::new(None)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
        }
    }

}

/// Load embedding model if configured in settings (call in background after startup)
pub async fn load_embedder_if_configured(
    config: &Config,
    embedder: &RwLock<Option<Embedder>>,
    embedding_model_id: &RwLock<Option<String>>,
    app_handle: &AppHandle,
) {
    let settings = Settings::load(&config.settings_file);

    if let Some(ref model_id) = settings.embedding_model_id {
        if let Some(model) = models::get_embedding_model(model_id) {
            let _ = app_handle.emit(
                "boot-phase",
                BootPhase::EmbedderLoading {
                    model_id: model_id.clone(),
                    model_name: model.name.to_string(),
                },
            );

            tracing::info!("Loading embedding model '{}' on boot...", model_id);
            match Embedder::new(&model.hf_repo_id, model.dimensions).await {
                Ok(emb) => {
                    *embedder.write().await = Some(emb);
                    *embedding_model_id.write().await = Some(model_id.clone());

                    tracing::info!("Embedding model '{}' loaded successfully", model_id);
                    let _ = app_handle.emit(
                        "boot-phase",
                        BootPhase::EmbedderReady {
                            model_id: model_id.clone(),
                        },
                    );
                }
                Err(e) => {
                    tracing::error!("Failed to load embedder on boot: {}", e);
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

    let _ = app_handle.emit("backend-ready", ());
    tracing::info!("Backend ready");
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}
