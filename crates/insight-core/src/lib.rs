//! Insight Core - Business logic for document management and search
//!
//! This crate contains all the core functionality for Insight, including:
//! - Document storage (iroh P2P)
//! - Full-text and semantic search (milli)
//! - PDF text extraction (lopdf)
//! - Inference providers (local + remote) via [`provider`] + [`manager::ModelManager`]
//! - Agent/conversation handling
//! - Event-driven pipeline for document import

pub mod agent;
pub mod config;
pub mod conversations;
pub mod manager;
pub mod models;
pub mod pdf;
pub mod pipeline;
pub mod provider;
pub mod search;
pub mod storage;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use milli::update::IndexerConfig;
use milli::Index;
use serde::{Deserialize, Serialize};

/// Model role identifier used for status events and downloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Embedding,
    Language,
    Ocr,
}

/// Collection info - canonical definition used across API responses and agent context
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollectionInfo {
    pub id: String,
    pub name: String,
    /// Number of documents in this collection
    #[serde(default)]
    pub document_count: usize,
    /// Total pages across all documents
    #[serde(default)]
    pub total_pages: usize,
    /// When the collection was created (ISO 8601)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
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
    /// Model is ready for use (configured; weights may or may not be resident)
    Ready {
        model_type: ModelType,
        model_id: String,
    },
    /// Model weights were unloaded from memory (idle reaper, eviction).
    /// The provider stays configured; the next request reloads.
    Unloaded {
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

pub use agent::{AgentContext, AgentEvent, Conversation};
pub use config::{Config, LifecycleConfig, Settings};
pub use manager::{ChatLease, EmbeddingLease, ModelManager, OcrLease};
pub use pipeline::{Pipeline, PipelineProgress, StageProgress};
pub use provider::{
    get_provider_families, get_tool_definitions, AnthropicChatProvider, ChatProvider,
    CompletedToolCall, CompletionResult, EmbeddingProvider, LocalChatProvider,
    LocalEmbeddingProvider, LocalOcrProvider, OcrProvider, OpenAIChatProvider, ProviderConfig,
    ProviderEvent, ProviderFamily, RemoteModelInfo, ToolDefinition,
};
pub use search::{spawn_index_worker, IndexWorkerHandle};
pub use storage::{EmbeddingChunk, EmbeddingData, Storage};

/// Application state shared across Tauri commands
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Downloads and caches HuggingFace model files on disk.
    pub model_downloader: Arc<models::ModelDownloader>,
    /// Central manager for in-memory inference providers (chat, embed, OCR).
    pub models: Arc<ModelManager>,
    /// Storage - initialized in setup(), always available to commands
    pub storage: Arc<RwLock<Storage>>,
    /// Search index - shared for reads, writes go through index worker
    /// Note: No RwLock needed since LMDB handles read concurrency internally
    pub search: Arc<Index>,
    /// Index worker handle for search write operations (indexing, deletion)
    pub index_worker: IndexWorkerHandle,
    /// Active conversations
    pub conversations: Arc<RwLock<HashMap<String, Conversation>>>,
    /// Cancellation tokens for active generations
    pub active_generations: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Cancellation tokens for active predictions (tab completion)
    pub active_predictions: Arc<RwLock<HashMap<String, CancellationToken>>>,
    /// Event-driven document processing pipeline
    pub pipeline: Arc<Pipeline>,
}

impl AppState {
    /// Create AppState with initialized storage and search.
    /// Called from setup() where Tauri's async runtime is available.
    ///
    /// Returns the state and a receiver for pipeline progress events
    /// that should be forwarded to the frontend.
    pub async fn new(
        config: Config,
    ) -> anyhow::Result<(Self, tokio::sync::mpsc::Receiver<PipelineProgress>)> {
        let model_downloader = Arc::new(models::ModelDownloader::new().await?);
        let models = Arc::new(ModelManager::new());

        // Fast async init - just opens files
        let storage = Storage::open(&config.iroh_dir).await?;

        // Sync init - create index and indexer config
        let index = search::open_index(&config.search_dir)?;
        let indexer_config = IndexerConfig::default();

        let storage = Arc::new(RwLock::new(storage));
        let search = Arc::new(index);

        // Spawn index worker - handles all milli write operations in a dedicated thread
        let index_worker = spawn_index_worker(search.clone(), indexer_config);

        // Create event-driven pipeline for document processing
        let (pipeline, progress_rx) =
            Pipeline::new(storage.clone(), models.clone(), index_worker.clone());

        Ok((
            Self {
                config,
                model_downloader,
                models,
                storage,
                search,
                index_worker,
                conversations: Arc::new(RwLock::new(HashMap::new())),
                active_generations: Arc::new(RwLock::new(HashMap::new())),
                active_predictions: Arc::new(RwLock::new(HashMap::new())),
                pipeline: Arc::new(pipeline),
            },
            progress_rx,
        ))
    }

    /// Restore provider configurations from settings. Called once at startup.
    ///
    /// Constructs providers without loading weights — that happens on first
    /// inference request. Auto-downloads the default embedding model if
    /// settings don't name one. The only blocking work done here is the
    /// potential download; everything else is cheap.
    pub async fn restore_configs_from_settings(
        &self,
        status_tx: tokio::sync::mpsc::Sender<ModelStatus>,
        progress_tx: tokio::sync::mpsc::Sender<ModelDownloadProgress>,
    ) {
        let mut settings = Settings::load(&self.config.settings_file);

        // Prime the manager with the current lifecycle settings so provider
        // installs pick up the right coexist flags.
        self.models
            .set_lifecycle_config(settings.lifecycle.clone())
            .await;

        // Start watching existing collections for indexing events.
        self.watch_existing_collections().await;

        // Drain any orphan OCR tasks (interrupted process, or imports
        // that landed before an OCR model was configured). Idempotent —
        // tasks with a matching text entry are skipped.
        self.pipeline.requeue_pending_ocr().await;

        // Install chat provider (no load) if configured.
        if let Some(ref provider_config) = settings.provider {
            self.install_chat_provider_from_config(provider_config, &status_tx)
                .await;
        }

        // Auto-configure default embedding model if not set.
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

        // Download if missing. Loading is deferred to first inference.
        if !self.model_downloader.is_downloaded(&model) {
            if let Err(e) = self
                .model_downloader
                .download(&model, ModelType::Embedding, status_tx.clone(), progress_tx)
                .await
            {
                tracing::error!("Failed to download embedding model: {}", e);
                return;
            }
        }

        // Configure milli's embedder entry up front so vector search paths
        // don't fail while the actual embedding model is still unloaded.
        if let Err(e) = self
            .index_worker
            .configure_embedder("default".to_string(), model.dimensions)
            .await
        {
            tracing::warn!("Failed to configure embedder in index: {}", e);
        }

        let provider = LocalEmbeddingProvider::new(&model_id, &model.hf_repo_id, model.dimensions);
        if let Err(e) = self
            .models
            .set_embedding(Arc::new(provider), model_id.clone())
            .await
        {
            tracing::error!("Failed to install embedding provider: {}", e);
            let _ = status_tx
                .send(ModelStatus::Failed {
                    model_type: ModelType::Embedding,
                    model_id: model_id.clone(),
                    error: e.to_string(),
                })
                .await;
            return;
        }

        tracing::info!("Embedding provider '{}' installed (lazy)", model_id);
        let _ = status_tx
            .send(ModelStatus::Ready {
                model_type: ModelType::Embedding,
                model_id: model_id.clone(),
            })
            .await;
    }

    /// Install a chat provider from saved configuration without loading
    /// weights. The first inference request pays the load cost.
    async fn install_chat_provider_from_config(
        &self,
        config: &ProviderConfig,
        status_tx: &tokio::sync::mpsc::Sender<ModelStatus>,
    ) {
        match config {
            ProviderConfig::Local { model_id } => {
                let Some(model) = models::get_language_model(model_id) else {
                    tracing::warn!("Unknown local model: {}", model_id);
                    return;
                };
                if !self.model_downloader.is_downloaded(&model) {
                    tracing::info!("Local model '{}' not downloaded, skipping", model_id);
                    return;
                }
                let Some(path) = self.model_downloader.get_path(&model) else {
                    return;
                };

                let provider = LocalChatProvider::new(&path, &model);
                if let Err(e) = self
                    .models
                    .set_chat(Arc::new(provider), config.clone())
                    .await
                {
                    tracing::error!("Failed to install chat provider: {}", e);
                    let _ = status_tx
                        .send(ModelStatus::Failed {
                            model_type: ModelType::Language,
                            model_id: model_id.clone(),
                            error: e.to_string(),
                        })
                        .await;
                    return;
                }
                tracing::info!("Local chat provider '{}' installed (lazy)", model_id);
                let _ = status_tx
                    .send(ModelStatus::Ready {
                        model_type: ModelType::Language,
                        model_id: model_id.clone(),
                    })
                    .await;
            }
            ProviderConfig::OpenAI { api_key, model } => {
                let provider = OpenAIChatProvider::new(api_key, model);
                if let Err(e) = self
                    .models
                    .set_chat(Arc::new(provider), config.clone())
                    .await
                {
                    tracing::error!("Failed to install OpenAI provider: {}", e);
                    return;
                }
                tracing::info!("Loaded OpenAI provider: {}", model);
            }
            ProviderConfig::Anthropic { api_key, model } => {
                let provider = AnthropicChatProvider::new(api_key, model);
                if let Err(e) = self
                    .models
                    .set_chat(Arc::new(provider), config.clone())
                    .await
                {
                    tracing::error!("Failed to install Anthropic provider: {}", e);
                    return;
                }
                tracing::info!("Loaded Anthropic provider: {}", model);
            }
        }
    }

    /// Start watching all existing collections for pipeline events.
    ///
    /// This enables the event-driven pipeline for all existing collections.
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
            self.pipeline.watch(*namespace_id).await;
            tracing::debug!("Watching collection '{}' ({})", metadata.name, namespace_id);
        }

        tracing::info!(
            "Started watching {} existing collections",
            collections.len()
        );
    }

    /// Start watching a namespace for pipeline events.
    pub async fn watch_namespace(&self, namespace_id: iroh_docs::NamespaceId) {
        self.pipeline.watch(namespace_id).await;
    }

    /// Stop watching a namespace.
    pub async fn unwatch_namespace(&self, namespace_id: &iroh_docs::NamespaceId) {
        self.pipeline.unwatch(namespace_id).await;
    }
}
