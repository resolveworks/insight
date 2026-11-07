//! Pipeline types and job definitions.

use iroh_docs::NamespaceId;
use serde::{Deserialize, Serialize};

/// Processing stage in the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    Store,
    Extract,
    Index,
}

impl std::fmt::Display for Stage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Stage::Store => write!(f, "store"),
            Stage::Extract => write!(f, "extract"),
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

/// Job for the index worker.
#[derive(Debug, Clone)]
pub struct IndexJob {
    pub namespace_id: NamespaceId,
    pub doc_id: String,
}
