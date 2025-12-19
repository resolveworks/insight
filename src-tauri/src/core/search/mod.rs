use std::path::Path;

use anyhow::{Context, Result};
use milli::heed::EnvOpenOptions;
use milli::Index;

/// Open or create a milli search index
pub fn open_index(path: &Path) -> Result<Index> {
    std::fs::create_dir_all(path)?;

    let index = Index::new(
        EnvOpenOptions::new().read_txn_without_tls(),
        path,
        true,
    )
    .context("Failed to create milli index")?;

    tracing::info!("Search index opened at {:?}", path);

    Ok(index)
}
