use std::path::Path;

use anyhow::{Context, Result};
use milli::heed::EnvOpenOptions;
use milli::progress::Progress;
use milli::update::IndexerConfig;
use milli::{FilterableAttributesRule, Index};

/// Open or create a milli search index
pub fn open_index(path: &Path) -> Result<Index> {
    std::fs::create_dir_all(path)?;

    let index = Index::new(
        EnvOpenOptions::new().read_txn_without_tls(),
        path,
        true,
    )
    .context("Failed to create milli index")?;

    // Configure filterable attributes for collection faceting
    let needs_setup = {
        let rtxn = index.read_txn()?;
        let current_rules = index.filterable_attributes_rules(&rtxn)?;
        !current_rules
            .iter()
            .any(|rule| matches!(rule, FilterableAttributesRule::Field(f) if f == "collection_id"))
    };

    if needs_setup {
        let indexer_config = IndexerConfig::default();
        let mut wtxn = index.write_txn()?;
        let mut settings = milli::update::Settings::new(&mut wtxn, &index, &indexer_config);
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field(
            "collection_id".to_string(),
        )]);
        settings.execute(&|| false, &Progress::default(), Default::default())?;
        wtxn.commit()?;
        tracing::info!("Configured filterable attribute: collection_id");
    }

    tracing::info!("Search index opened at {:?}", path);

    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_open_index() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();

        // Verify we can read from the index
        let rtxn = index.read_txn().unwrap();
        let count = index.number_of_documents(&rtxn).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_index_reopens() {
        let temp_dir = tempfile::tempdir().unwrap();

        // Open and close
        {
            let _index = open_index(temp_dir.path()).unwrap();
        }

        // Should reopen successfully
        let index = open_index(temp_dir.path()).unwrap();
        let rtxn = index.read_txn().unwrap();
        let count = index.number_of_documents(&rtxn).unwrap();
        assert_eq!(count, 0);
    }
}
