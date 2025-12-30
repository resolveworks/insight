//! Import tracking for active document imports
//!
//! Tracks in-memory progress of files being imported. No persistence.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Status of a file being imported
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ImportFileStatus {
    /// Queued for import
    Pending,
    /// Currently being imported
    InProgress,
    /// Successfully imported
    Completed { document_id: String },
    /// Import failed
    Failed { error: String },
}

/// Summary of import progress for a collection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportProgress {
    pub collection_id: String,
    pub total: usize,
    pub completed: usize,
    pub failed: usize,
    pub pending: usize,
    pub in_progress: usize,
}

/// Internal tracking of a file being imported
#[derive(Debug, Clone)]
struct TrackedFile {
    collection_id: String,
    status: ImportFileStatus,
}

/// Tracks active imports in memory
#[derive(Clone, Default)]
pub struct ImportTracker {
    /// Files indexed by path
    files: Arc<RwLock<HashMap<String, TrackedFile>>>,
}

impl ImportTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue files for import
    pub async fn queue_files(&self, collection_id: &str, paths: &[String]) {
        let mut files = self.files.write().await;
        for path in paths {
            files.insert(
                path.clone(),
                TrackedFile {
                    collection_id: collection_id.to_string(),
                    status: ImportFileStatus::Pending,
                },
            );
        }
    }

    /// Get progress for a collection
    pub async fn get_progress(&self, collection_id: &str) -> ImportProgress {
        let files = self.files.read().await;
        let collection_files: Vec<_> = files
            .values()
            .filter(|f| f.collection_id == collection_id)
            .collect();

        ImportProgress {
            collection_id: collection_id.to_string(),
            total: collection_files.len(),
            completed: collection_files
                .iter()
                .filter(|f| matches!(f.status, ImportFileStatus::Completed { .. }))
                .count(),
            failed: collection_files
                .iter()
                .filter(|f| matches!(f.status, ImportFileStatus::Failed { .. }))
                .count(),
            pending: collection_files
                .iter()
                .filter(|f| matches!(f.status, ImportFileStatus::Pending))
                .count(),
            in_progress: collection_files
                .iter()
                .filter(|f| matches!(f.status, ImportFileStatus::InProgress))
                .count(),
        }
    }

    /// Get progress for all collections with active imports
    pub async fn get_all_progress(&self) -> Vec<ImportProgress> {
        let files = self.files.read().await;

        // Group by collection
        let mut by_collection: HashMap<&str, Vec<&TrackedFile>> = HashMap::new();
        for file in files.values() {
            by_collection
                .entry(&file.collection_id)
                .or_default()
                .push(file);
        }

        by_collection
            .into_iter()
            .filter(|(_, files)| {
                // Only include collections with active imports
                files.iter().any(|f| {
                    matches!(
                        f.status,
                        ImportFileStatus::Pending | ImportFileStatus::InProgress
                    )
                })
            })
            .map(|(collection_id, collection_files)| ImportProgress {
                collection_id: collection_id.to_string(),
                total: collection_files.len(),
                completed: collection_files
                    .iter()
                    .filter(|f| matches!(f.status, ImportFileStatus::Completed { .. }))
                    .count(),
                failed: collection_files
                    .iter()
                    .filter(|f| matches!(f.status, ImportFileStatus::Failed { .. }))
                    .count(),
                pending: collection_files
                    .iter()
                    .filter(|f| matches!(f.status, ImportFileStatus::Pending))
                    .count(),
                in_progress: collection_files
                    .iter()
                    .filter(|f| matches!(f.status, ImportFileStatus::InProgress))
                    .count(),
            })
            .collect()
    }

    /// Mark a file as in progress
    pub async fn mark_in_progress(&self, path: &str) {
        let mut files = self.files.write().await;
        if let Some(file) = files.get_mut(path) {
            file.status = ImportFileStatus::InProgress;
        }
    }

    /// Mark a file as completed and remove from tracking
    pub async fn mark_completed(&self, path: &str, document_id: String) {
        let mut files = self.files.write().await;
        if let Some(file) = files.get_mut(path) {
            file.status = ImportFileStatus::Completed { document_id };
        }
    }

    /// Mark a file as failed
    pub async fn mark_failed(&self, path: &str, error: String) {
        let mut files = self.files.write().await;
        if let Some(file) = files.get_mut(path) {
            file.status = ImportFileStatus::Failed { error };
        }
    }

    /// Remove all finished files for a collection
    pub async fn cleanup_collection(&self, collection_id: &str) {
        let mut files = self.files.write().await;
        files.retain(|_, f| {
            f.collection_id != collection_id
                || matches!(
                    f.status,
                    ImportFileStatus::Pending | ImportFileStatus::InProgress
                )
        });
    }

    /// Check if there are active imports for a collection
    pub async fn has_active_imports(&self, collection_id: &str) -> bool {
        let files = self.files.read().await;
        files.values().any(|f| {
            f.collection_id == collection_id
                && matches!(
                    f.status,
                    ImportFileStatus::Pending | ImportFileStatus::InProgress
                )
        })
    }

    /// Get pending file paths for a collection
    pub async fn pending_paths(&self, collection_id: &str) -> Vec<String> {
        let files = self.files.read().await;
        files
            .iter()
            .filter(|(_, f)| {
                f.collection_id == collection_id && matches!(f.status, ImportFileStatus::Pending)
            })
            .map(|(path, _)| path.clone())
            .collect()
    }
}
