//! Insight Core - Business logic for document management and search
//!
//! This crate contains all the core functionality for Insight, including:
//! - Document storage (iroh P2P)
//! - Full-text search (milli)
//! - Document text extraction (MuPDF), routed through the [`parsers`] registry
//! - Remote inference providers via the [`rig`] framework: the provider
//!   registry ([`providers`]), typed document tools ([`tools`]), file-backed
//!   conversation memory ([`memory`]), and the agent [`runtime`]
//! - Event-driven pipeline for document import

pub mod chat;
pub mod config;
pub mod memory;
pub mod parsers;
pub mod pipeline;
pub mod providers;
pub mod runtime;
pub mod search;
pub mod storage;
pub mod text;
pub mod tools;

use anyhow::Context;
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
    /// Total pages across the documents that report a page count.
    /// `None` when no document in the collection reports pages.
    #[serde(default)]
    pub total_pages: Option<usize>,
    /// When the collection was created (ISO 8601)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Sum of the known page counts, or `None` when no document reports pages.
pub fn sum_page_counts(pages: impl IntoIterator<Item = Option<usize>>) -> Option<usize> {
    let mut total = None;
    for pages in pages.into_iter().flatten() {
        *total.get_or_insert(0) += pages;
    }
    total
}

pub use chat::{from_messages, ChatEvent, RunTranscript, TranscriptItem, TranscriptKind};
pub use config::{Config, Settings, SettingsError};
pub use memory::{ConversationSummary, FileConversationMemory, DEFAULT_TITLE};
pub use pipeline::{AppEvent, Pipeline, PipelineProgress, StageProgress};
pub use providers::{ModelChoice, ProviderSettings};
pub use runtime::{send_message, BASE_SYSTEM_PROMPT};
pub use search::{spawn_index_worker, IndexWorkerHandle};
pub use storage::Storage;

/// How long an import waits for the shared collection's metadata to arrive.
const COLLECTION_IMPORT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// Event-driven document processing pipeline
    pub pipeline: Arc<Pipeline>,
}

impl AppState {
    /// Create AppState with initialized storage and search.
    /// Called from setup() where Tauri's async runtime is available.
    ///
    /// Returns the state and a receiver for application-layer events
    /// (pipeline progress and collection invalidation) that should be
    /// forwarded to the frontend.
    pub async fn new(
        config: Config,
    ) -> anyhow::Result<(Self, tokio::sync::mpsc::UnboundedReceiver<AppEvent>)> {
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
        )?);

        // Create event-driven pipeline for document processing
        let (pipeline, events_rx) = Pipeline::new(storage.clone(), index_worker.clone());

        Ok((
            Self {
                config,
                storage,
                search,
                index_worker,
                memory,
                active_generations: Arc::new(RwLock::new(Default::default())),
                pipeline: Arc::new(pipeline),
            },
            events_rx,
        ))
    }

    /// Restore provider configuration from settings. Called once at startup.
    ///
    /// Starts watching existing collections for indexing events and — when a
    /// chat provider is configured — validates that it still constructs
    /// (logging a warning instead of failing startup otherwise). Provider
    /// models are built per request; nothing long-lived is installed here.
    ///
    /// Fails when the persisted settings cannot be read or parsed; invalid
    /// settings are never silently treated as an unconfigured provider.
    pub async fn restore_configs_from_settings(&self) -> anyhow::Result<()> {
        // Start watching existing collections for indexing events.
        self.watch_existing_collections().await;

        let settings = Settings::load(&self.config.settings_file)
            .map_err(|e| anyhow::anyhow!(e))
            .context("failed to load settings")?;
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

        Ok(())
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
            // Subscribe before sync starts so startup neighbor and sync events
            // reach the watcher rather than landing before it listens.
            if let Err(e) = self.pipeline.watch(*namespace_id).await {
                tracing::error!(
                    namespace = %namespace_id,
                    collection = %metadata.name,
                    error = %e,
                    "Failed to watch collection at startup"
                );
                continue;
            }
            tracing::debug!("Watching collection '{}' ({})", metadata.name, namespace_id);

            if let Err(e) = self.storage.read().await.resume_sync(*namespace_id).await {
                tracing::warn!(
                    namespace = %namespace_id,
                    collection = %metadata.name,
                    error = %e,
                    "Failed to resume collection sync at startup"
                );
            }

            if let Err(e) = self
                .pipeline
                .resume_collection(*namespace_id, &self.search)
                .await
            {
                tracing::warn!(
                    namespace = %namespace_id,
                    collection = %metadata.name,
                    error = %e,
                    "Failed to resume collection pipeline at startup"
                );
            }
        }

        tracing::info!(
            "Started watching {} existing collections",
            collections.len()
        );
    }

    /// Start watching a namespace for pipeline events.
    pub async fn watch_namespace(
        &self,
        namespace_id: iroh_docs::NamespaceId,
    ) -> anyhow::Result<()> {
        self.pipeline.watch(namespace_id).await?;
        Ok(())
    }

    /// Stop watching a namespace.
    pub async fn unwatch_namespace(&self, namespace_id: &iroh_docs::NamespaceId) {
        self.pipeline.unwatch(namespace_id).await;
    }

    /// Import a collection from a share ticket and wait for its metadata.
    ///
    /// The watcher is attached before sync starts so the initial neighbor and
    /// sync events are observed. A failed first import is rolled back; a failed
    /// re-import leaves the existing collection in place.
    pub async fn import_collection(
        &self,
        ticket_str: &str,
    ) -> anyhow::Result<(iroh_docs::NamespaceId, storage::CollectionMetadata)> {
        self.import_collection_with_timeout(ticket_str, COLLECTION_IMPORT_TIMEOUT)
            .await
    }

    async fn import_collection_with_timeout(
        &self,
        ticket_str: &str,
        timeout: std::time::Duration,
    ) -> anyhow::Result<(iroh_docs::NamespaceId, storage::CollectionMetadata)> {
        let ticket: iroh_docs::DocTicket = ticket_str.parse().context("Invalid share ticket")?;
        let namespace_id = ticket.capability.id();

        let existed = {
            let storage = self.storage.read().await;
            storage.collection_exists(namespace_id).await?
        };
        {
            let storage = self.storage.read().await;
            storage.register_collection_import(&ticket).await?;
        }

        let (watching, result) = match self.pipeline.watch(namespace_id).await {
            Ok(watching) => {
                let result = async {
                    {
                        let storage = self.storage.read().await;
                        storage.start_collection_sync(&ticket).await?;
                    }
                    let storage = self.storage.read().await;
                    storage
                        .wait_for_collection_metadata(namespace_id, timeout)
                        .await
                }
                .await;
                (watching, result)
            }
            Err(e) => (false, Err(e)),
        };

        match result {
            Ok(metadata) => {
                tracing::info!("Imported collection {} ('{}')", namespace_id, metadata.name);
                Ok((namespace_id, metadata))
            }
            Err(err) => {
                if watching {
                    self.pipeline.unwatch(&namespace_id).await;
                }
                if !existed {
                    let storage = self.storage.read().await;
                    if let Err(cleanup) = storage.delete_collection(namespace_id).await {
                        tracing::error!(
                            namespace = %namespace_id,
                            error = %cleanup,
                            "Failed to remove collection after failed import"
                        );
                    }
                }
                Err(err)
            }
        }
    }

    /// Delete a collection, its conversations and its search index entries.
    ///
    /// Cancels in-flight generations first, then removes storage state and the
    /// watcher so a later import attaches a fresh one.
    pub async fn delete_collection(
        &self,
        namespace_id: iroh_docs::NamespaceId,
    ) -> anyhow::Result<()> {
        let collection_id = namespace_id.to_string();
        tracing::info!("Deleting collection {}", collection_id);

        let summaries = self.memory.list_summaries(&collection_id).await?;
        for summary in summaries {
            if let Some(token) = self.active_generations.write().await.remove(&summary.id) {
                token.cancel();
            }
        }

        {
            let storage = self.storage.read().await;
            storage.delete_collection(namespace_id).await?;
        }
        self.pipeline.unwatch(&namespace_id).await;

        let index_worker = self.index_worker.clone();
        let index_collection_id = collection_id.clone();
        tokio::spawn(async move {
            match index_worker
                .delete_collection_chunks(index_collection_id.clone())
                .await
            {
                Ok(deleted_count) => {
                    tracing::info!(
                        "Removed {} chunks from search index for collection {}",
                        deleted_count,
                        index_collection_id
                    );
                }
                Err(e) => {
                    tracing::error!(
                        "Failed to remove chunks from search index for collection {}: {}",
                        index_collection_id,
                        e
                    );
                }
            }
        });

        self.memory.delete_collection(&collection_id).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// An AppState over a temporary data directory.
    async fn test_state() -> (tempfile::TempDir, AppState) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            conversations_dir: temp_dir.path().join("conversations"),
            settings_file: temp_dir.path().join("settings.json"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _events) = AppState::new(config).await.unwrap();
        (temp_dir, state)
    }

    async fn share(state: &AppState, collection: iroh_docs::NamespaceId) -> String {
        state
            .storage
            .read()
            .await
            .share_collection(collection)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn failed_first_import_leaves_no_collection() {
        let (_source_dir, source) = test_state().await;
        let (collection, _) = source
            .storage
            .write()
            .await
            .create_collection("Gone before import")
            .await
            .unwrap();
        let ticket = share(&source, collection).await;

        // The source no longer has the collection, so its metadata never syncs.
        source
            .storage
            .read()
            .await
            .delete_collection(collection)
            .await
            .unwrap();

        let (_target_dir, target) = test_state().await;
        target
            .import_collection_with_timeout(&ticket, Duration::from_millis(200))
            .await
            .unwrap_err();

        let storage = target.storage.read().await;
        assert!(!storage.collection_exists(collection).await.unwrap());
        assert!(storage.list_collections().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_reimport_leaves_existing_collection() {
        let (_source_dir, source) = test_state().await;
        let (collection, _) = source
            .storage
            .write()
            .await
            .create_collection("Existing")
            .await
            .unwrap();
        let ticket = share(&source, collection).await;

        let (_target_dir, target) = test_state().await;
        let parsed: iroh_docs::DocTicket = ticket.parse().unwrap();
        target
            .storage
            .read()
            .await
            .register_collection_import(&parsed)
            .await
            .unwrap();
        assert!(target
            .storage
            .read()
            .await
            .collection_exists(collection)
            .await
            .unwrap());

        // Make the shared metadata unreachable while keeping the namespace local.
        source
            .storage
            .read()
            .await
            .delete_collection(collection)
            .await
            .unwrap();

        target
            .import_collection_with_timeout(&ticket, Duration::from_millis(200))
            .await
            .unwrap_err();

        assert!(target
            .storage
            .read()
            .await
            .collection_exists(collection)
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn delete_then_reimport_attaches_a_fresh_watcher() {
        let (_source_dir, source) = test_state().await;
        let (_target_dir, target) = test_state().await;
        let (collection, _) = source
            .storage
            .write()
            .await
            .create_collection("Reimported")
            .await
            .unwrap();
        let ticket = share(&source, collection).await;

        let (imported, metadata) = target.import_collection(&ticket).await.unwrap();
        assert_eq!(imported, collection);
        assert_eq!(metadata.name, "Reimported");

        target.delete_collection(collection).await.unwrap();
        assert!(!target
            .storage
            .read()
            .await
            .collection_exists(collection)
            .await
            .unwrap());

        let (reimported, _) = target.import_collection(&ticket).await.unwrap();
        assert_eq!(reimported, collection);

        let doc_id = source
            .storage
            .read()
            .await
            .add_document(
                collection,
                "after-delete.pdf",
                "application/pdf",
                b"written after the re-import",
            )
            .await
            .unwrap()
            .id;

        let synced = tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                let documents = target
                    .storage
                    .read()
                    .await
                    .list_documents(collection)
                    .await
                    .unwrap();
                if let Some(document) = documents.iter().find(|document| document.id == doc_id) {
                    break document.clone();
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("the re-imported collection should receive the new document");

        assert_eq!(synced.name, "after-delete.pdf");
    }
}
