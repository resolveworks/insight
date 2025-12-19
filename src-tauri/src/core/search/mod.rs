use std::path::Path;

use anyhow::{Context, Result};
use bumpalo::Bump;
use milli::documents::mmap_from_objects;
use milli::heed::EnvOpenOptions;
use milli::progress::Progress;
use milli::score_details::ScoringStrategy;
use milli::update::new::indexer::{self, DocumentOperation};
use milli::update::IndexerConfig;
use milli::vector::RuntimeEmbedders;
use milli::{FilterableAttributesRule, Index, TermsMatchingStrategy};
use serde_json::{Map, Value};

/// Default map size for the LMDB environment (10 GB)
/// This is the maximum size the database can grow to
const DEFAULT_MAP_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Open or create a milli search index
pub fn open_index(path: &Path) -> Result<Index> {
    std::fs::create_dir_all(path)?;

    let mut env_options = EnvOpenOptions::new();
    env_options.map_size(DEFAULT_MAP_SIZE);
    let env_options = env_options.read_txn_without_tls();

    let index = Index::new(env_options, path, true).context("Failed to create milli index")?;

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
        settings.set_primary_key("id".to_string());
        settings.set_filterable_fields(vec![FilterableAttributesRule::Field(
            "collection_id".to_string(),
        )]);
        settings.execute(&|| false, &Progress::default(), Default::default())?;
        wtxn.commit()?;
        tracing::info!("Configured primary key and filterable attribute");
    }

    tracing::info!("Search index opened at {:?}", path);

    Ok(index)
}

/// Document to be indexed
pub struct DocToIndex {
    pub id: String,
    pub name: String,
    pub content: String,
    pub collection_id: String,
}

/// Index a single document in milli (uses shared IndexerConfig)
pub fn index_document(
    index: &Index,
    indexer_config: &IndexerConfig,
    doc_id: &str,
    name: &str,
    content: &str,
    collection_id: &str,
) -> Result<()> {
    index_documents_batch(
        index,
        indexer_config,
        vec![DocToIndex {
            id: doc_id.to_string(),
            name: name.to_string(),
            content: content.to_string(),
            collection_id: collection_id.to_string(),
        }],
    )
}

/// Maximum documents per indexing chunk to avoid stack overflow
const BATCH_CHUNK_SIZE: usize = 100;

/// Index multiple documents in a single batch operation
pub fn index_documents_batch(
    index: &Index,
    indexer_config: &IndexerConfig,
    docs: Vec<DocToIndex>,
) -> Result<()> {
    if docs.is_empty() {
        return Ok(());
    }

    let total = docs.len();

    // Process in chunks to avoid stack overflow with large batches
    let num_chunks = (total + BATCH_CHUNK_SIZE - 1) / BATCH_CHUNK_SIZE;
    for (chunk_idx, chunk) in docs.chunks(BATCH_CHUNK_SIZE).enumerate() {
        index_chunk(index, indexer_config, chunk)?;
        tracing::info!(
            "Indexed chunk {}/{} ({} documents, {}% complete)",
            chunk_idx + 1,
            num_chunks,
            chunk.len(),
            ((chunk_idx + 1) * 100) / num_chunks
        );
    }

    tracing::debug!("Indexed batch of {} documents total", total);

    Ok(())
}

/// Index a single chunk of documents
fn index_chunk(index: &Index, indexer_config: &IndexerConfig, docs: &[DocToIndex]) -> Result<()> {
    let json_docs: Vec<Map<String, Value>> = docs
        .iter()
        .map(|doc| {
            let mut m = Map::new();
            m.insert("id".to_string(), Value::String(doc.id.clone()));
            m.insert("name".to_string(), Value::String(doc.name.clone()));
            m.insert("content".to_string(), Value::String(doc.content.clone()));
            m.insert(
                "collection_id".to_string(),
                Value::String(doc.collection_id.clone()),
            );
            m
        })
        .collect();

    let mmap = mmap_from_objects(json_docs);

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let embedders = RuntimeEmbedders::default();

    let mut operation = DocumentOperation::new();
    operation.replace_documents(&mmap)?;

    let indexer_alloc = Bump::new();
    let (document_changes, operation_stats, primary_key) = operation.into_changes(
        &indexer_alloc,
        index,
        &rtxn,
        None,
        &mut new_fields_ids_map,
        &|| false,
        Progress::default(),
        None,
    )?;

    if let Some(error) = operation_stats.into_iter().find_map(|stat| stat.error) {
        anyhow::bail!("Document operation error: {}", error);
    }

    let mut wtxn = index.write_txn()?;

    // Use the shared thread pool from IndexerConfig for both outer and inner operations
    indexer_config
        .thread_pool
        .install(|| {
            indexer::index(
                &mut wtxn,
                index,
                &indexer_config.thread_pool,
                indexer_config.grenad_parameters(),
                &db_fields_ids_map,
                new_fields_ids_map,
                primary_key,
                &document_changes,
                embedders,
                &|| false,
                &Progress::default(),
                &Default::default(),
            )
        })
        .map_err(|e| anyhow::anyhow!("Thread pool error: {}", e))??;

    wtxn.commit()?;

    Ok(())
}

/// Search result with document ID and score
pub struct SearchHit {
    pub doc_id: u32,
    pub scores: Vec<milli::score_details::ScoreDetails>,
}

/// Search results with pagination info
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total_hits: usize,
}

/// Get the number of documents in the index
pub fn get_document_count(index: &Index) -> Result<u64> {
    let rtxn = index.read_txn()?;
    Ok(index.number_of_documents(&rtxn)?)
}

/// Search the milli index
pub fn search_index(
    index: &Index,
    query: &str,
    limit: usize,
    offset: usize,
    collection_ids: Option<&[String]>,
) -> Result<SearchResults> {
    let rtxn = index.read_txn()?;
    let mut search = milli::Search::new(&rtxn, index);
    search.query(query);
    search.limit(limit);
    search.offset(offset);
    search.scoring_strategy(ScoringStrategy::Detailed);
    search.exhaustive_number_hits(true);
    search.terms_matching_strategy(TermsMatchingStrategy::Last);

    // Build filter string (must outlive the search)
    let filter_str = collection_ids
        .filter(|ids| !ids.is_empty())
        .map(|ids| {
            let quoted: Vec<String> = ids.iter().map(|id| format!("\"{}\"", id)).collect();
            format!("collection_id IN [{}]", quoted.join(", "))
        });

    if let Some(ref fs) = filter_str {
        let filter = milli::Filter::from_str(fs)
            .map_err(|e| anyhow::anyhow!("Filter error: {:?}", e))?;
        if let Some(f) = filter {
            search.filter(f);
        }
    }

    let result = search.execute()?;

    let hits = result
        .documents_ids
        .into_iter()
        .zip(result.document_scores)
        .map(|(doc_id, scores)| SearchHit { doc_id, scores })
        .collect();

    Ok(SearchResults {
        hits,
        total_hits: result.candidates.len() as usize,
    })
}

/// Extract a string field from an indexed document
pub fn get_document_field(index: &Index, doc_id: u32, field: &str) -> Result<Option<String>> {
    let rtxn = index.read_txn()?;
    let fields_ids_map = index.fields_ids_map(&rtxn)?;

    let docs = index.documents(&rtxn, [doc_id])?;
    if let Some((_id, obkv)) = docs.first() {
        let value = fields_ids_map
            .id(field)
            .and_then(|fid| obkv.get(fid))
            .and_then(|v| serde_json::from_slice::<String>(v).ok());
        Ok(value)
    } else {
        Ok(None)
    }
}

/// Delete a single document from the index by its external ID
pub fn delete_document(index: &Index, indexer_config: &IndexerConfig, doc_id: &str) -> Result<()> {
    delete_documents(index, indexer_config, &[doc_id.to_string()])
}

/// Delete multiple documents from the index by their external IDs
pub fn delete_documents(
    index: &Index,
    indexer_config: &IndexerConfig,
    doc_ids: &[String],
) -> Result<()> {
    if doc_ids.is_empty() {
        return Ok(());
    }

    // Convert to slice of &str for the API
    let doc_ids_refs: Vec<&str> = doc_ids.iter().map(|s| s.as_str()).collect();

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let embedders = RuntimeEmbedders::default();

    let mut operation = DocumentOperation::new();
    operation.delete_documents(&doc_ids_refs);

    let indexer_alloc = Bump::new();
    let (document_changes, operation_stats, primary_key) = operation.into_changes(
        &indexer_alloc,
        index,
        &rtxn,
        None,
        &mut new_fields_ids_map,
        &|| false,
        Progress::default(),
        None,
    )?;

    if let Some(error) = operation_stats.into_iter().find_map(|stat| stat.error) {
        anyhow::bail!("Document deletion error: {}", error);
    }

    let mut wtxn = index.write_txn()?;

    indexer_config
        .thread_pool
        .install(|| {
            indexer::index(
                &mut wtxn,
                index,
                &indexer_config.thread_pool,
                indexer_config.grenad_parameters(),
                &db_fields_ids_map,
                new_fields_ids_map,
                primary_key,
                &document_changes,
                embedders,
                &|| false,
                &Progress::default(),
                &Default::default(),
            )
        })
        .map_err(|e| anyhow::anyhow!("Thread pool error: {}", e))??;

    wtxn.commit()?;

    tracing::debug!("Deleted {} documents from index", doc_ids.len());

    Ok(())
}

/// Delete all documents belonging to a specific collection
pub fn delete_documents_by_collection(
    index: &Index,
    indexer_config: &IndexerConfig,
    collection_id: &str,
) -> Result<usize> {
    // First, find all document IDs in this collection
    let rtxn = index.read_txn()?;
    let fields_ids_map = index.fields_ids_map(&rtxn)?;

    let id_field = fields_ids_map.id("id");
    let collection_field = fields_ids_map.id("collection_id");

    if id_field.is_none() || collection_field.is_none() {
        // Index not set up yet, nothing to delete
        return Ok(0);
    }

    let id_fid = id_field.unwrap();
    let collection_fid = collection_field.unwrap();

    // Get all document IDs
    let all_doc_ids: Vec<u32> = index.documents_ids(&rtxn)?.iter().collect();

    // Filter to find documents in this collection
    let mut doc_ids_to_delete = Vec::new();
    for internal_id in all_doc_ids {
        let docs = index.documents(&rtxn, [internal_id])?;
        if let Some((_, obkv)) = docs.first() {
            // Check if this document belongs to the collection
            if let Some(coll_bytes) = obkv.get(collection_fid) {
                if let Ok(coll) = serde_json::from_slice::<String>(coll_bytes) {
                    if coll == collection_id {
                        // Get the external ID
                        if let Some(id_bytes) = obkv.get(id_fid) {
                            if let Ok(external_id) = serde_json::from_slice::<String>(id_bytes) {
                                doc_ids_to_delete.push(external_id);
                            }
                        }
                    }
                }
            }
        }
    }
    drop(rtxn);

    let count = doc_ids_to_delete.len();
    if count > 0 {
        delete_documents(index, indexer_config, &doc_ids_to_delete)?;
        tracing::info!(
            "Deleted {} documents from index for collection {}",
            count,
            collection_id
        );
    }

    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_indexer_config() -> IndexerConfig {
        IndexerConfig::default()
    }

    #[test]
    fn test_open_index() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();

        let rtxn = index.read_txn().unwrap();
        let count = index.number_of_documents(&rtxn).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_index_reopens() {
        let temp_dir = tempfile::tempdir().unwrap();

        {
            let _index = open_index(temp_dir.path()).unwrap();
        }

        let index = open_index(temp_dir.path()).unwrap();
        let rtxn = index.read_txn().unwrap();
        let count = index.number_of_documents(&rtxn).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_index_and_search_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(
            &index,
            &config,
            "doc1",
            "test.pdf",
            "This is a test document about climate change.",
            "collection1",
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should find the document
        let results = search_index(&index, "climate", 10, 0, None).unwrap();
        assert_eq!(results.hits.len(), 1);

        // Verify we can retrieve the document fields
        let name = get_document_field(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("test.pdf".to_string()));
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(&index, &config, "doc1", "test.pdf", "Hello world", "col1").unwrap();

        let results = search_index(&index, "nonexistent", 10, 0, None).unwrap();
        assert!(results.hits.is_empty());
    }

    #[test]
    fn test_filter_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(&index, &config, "doc1", "a.pdf", "Climate research paper", "climate")
            .unwrap();
        index_document(&index, &config, "doc2", "b.pdf", "Climate news article", "news").unwrap();
        index_document(&index, &config, "doc3", "c.pdf", "Climate policy document", "policy")
            .unwrap();

        // Search all collections
        let all = search_index(&index, "climate", 10, 0, None).unwrap();
        assert_eq!(all.hits.len(), 3);

        // Filter to single collection
        let climate_only =
            search_index(&index, "climate", 10, 0, Some(&["climate".to_string()])).unwrap();
        assert_eq!(climate_only.hits.len(), 1);
        let name = get_document_field(&index, climate_only.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("a.pdf".to_string()));

        // Filter to multiple collections
        let two = search_index(
            &index,
            "climate",
            10,
            0,
            Some(&["climate".to_string(), "news".to_string()]),
        )
        .unwrap();
        assert_eq!(two.hits.len(), 2);
    }

    #[test]
    fn test_empty_filter_returns_all() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(&index, &config, "doc1", "a.pdf", "Test content", "col1").unwrap();
        index_document(&index, &config, "doc2", "b.pdf", "Test content", "col2").unwrap();

        // Empty filter should return all
        let results = search_index(&index, "test", 10, 0, Some(&[])).unwrap();
        assert_eq!(results.hits.len(), 2);
    }

    #[test]
    fn test_batch_indexing() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let docs = vec![
            DocToIndex {
                id: "doc1".to_string(),
                name: "a.pdf".to_string(),
                content: "First document about science".to_string(),
                collection_id: "col1".to_string(),
            },
            DocToIndex {
                id: "doc2".to_string(),
                name: "b.pdf".to_string(),
                content: "Second document about science".to_string(),
                collection_id: "col1".to_string(),
            },
            DocToIndex {
                id: "doc3".to_string(),
                name: "c.pdf".to_string(),
                content: "Third document about science".to_string(),
                collection_id: "col1".to_string(),
            },
        ];

        index_documents_batch(&index, &config, docs).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);

        let results = search_index(&index, "science", 10, 0, None).unwrap();
        assert_eq!(results.hits.len(), 3);
    }

    #[test]
    fn test_delete_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index two documents
        index_document(&index, &config, "doc1", "a.pdf", "First document", "col1").unwrap();
        index_document(&index, &config, "doc2", "b.pdf", "Second document", "col1").unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        // Delete one document
        delete_document(&index, &config, "doc1").unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should only find the remaining document
        let results = search_index(&index, "document", 10, 0, None).unwrap();
        assert_eq!(results.hits.len(), 1);
        let name = get_document_field(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("b.pdf".to_string()));
    }

    #[test]
    fn test_delete_documents_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index documents in different collections
        index_document(&index, &config, "doc1", "a.pdf", "Document alpha", "col1").unwrap();
        index_document(&index, &config, "doc2", "b.pdf", "Document beta", "col1").unwrap();
        index_document(&index, &config, "doc3", "c.pdf", "Document gamma", "col2").unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        // Delete all documents from col1
        let deleted = delete_documents_by_collection(&index, &config, "col1").unwrap();
        assert_eq!(deleted, 2);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Only col2 document should remain
        let results = search_index(&index, "document", 10, 0, None).unwrap();
        assert_eq!(results.hits.len(), 1);
        let name = get_document_field(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("c.pdf".to_string()));
    }
}
