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
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use milli::update::IndexerConfig;
use milli::Index;
use serde::{Deserialize, Serialize};

pub use embeddings::Embedder;

/// Model type identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Embedding,
    Language,
}

/// Model status for frontend synchronization
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ModelStatus {
    /// Model is being downloaded
    Downloading {
        model_type: ModelType,
        model_id: String,
        model_name: String,
    },
    /// Model is being loaded into memory
    Loading {
        model_type: ModelType,
        model_id: String,
        model_name: String,
    },
    /// Model is ready for use
    Ready {
        model_type: ModelType,
        model_id: String,
    },
    /// Model failed to download or load
    Failed {
        model_type: ModelType,
        model_id: String,
        error: String,
    },
}

/// Download progress with model type
#[derive(Debug, Clone, Serialize)]
pub struct ModelDownloadProgress {
    pub model_type: ModelType,
    #[serde(flatten)]
    pub progress: models::DownloadProgress,
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
pub use jobs::{
    import_and_index_pdf, process_document, spawn_index_worker, IndexWorkerHandle, SyncWatcher,
};
pub use storage::{EmbeddingChunk, EmbeddingData, Storage};

/// Application state shared across Tauri commands
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Model manager for downloading and checking model status
    pub model_manager: Arc<models::ModelManager>,
    /// Storage - initialized in setup(), always available to commands
    pub storage: Arc<RwLock<Storage>>,
    /// Search index - shared for reads, writes go through index worker
    /// Note: No RwLock needed since LMDB handles read concurrency internally
    pub search: Arc<Index>,
    /// Index worker handle for search write operations (indexing, deletion)
    pub index_worker: IndexWorkerHandle,
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
    /// Sync watchers for collections (handles documents from peers)
    sync_watchers: Arc<RwLock<HashMap<iroh_docs::NamespaceId, SyncWatcher>>>,
    /// Master cancellation token for all sync watchers
    sync_cancel: CancellationToken,
}

impl AppState {
    /// Create AppState with initialized storage and search.
    /// Called from setup() where Tauri's async runtime is available.
    pub async fn new(config: Config) -> anyhow::Result<Self> {
        // Initialize model manager (HuggingFace cache + API client)
        let model_manager = Arc::new(models::ModelManager::new().await?);

        // Fast async init - just opens files
        let storage = Storage::open(&config.iroh_dir).await?;

        // Sync init - create index and indexer config
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

        // Wrap in Arc for sharing
        let storage = Arc::new(RwLock::new(storage));
        let search = Arc::new(index);
        let embedder = Arc::new(RwLock::new(None));
        let embedding_model_id = Arc::new(RwLock::new(None));

        // Spawn index worker - handles all milli write operations in a dedicated thread
        // The worker is automatically stopped when all handles are dropped
        let index_worker = spawn_index_worker(search.clone(), indexer_config);

        Ok(Self {
            config,
            model_manager,
            storage,
            search,
            index_worker,
            embedder,
            embedding_model_id,
            chat_provider: Arc::new(RwLock::new(None)),
            provider_config: Arc::new(RwLock::new(None)),
            conversations: Arc::new(RwLock::new(HashMap::new())),
            active_generations: Arc::new(RwLock::new(HashMap::new())),
            active_predictions: Arc::new(RwLock::new(HashMap::new())),
            sync_watchers: Arc::new(RwLock::new(HashMap::new())),
            sync_cancel: CancellationToken::new(),
        })
    }

    /// Load configured models on startup.
    /// Called in background after setup() completes.
    ///
    /// Auto-downloads the default embedding model if not configured.
    /// Sends status changes to `status_tx` and download progress to `progress_tx`.
    pub async fn load_models_if_configured(
        &self,
        status_tx: tokio::sync::mpsc::Sender<ModelStatus>,
        progress_tx: tokio::sync::mpsc::Sender<ModelDownloadProgress>,
    ) {
        let mut settings = Settings::load(&self.config.settings_file);

        // Start watching existing collections for indexing events
        self.watch_existing_collections().await;

        // Load chat provider if configured
        if let Some(ref provider_config) = settings.provider {
            self.load_provider_from_config(provider_config).await;
        }

        // Auto-configure default embedding model if not set
        let model_id = match settings.embedding_model_id.clone() {
            Some(id) => id,
            None => {
                let default_model = models::default_embedding_model();
                tracing::info!(
                    "No embedding model configured, using default: {}",
                    default_model.id
                );
                settings.embedding_model_id = Some(default_model.id.clone());
                if let Err(e) = settings.save(&self.config.settings_file) {
                    tracing::warn!("Failed to save default embedding model setting: {}", e);
                }
                default_model.id
            }
        };

        // Get model info
        let model = match models::get_embedding_model(&model_id) {
            Some(m) => m,
            None => {
                tracing::error!("Unknown embedding model: {}", model_id);
                let _ = status_tx
                    .send(ModelStatus::Failed {
                        model_type: ModelType::Embedding,
                        model_id: model_id.clone(),
                        error: format!("Unknown embedding model: {}", model_id),
                    })
                    .await;
                return;
            }
        };

        // Download if not cached
        if !self.model_manager.is_downloaded(&model) {
            if let Err(e) = self
                .model_manager
                .download(&model, ModelType::Embedding, status_tx.clone(), progress_tx)
                .await
            {
                tracing::error!("Failed to download embedding model: {}", e);
                return;
            }
        }

        // Load model into memory
        let _ = status_tx
            .send(ModelStatus::Loading {
                model_type: ModelType::Embedding,
                model_id: model_id.clone(),
                model_name: model.name.clone(),
            })
            .await;

        tracing::info!("Loading embedding model '{}'...", model_id);
        match Embedder::new(&model.hf_repo_id, model.dimensions).await {
            Ok(emb) => {
                *self.embedder.write().await = Some(emb);
                *self.embedding_model_id.write().await = Some(model_id.clone());

                tracing::info!("Embedding model '{}' loaded", model_id);
                let _ = status_tx
                    .send(ModelStatus::Ready {
                        model_type: ModelType::Embedding,
                        model_id: model_id.clone(),
                    })
                    .await;
            }
            Err(e) => {
                tracing::error!("Failed to load embedder: {}", e);
                let _ = status_tx
                    .send(ModelStatus::Failed {
                        model_type: ModelType::Embedding,
                        model_id: model_id.clone(),
                        error: e.to_string(),
                    })
                    .await;
            }
        }
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
                    if self.model_manager.is_downloaded(&model) {
                        if let Some(path) = self.model_manager.get_path(&model) {
                            match LocalProvider::load(&path, &model).await {
                                Ok(provider) => {
                                    *self.chat_provider.write().await = Some(Box::new(provider));
                                    *self.provider_config.write().await = Some(config.clone());
                                    tracing::info!("Loaded local provider: {}", model_id);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to load local provider: {}", e);
                                }
                            }
                        }
                    } else {
                        tracing::info!("Local model '{}' not downloaded, skipping", model_id);
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

    /// Start watching all existing collections for sync events.
    ///
    /// This watches for documents arriving from peers (InsertRemote events).
    /// Local imports are processed directly without events.
    async fn watch_existing_collections(&self) {
        let collections = {
            let storage = self.storage.read().await;
            match storage.list_collections().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to list collections for watching: {}", e);
                    return;
                }
            }
        };

        for (namespace_id, metadata) in &collections {
            self.watch_namespace(*namespace_id).await;
            tracing::debug!("Watching collection '{}' ({})", metadata.name, namespace_id);
        }

        tracing::info!(
            "Started watching {} existing collections for sync",
            collections.len()
        );
    }

    /// Start watching a namespace for sync events (documents from peers).
    ///
    /// This is only needed for remote sync. Local imports are processed directly.
    pub async fn watch_namespace(&self, namespace_id: iroh_docs::NamespaceId) {
        let mut watchers = self.sync_watchers.write().await;

        if watchers.contains_key(&namespace_id) {
            tracing::debug!(namespace = %namespace_id, "Already watching namespace");
            return;
        }

        let watcher = SyncWatcher::spawn(
            namespace_id,
            self.storage.clone(),
            self.embedder.clone(),
            self.embedding_model_id.clone(),
            self.index_worker.clone(),
            self.sync_cancel.clone(),
        );

        watchers.insert(namespace_id, watcher);
        tracing::debug!(namespace = %namespace_id, "Started sync watcher");
    }

    /// Stop watching a namespace for sync events.
    pub async fn unwatch_namespace(&self, namespace_id: &iroh_docs::NamespaceId) {
        let mut watchers = self.sync_watchers.write().await;
        if let Some(watcher) = watchers.remove(namespace_id) {
            watcher.stop();
            tracing::debug!(namespace = %namespace_id, "Stopped sync watcher");
        }
    }
}
