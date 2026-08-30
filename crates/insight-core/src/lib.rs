//! Insight Core - Business logic for document management and search
//!
//! This crate contains all the core functionality for Insight, including:
//! - Document storage (iroh P2P)
//! - Full-text search (milli)
//! - PDF text extraction (lopdf)
//! - Remote inference providers via the [`rig`] framework: the provider
//!   registry ([`providers`]), typed document tools ([`tools`]), file-backed
//!   conversation memory ([`memory`]), and the agent [`runtime`]
//! - Event-driven pipeline for document import

pub mod config;
pub mod memory;
pub mod pdf;
pub mod pipeline;
pub mod providers;
pub mod runtime;
pub mod search;
pub mod storage;
pub mod tools;

use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use milli::update::IndexerConfig;
use milli::Index;
use serde::{Deserialize, Serialize};

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

pub use config::{Config, Settings};
pub use memory::{ConversationMeta, ConversationSummary, FileConversationMemory};
pub use pipeline::{Pipeline, PipelineProgress, StageProgress};
pub use providers::{ProviderFamily, ProviderModel, ProviderSettings};
pub use runtime::{generate_title, predict_next_message, send_message, BASE_SYSTEM_PROMPT};
pub use search::{spawn_index_worker, IndexWorkerHandle};
pub use storage::Storage;

/// Application state shared across Tauri commands
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    /// Storage - initialized in setup(), always available to commands
    pub storage: Arc<RwLock<Storage>>,
    /// Search index - shared for reads, writes go through index worker
    /// Note: No RwLock needed since LMDB handles read concurrency internally
    pub search: Arc<Index>,
    /// Index worker handle for search write operations (indexing, deletion)
    pub index_worker: IndexWorkerHandle,
    /// File-backed conversation memory (messages + metadata sidecars)
    pub memory: Arc<FileConversationMemory>,
    /// Cancellation tokens for active generations
    pub active_generations: Arc<RwLock<std::collections::HashMap<String, CancellationToken>>>,
    /// Cancellation tokens for active predictions (tab completion)
    pub active_predictions: Arc<RwLock<std::collections::HashMap<String, CancellationToken>>>,
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
        // Fast async init - just opens files
        let storage = Storage::open(&config.iroh_dir).await?;

        // Sync init - create index and indexer config
        let index = search::open_index(&config.search_dir)?;
        let indexer_config = IndexerConfig::default();

        let storage = Arc::new(RwLock::new(storage));
        let search = Arc::new(index);

        // Spawn index worker - handles all milli write operations in a dedicated thread
        let index_worker = spawn_index_worker(search.clone(), indexer_config);

        let memory = Arc::new(FileConversationMemory::new(
            config.conversations_dir.clone(),
        ));

        // Create event-driven pipeline for document processing
        let (pipeline, progress_rx) = Pipeline::new(storage.clone(), index_worker.clone());

        Ok((
            Self {
                config,
                storage,
                search,
                index_worker,
                memory,
                active_generations: Arc::new(RwLock::new(Default::default())),
                active_predictions: Arc::new(RwLock::new(Default::default())),
                pipeline: Arc::new(pipeline),
            },
            progress_rx,
        ))
    }

    /// Restore provider configuration from settings. Called once at startup.
    ///
    /// Starts watching existing collections for indexing events and — when a
    /// chat provider is configured — validates that it still constructs
    /// (logging a warning instead of failing startup otherwise). Provider
    /// models are built per request; nothing long-lived is installed here.
    pub async fn restore_configs_from_settings(&self) {
        // Start watching existing collections for indexing events.
        self.watch_existing_collections().await;

        let settings = Settings::load(&self.config.settings_file);
        if let Some(ref provider) = settings.provider {
            match providers::build_model(provider) {
                Ok(_) => tracing::info!(
                    family = %provider.family,
                    model = %provider.model,
                    "Chat provider configured"
                ),
                Err(e) => tracing::warn!(
                    family = %provider.family,
                    error = %e,
                    "Configured chat provider failed validation; chat will be unavailable until it is reconfigured"
                ),
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
