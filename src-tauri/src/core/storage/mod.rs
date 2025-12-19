use std::path::Path;

use anyhow::{Context, Result};
use iroh_blobs::store::fs::Store as BlobStore;
use iroh_docs::store::fs::Store as DocStore;

/// Storage layer using iroh for P2P content-addressed storage
pub struct Storage {
    /// Content-addressed blob storage (PDFs, extracted text)
    pub blobs: BlobStore,
    /// CRDT document store (metadata, collections)
    pub docs: DocStore,
}

impl Storage {
    /// Initialize storage at the given path
    pub async fn open(path: &Path) -> Result<Self> {
        std::fs::create_dir_all(path)?;

        let blobs_path = path.join("blobs");
        let docs_path = path.join("docs.redb");

        let blobs = BlobStore::load(&blobs_path)
            .await
            .context("Failed to open blob store")?;

        let docs = DocStore::persistent(&docs_path)
            .context("Failed to open docs store")?;

        tracing::info!("Storage opened at {:?}", path);

        Ok(Self { blobs, docs })
    }
}
