//! Progress tracking for the document processing pipeline.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};

use super::types::{ProgressUpdate, Stage};

/// Progress for a single processing stage.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StageProgress {
    pub pending: usize,
    pub active: usize,
    pub completed: usize,
    pub failed: usize,
}

impl StageProgress {
    /// Total documents that entered this stage.
    pub fn total(&self) -> usize {
        self.pending + self.active + self.completed + self.failed
    }

    /// Check if stage has any active work.
    pub fn is_active(&self) -> bool {
        self.pending > 0 || self.active > 0
    }
}

/// Progress for a collection across all pipeline stages.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PipelineProgress {
    pub collection_id: String,
    pub store: StageProgress,
    pub extract: StageProgress,
    pub embed: StageProgress,
    pub index: StageProgress,
}

impl PipelineProgress {
    pub fn new(collection_id: String) -> Self {
        Self {
            collection_id,
            ..Default::default()
        }
    }

    /// Get mutable reference to stage progress.
    fn stage_mut(&mut self, stage: Stage) -> &mut StageProgress {
        match stage {
            Stage::Store => &mut self.store,
            Stage::Extract => &mut self.extract,
            Stage::Embed => &mut self.embed,
            Stage::Index => &mut self.index,
        }
    }

    /// Check if any stage has active work.
    pub fn is_active(&self) -> bool {
        self.store.is_active()
            || self.extract.is_active()
            || self.embed.is_active()
            || self.index.is_active()
    }
}

/// Tracks progress for all collections.
#[derive(Clone)]
pub struct ProgressTracker {
    collections: Arc<RwLock<HashMap<String, PipelineProgress>>>,
    /// Channel to notify listeners of progress changes
    notify_tx: mpsc::Sender<PipelineProgress>,
}

impl ProgressTracker {
    pub fn new() -> (Self, mpsc::Receiver<PipelineProgress>) {
        let (notify_tx, notify_rx) = mpsc::channel(256);
        (
            Self {
                collections: Arc::new(RwLock::new(HashMap::new())),
                notify_tx,
            },
            notify_rx,
        )
    }

    /// Queue a job (sync version for event handlers).
    /// Spawns async work in background to avoid blocking.
    pub fn queue(&self, collection_id: &str, stage: Stage) {
        let tracker = self.clone();
        let collection_id = collection_id.to_string();
        tokio::spawn(async move {
            tracker
                .apply(ProgressUpdate::Queued {
                    collection_id,
                    stage,
                })
                .await;
        });
    }

    /// Apply a progress update.
    pub async fn apply(&self, update: ProgressUpdate) {
        let mut collections = self.collections.write().await;

        let collection_id = match &update {
            ProgressUpdate::Queued { collection_id, .. }
            | ProgressUpdate::Started { collection_id, .. }
            | ProgressUpdate::Completed { collection_id, .. }
            | ProgressUpdate::Failed { collection_id, .. } => collection_id.clone(),
        };

        match update {
            ProgressUpdate::Queued {
                collection_id,
                stage,
            } => {
                let progress = collections
                    .entry(collection_id.clone())
                    .or_insert_with(|| PipelineProgress::new(collection_id));
                progress.stage_mut(stage).pending += 1;
            }
            ProgressUpdate::Started {
                collection_id,
                stage,
            } => {
                if let Some(progress) = collections.get_mut(&collection_id) {
                    let stage_progress = progress.stage_mut(stage);
                    stage_progress.pending = stage_progress.pending.saturating_sub(1);
                    stage_progress.active += 1;
                }
            }
            ProgressUpdate::Completed {
                collection_id,
                stage,
            } => {
                if let Some(progress) = collections.get_mut(&collection_id) {
                    let stage_progress = progress.stage_mut(stage);
                    stage_progress.active = stage_progress.active.saturating_sub(1);
                    stage_progress.completed += 1;
                }
            }
            ProgressUpdate::Failed {
                collection_id,
                stage,
                ..
            } => {
                if let Some(progress) = collections.get_mut(&collection_id) {
                    let stage_progress = progress.stage_mut(stage);
                    stage_progress.active = stage_progress.active.saturating_sub(1);
                    stage_progress.failed += 1;
                }
            }
        }

        // Notify listeners of the updated progress
        if let Some(progress) = collections.get(&collection_id) {
            let _ = self.notify_tx.try_send(progress.clone());
        }
    }

    /// Get progress for a collection.
    pub async fn get(&self, collection_id: &str) -> Option<PipelineProgress> {
        self.collections.read().await.get(collection_id).cloned()
    }

    /// Get progress for all active collections.
    pub async fn get_all_active(&self) -> Vec<PipelineProgress> {
        self.collections
            .read()
            .await
            .values()
            .filter(|p| p.is_active())
            .cloned()
            .collect()
    }

    /// Remove a collection from tracking.
    pub async fn remove(&self, collection_id: &str) {
        self.collections.write().await.remove(collection_id);
    }
}
