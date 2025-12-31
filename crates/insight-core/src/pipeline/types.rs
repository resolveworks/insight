//! Pipeline types and job definitions.

use iroh_docs::NamespaceId;
use serde::{Deserialize, Serialize};

/// Processing stage in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Store,
    Extract,
    Embed,
    Index,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stage::Store => write!(f, "store"),
            Stage::Extract => write!(f, "extract"),
            Stage::Embed => write!(f, "embed"),
            Stage::Index => write!(f, "index"),
        }
    }
}

/// Job for the extract worker pool.
#[derive(Debug, Clone)]
pub struct ExtractJob {
    pub namespace_id: NamespaceId,
    pub doc_id: String,
}

/// Job for the embed worker pool.
#[derive(Debug, Clone)]
pub struct EmbedJob {
    pub namespace_id: NamespaceId,
    pub doc_id: String,
}

/// Job for the index worker.
#[derive(Debug, Clone)]
pub struct IndexJob {
    pub namespace_id: NamespaceId,
    pub doc_id: String,
    pub model_id: String,
}

/// Progress update from workers.
#[derive(Debug, Clone)]
pub enum ProgressUpdate {
    /// Job started processing (pending -> active).
    Started { collection_id: String, stage: Stage },
    /// Job completed successfully (active -> completed).
    Completed { collection_id: String, stage: Stage },
    /// Job failed (active -> failed).
    Failed {
        collection_id: String,
        stage: Stage,
        error: String,
    },
    /// Job queued for processing (-> pending).
    Queued { collection_id: String, stage: Stage },
}
