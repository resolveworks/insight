//! Insight Core - Business logic for document management and search
//!
//! This crate contains all the core functionality for Insight, including:
//! - Document storage (iroh P2P)
//! - Full-text search (milli)
//! - PDF text extraction (lopdf)
//! - Remote inference providers (OpenAI, Anthropic) via [`provider`] +
//!   [`manager::ModelManager`]
//! - Agent/conversation handling
//! - Event-driven pipeline for document import

pub mod agent;
pub mod config;
pub mod conversations;
pub mod manager;
pub mod memory;
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

/// Model role identifier used for status events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelType {
    Language,
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
    /// Model was unloaded from memory. The provider stays configured;
    /// the next request reloads.
    Unloaded {
        model_type: ModelType,
        model_id: String,
    },
    /// Model failed to load
    Failed {
        model_type: ModelType,
        model_id: String,
        error: String,
    },
}

pub use agent::{AgentContext, AgentEvent, Conversation};
pub use config::{Config, Settings};
pub use manager::{ChatLease, ModelManager};
pub use pipeline::{Pipeline, PipelineProgress, StageProgress};
pub use provider::{
    get_provider_families, get_tool_definitions, AnthropicChatProvider, ChatProvider,
    CompletedToolCall, CompletionResult, OpenAIChatProvider, ProviderConfig, ProviderEvent,
    ProviderFamily, RemoteModelInfo, ToolDefinition,
};
pub use search::{spawn_index_worker, IndexWorkerHandle};
pub use storage::Storage;

/// Application state shared across Tauri commands
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Central manager for the chat provider slot.
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
        let (pipeline, progress_rx) = Pipeline::new(storage.clone(), index_worker.clone());

        Ok((
            Self {
                config,
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
    /// Installs the configured chat provider (if any) without any blocking
    /// work — remote providers are ready immediately.
    pub async fn restore_configs_from_settings(&self) {
        let settings = Settings::load(&self.config.settings_file);

        // Start watching existing collections for indexing events.
        self.watch_existing_collections().await;

        // Install chat provider if configured.
        if let Some(ref provider_config) = settings.provider {
            self.install_chat_provider_from_config(provider_config)
                .await;
        }
    }

    /// Install a chat provider from saved configuration without loading
    /// weights. The first inference request pays the load cost.
    async fn install_chat_provider_from_config(&self, config: &ProviderConfig) {
        match config {
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
