//! Insight Core - Business logic for document management and search
//!
//! This crate contains all the core functionality for Insight, including:
//! - Document storage (iroh P2P)
//! - Full-text and semantic search (milli)
//! - PDF text extraction (lopdf)
//! - Embedding generation (mistralrs)
//! - Agent/conversation handling
//! - Job pipeline for document import

pub mod agent;
pub mod config;
pub mod conversations;
pub mod embeddings;
pub mod jobs;
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

pub use embeddings::Embedder;

/// Boot phase events for frontend synchronization
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "phase")]
pub enum BootPhase {
    /// Starting collection watchers
    WatchingCollections,
    /// Storage and search index initialized, ready to check model configuration
    StorageReady {
        embedding_configured: bool,
        embedding_model_id: Option<String>,
        embedding_downloaded: bool,
    },
    /// Embedding model needs to be downloaded before loading
    EmbedderDownloadRequired {
        model_id: String,
        model_name: String,
    },
    /// Embedding model is being loaded
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

/// Trait for emitting boot phase events.
///
/// Implement this trait to receive boot phase notifications during app startup.
pub trait BootPhaseEmitter: Send + Sync {
    fn emit_boot_phase(&self, phase: BootPhase);
}

/// No-op implementation for testing
pub struct NoOpEmitter;

impl BootPhaseEmitter for NoOpEmitter {
    fn emit_boot_phase(&self, _phase: BootPhase) {}
}

pub use agent::provider::anthropic::AnthropicProvider;
pub use agent::provider::local::LocalProvider;
pub use agent::provider::openai::OpenAIProvider;
pub use agent::provider::{
    get_provider_families, get_tool_definitions, ChatProvider, CompletedToolCall, CompletionResult,
    ProviderConfig, ProviderEvent, ProviderFamily, RemoteModelInfo, ToolDefinition,
};
pub use agent::{AgentContext, AgentEvent, CollectionInfo, Conversation};
pub use config::{Config, Settings};
pub use jobs::JobCoordinator;
pub use storage::Storage;

/// Check if embedding model is configured and downloaded.
/// Returns (configured, downloaded).
pub async fn check_embedding_status(settings: &Settings) -> (bool, bool) {
    let Some(ref model_id) = settings.embedding_model_id else {
        return (false, false);
    };
    let Some(model) = models::get_embedding_model(model_id) else {
        return (false, false);
    };
    let downloaded = match models::ModelManager::new().await {
        Ok(manager) => manager.is_downloaded(&model),
        Err(e) => {
            tracing::warn!("Failed to create model manager: {}", e);
            false
        }
    };
    (true, downloaded)
}

/// Application state shared across Tauri commands
#[derive(Clone)]
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
    /// Active chat provider (local, OpenAI, or Anthropic)
    pub chat_provider: Arc<RwLock<Option<Box<dyn ChatProvider>>>>,
    /// Current provider configuration (for persistence and display)
    pub provider_config: Arc<RwLock<Option<ProviderConfig>>>,
    /// Active conversations
    pub conversations: Arc<RwLock<HashMap<String, Conversation>>>,
    /// Cancellation tokens for active generations
    pub active_generations: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Cancellation tokens for active predictions (tab completion)
    pub active_predictions: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Job coordinator for document import pipeline
    pub job_coordinator: Arc<RwLock<Option<JobCoordinator>>>,
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

        let storage = Arc::new(RwLock::new(storage));
        let search = Arc::new(RwLock::new(index));
        let indexer_config = Arc::new(Mutex::new(indexer_config));
        let embedder = Arc::new(RwLock::new(None));

        // Create job coordinator with shared resources
        let job_coordinator = JobCoordinator::new(
            storage.clone(),
            embedder.clone(),
            search.clone(),
            indexer_config.clone(),
        );

        Ok(Self {
            config,
            storage,
            search,
            indexer_config,
            embedder,
            embedding_model_id: Arc::new(RwLock::new(None)),
            chat_provider: Arc::new(RwLock::new(None)),
            provider_config: Arc::new(RwLock::new(None)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
            active_predictions: Arc::new(RwLock::new(HashMap::new())),
            job_coordinator: Arc::new(RwLock::new(Some(job_coordinator))),
        })
    }

    /// Load configured models on startup.
    /// Called in background after setup() completes.
    ///
    /// Uses the provided emitter to send boot phase events.
    pub async fn load_models_if_configured<E: BootPhaseEmitter>(&self, emitter: &E) {
        let settings = Settings::load(&self.config.settings_file);

        // Emit WatchingCollections phase
        emitter.emit_boot_phase(BootPhase::WatchingCollections);

        // Start watching existing collections for indexing events
        self.watch_existing_collections().await;

        let (embedding_configured, embedding_downloaded) = check_embedding_status(&settings).await;

        // Emit StorageReady so frontend knows initialization is complete
        emitter.emit_boot_phase(BootPhase::StorageReady {
            embedding_configured,
            embedding_model_id: settings.embedding_model_id.clone(),
            embedding_downloaded,
        });

        // Load chat provider if configured
        if let Some(ref provider_config) = settings.provider {
            self.load_provider_from_config(provider_config).await;
        }

        // Load embedding model if configured AND downloaded
        if let Some(ref model_id) = settings.embedding_model_id {
            if let Some(model) = models::get_embedding_model(model_id) {
                if !embedding_downloaded {
                    // Model needs to be downloaded - emit event and let frontend handle it
                    tracing::info!(
                        "Embedding model '{}' not downloaded, waiting for user action",
                        model_id
                    );
                    emitter.emit_boot_phase(BootPhase::EmbedderDownloadRequired {
                        model_id: model_id.clone(),
                        model_name: model.name.clone(),
                    });
                    // Don't emit AppReady - frontend will trigger download and reload
                    return;
                }

                // Model is downloaded, proceed to load it
                emitter.emit_boot_phase(BootPhase::EmbedderLoading {
                    model_id: model_id.clone(),
                    model_name: model.name.clone(),
                });

                tracing::info!("Loading embedding model '{}'...", model_id);
                match Embedder::new(&model.hf_repo_id, model.dimensions).await {
                    Ok(emb) => {
                        *self.embedder.write().await = Some(emb);
                        *self.embedding_model_id.write().await = Some(model_id.clone());

                        tracing::info!("Embedding model '{}' loaded", model_id);
                        emitter.emit_boot_phase(BootPhase::EmbedderReady {
                            model_id: model_id.clone(),
                        });
                    }
                    Err(e) => {
                        tracing::error!("Failed to load embedder: {}", e);
                        emitter.emit_boot_phase(BootPhase::EmbedderFailed {
                            model_id: model_id.clone(),
                            error: e.to_string(),
                        });
                    }
                }
            }
        }

        emitter.emit_boot_phase(BootPhase::AppReady);
        tracing::info!("Backend ready");
    }

    /// Load a chat provider from saved configuration.
    async fn load_provider_from_config(&self, config: &ProviderConfig) {
        use agent::provider::{
            anthropic::AnthropicProvider, local::LocalProvider, openai::OpenAIProvider,
        };

        match config {
            ProviderConfig::Local { model_id } => {
                // For local models, we need to check if it's downloaded
                if let Some(model) = models::get_language_model(model_id) {
                    match models::ModelManager::new().await {
                        Ok(manager) => {
                            if manager.is_downloaded(&model) {
                                if let Some(path) = manager.get_path(&model) {
                                    match LocalProvider::load(&path, &model).await {
                                        Ok(provider) => {
                                            *self.chat_provider.write().await =
                                                Some(Box::new(provider));
                                            *self.provider_config.write().await =
                                                Some(config.clone());
                                            tracing::info!("Loaded local provider: {}", model_id);
                                        }
                                        Err(e) => {
                                            tracing::error!("Failed to load local provider: {}", e);
                                        }
                                    }
                                }
                            } else {
                                tracing::info!(
                                    "Local model '{}' not downloaded, skipping",
                                    model_id
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to create model manager: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("Unknown local model: {}", model_id);
                }
            }
            ProviderConfig::OpenAI { api_key, model } => {
                let provider = OpenAIProvider::new(api_key, model);
                *self.chat_provider.write().await = Some(Box::new(provider));
                *self.provider_config.write().await = Some(config.clone());
                tracing::info!("Loaded OpenAI provider: {}", model);
            }
            ProviderConfig::Anthropic { api_key, model } => {
                let provider = AnthropicProvider::new(api_key, model);
                *self.chat_provider.write().await = Some(Box::new(provider));
                *self.provider_config.write().await = Some(config.clone());
                tracing::info!("Loaded Anthropic provider: {}", model);
            }
        }
    }

    /// Start watching all existing collections for document events.
    async fn watch_existing_collections(&self) {
        let storage = self.storage.read().await;
        let collections = match storage.list_collections().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to list collections for watching: {}", e);
                return;
            }
        };
        drop(storage);

        let mut coordinator_guard = self.job_coordinator.write().await;
        let coordinator = match coordinator_guard.as_mut() {
            Some(c) => c,
            None => {
                tracing::warn!("Job coordinator not available for watching collections");
                return;
            }
        };

        for (namespace_id, metadata) in collections {
            coordinator.watch_namespace(namespace_id);
            tracing::debug!("Watching collection '{}' ({})", metadata.name, namespace_id);
        }

        tracing::info!(
            "Started watching {} existing collections",
            coordinator.watcher_count()
        );
    }
}
