use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use bumpalo::Bump;

use milli::documents::mmap_from_objects;
use milli::heed::{EnvOpenOptions, RoTxn};
use milli::progress::Progress;
use milli::prompt::Prompt;
use milli::score_details::ScoringStrategy;
use milli::update::new::indexer::{self, DocumentOperation};
use milli::update::{IndexerConfig, Setting};
use milli::vector::settings::{EmbedderSource, EmbeddingSettings};
use milli::vector::{embedder::manual, Embedder, RuntimeEmbedder, RuntimeEmbedders};
use milli::{FilterableAttributesRule, Index, TermsMatchingStrategy};
use serde_json::{json, Map, Value};

use std::collections::HashMap;

/// Default map size for the LMDB environment (10 GB)
/// This is the maximum size the database can grow to
const DEFAULT_MAP_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Get an embedder from the index's stored configuration
///
/// This reads the embedder settings from the index and creates an `Embedder` instance.
/// Returns None if the embedder is not configured.
fn get_embedder_from_index(
    index: &Index,
    rtxn: &RoTxn<'_>,
    embedder_name: &str,
) -> Result<Option<(Arc<Embedder>, bool)>> {
    let embedders = index.embedding_configs();

    // Check if embedder has an ID (meaning it's properly registered)
    let embedder_id = embedders.embedder_id(rtxn, embedder_name)?;
    if embedder_id.is_none() {
        return Ok(None);
    }

    // Get the embedder config
    let configs = embedders.embedding_configs(rtxn)?;
    let config = configs.iter().find(|c| c.name == embedder_name);

    match config {
        Some(cfg) => {
            // Create embedder from stored options
            let embedder = Embedder::new(cfg.config.embedder_options.clone(), 0)
                .map_err(|e| anyhow::anyhow!("Failed to create embedder: {}", e))?;
            let quantized = cfg.config.quantized.unwrap_or(false);
            Ok(Some((Arc::new(embedder), quantized)))
        }
        None => Ok(None),
    }
}

/// Create RuntimeEmbedders for user-provided vectors
///
/// This creates a minimal embedder configuration that tells milli to accept
/// pre-computed vectors from the `_vectors` field without generating new ones.
fn create_user_provided_embedders(embedder_name: &str, dimensions: usize) -> RuntimeEmbedders {
    let manual_embedder = manual::Embedder::new(manual::EmbedderOptions {
        dimensions,
        distribution: None,
    });
    let embedder = Arc::new(Embedder::UserProvided(manual_embedder));
    let prompt = Prompt::default();

    let runtime_embedder = Arc::new(RuntimeEmbedder::new(
        embedder,
        prompt,
        vec![], // no fragments for user-provided
        false,  // not quantized
    ));

    let mut map = HashMap::new();
    map.insert(embedder_name.to_string(), runtime_embedder);
    RuntimeEmbedders::new(map)
}

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

/// Configure the embedder settings for vector search
///
/// This must be called when an embedding model is configured so milli knows
/// how to index and search vectors stored in documents.
pub fn configure_embedder(
    index: &Index,
    indexer_config: &IndexerConfig,
    embedder_name: &str,
    dimensions: usize,
) -> Result<()> {
    tracing::info!(
        embedder_name = embedder_name,
        dimensions = dimensions,
        "Configuring embedder for vector search"
    );

    let mut wtxn = index.write_txn()?;
    let mut settings = milli::update::Settings::new(&mut wtxn, index, indexer_config);

    // Create userProvided embedder settings (matching milli's from_user_provided pattern)
    let embedder_settings = EmbeddingSettings {
        source: Setting::Set(EmbedderSource::UserProvided),
        model: Setting::NotSet,
        revision: Setting::NotSet,
        pooling: Setting::NotSet,
        api_key: Setting::NotSet,
        dimensions: Setting::Set(dimensions),
        binary_quantized: Setting::NotSet,
        document_template: Setting::NotSet,
        document_template_max_bytes: Setting::NotSet,
        url: Setting::NotSet,
        indexing_fragments: Setting::NotSet,
        search_fragments: Setting::NotSet,
        request: Setting::NotSet,
        response: Setting::NotSet,
        headers: Setting::NotSet,
        search_embedder: Setting::NotSet,
        indexing_embedder: Setting::NotSet,
        distribution: Setting::NotSet,
    };

    // Set the embedder configuration
    let mut embedders_map = BTreeMap::new();
    embedders_map.insert(embedder_name.to_string(), Setting::Set(embedder_settings));
    settings.set_embedder_settings(embedders_map);

    settings.execute(&|| false, &Progress::default(), Default::default())?;
    wtxn.commit()?;

    // Verify the embedder was registered with an ID
    let rtxn = index.read_txn()?;
    let embedders = index.embedding_configs();
    let embedder_id = embedders.embedder_id(&rtxn, embedder_name)?;
    let configs = embedders.embedding_configs(&rtxn)?;

    tracing::info!(
        embedder_name = embedder_name,
        embedder_id = ?embedder_id,
        config_count = configs.len(),
        "Embedder configured successfully"
    );

    if embedder_id.is_none() {
        tracing::error!("Embedder was not assigned an ID!");
    }

    Ok(())
}

/// Remove embedder configuration from the index
pub fn remove_embedder(index: &Index, indexer_config: &IndexerConfig) -> Result<()> {
    tracing::info!("Removing embedder configuration");

    let mut wtxn = index.write_txn()?;
    let mut settings = milli::update::Settings::new(&mut wtxn, index, indexer_config);
    settings.reset_embedder_settings();
    settings.execute(&|| false, &Progress::default(), Default::default())?;
    wtxn.commit()?;

    tracing::info!("Embedder configuration removed");
    Ok(())
}

/// Log current embedder configurations in the index (for debugging)
pub fn log_embedder_configs(index: &Index) -> Result<()> {
    let rtxn = index.read_txn()?;
    let embedders = index.embedding_configs();
    let configs = embedders.embedding_configs(&rtxn)?;

    if configs.is_empty() {
        tracing::info!("No embedders configured in index");
    } else {
        for config in &configs {
            // Check if embedder has an ID assigned
            let embedder_id = embedders.embedder_id(&rtxn, &config.name)?;
            tracing::info!(
                name = config.name,
                embedder_id = ?embedder_id,
                "Embedder configured in index"
            );
        }
    }
    Ok(())
}

/// Check if embedder is properly configured for vector search
pub fn check_embedder_ready(index: &Index, embedder_name: &str) -> Result<bool> {
    let rtxn = index.read_txn()?;
    let embedders = index.embedding_configs();

    // Check if embedder config exists
    let configs = embedders.embedding_configs(&rtxn)?;
    let has_config = configs.iter().any(|c| c.name == embedder_name);

    // Check if embedder has an ID
    let embedder_id = embedders.embedder_id(&rtxn, embedder_name)?;

    tracing::debug!(
        embedder_name = embedder_name,
        has_config = has_config,
        embedder_id = ?embedder_id,
        "Embedder readiness check"
    );

    Ok(has_config && embedder_id.is_some())
}

// ============================================================================
// Document Indexing
// ============================================================================

/// Document to be indexed
pub struct DocToIndex {
    pub id: String,
    pub name: String,
    pub content: String,
    pub collection_id: String,
    /// Pre-computed embedding vectors (one per chunk). If None, no vectors stored.
    pub vectors: Option<Vec<Vec<f32>>>,
}

/// Index a single document in milli
pub fn index_document(
    index: &Index,
    indexer_config: &IndexerConfig,
    doc_id: &str,
    name: &str,
    content: &str,
    collection_id: &str,
    vectors: Option<Vec<Vec<f32>>>,
) -> Result<()> {
    index_documents_batch(
        index,
        indexer_config,
        vec![DocToIndex {
            id: doc_id.to_string(),
            name: name.to_string(),
            content: content.to_string(),
            collection_id: collection_id.to_string(),
            vectors,
        }],
    )
}

/// Maximum documents per indexing chunk
const BATCH_CHUNK_SIZE: usize = 50;

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
    let num_chunks = total.div_ceil(BATCH_CHUNK_SIZE);
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
            // Add pre-computed vectors if present
            if let Some(ref vectors) = doc.vectors {
                m.insert("_vectors".to_string(), json!({ "default": vectors }));
            }
            m
        })
        .collect();

    let mmap = mmap_from_objects(json_docs);

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

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

    // Create RuntimeEmbedders if any documents have vectors
    // We need to tell milli about the embedder so it indexes the pre-computed vectors
    let embedders = docs
        .iter()
        .find_map(|doc| {
            doc.vectors
                .as_ref()
                .and_then(|vecs| vecs.first().map(|v| v.len()))
        })
        .map(|dimensions| create_user_provided_embedders("default", dimensions))
        .unwrap_or_default();

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

/// Extract the global score from a list of score details.
/// For hybrid search, this combines keyword and semantic scores.
fn compute_hit_score(scores: &[milli::score_details::ScoreDetails]) -> f64 {
    milli::score_details::ScoreDetails::global_score(scores.iter())
}

/// Parameters for searching the index
pub struct SearchParams<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub offset: usize,
    pub collection_ids: Option<&'a [String]>,
    /// Pre-computed query embedding for semantic search
    pub query_vector: Option<Vec<f32>>,
    /// Balance between keyword (0.0) and semantic (1.0) search
    pub semantic_ratio: f32,
    /// Filter out results below this score threshold
    pub min_score: Option<f32>,
}

impl Default for SearchParams<'_> {
    fn default() -> Self {
        Self {
            query: "",
            limit: 20,
            offset: 0,
            collection_ids: None,
            query_vector: None,
            semantic_ratio: 0.0,
            min_score: None,
        }
    }
}

/// Search the milli index
///
/// When `semantic_ratio > 0` and `query_vector` is provided, performs hybrid search
/// combining keyword (BM25) and vector similarity. The ratio controls the balance:
/// - 0.0 = pure keyword search
/// - 1.0 = pure semantic search
/// - 0.5 = equal weight to both
///
/// The `min_score` parameter filters out results below the threshold (0.0 to 1.0).
/// This is especially useful for semantic search to avoid returning irrelevant documents.
pub fn search_index(index: &Index, params: SearchParams<'_>) -> Result<SearchResults> {
    let SearchParams {
        query,
        limit,
        offset,
        collection_ids,
        query_vector,
        semantic_ratio,
        min_score,
    } = params;
    let rtxn = index.read_txn()?;
    let mut search = milli::Search::new(&rtxn, index);
    search.query(query);
    search.limit(limit);
    search.offset(offset);
    search.scoring_strategy(ScoringStrategy::Detailed);
    search.exhaustive_number_hits(true);
    search.terms_matching_strategy(TermsMatchingStrategy::Last);

    // Build filter string (must outlive the search)
    let filter_str = collection_ids.filter(|ids| !ids.is_empty()).map(|ids| {
        let quoted: Vec<String> = ids.iter().map(|id| format!("\"{}\"", id)).collect();
        format!("collection_id IN [{}]", quoted.join(", "))
    });

    if let Some(ref fs) = filter_str {
        let filter =
            milli::Filter::from_str(fs).map_err(|e| anyhow::anyhow!("Filter error: {:?}", e))?;
        if let Some(f) = filter {
            search.filter(f);
        }
    }

    // Use hybrid search when semantic_ratio > 0 and query vector is available
    let result = if let Some(query_vec) = query_vector.filter(|_| semantic_ratio > 0.0) {
        let embedder_name = "default";

        // Get embedder from index config (the proper way, matching meilisearch)
        match get_embedder_from_index(index, &rtxn, embedder_name)? {
            Some((embedder, quantized)) => {
                tracing::debug!(
                    query = query,
                    semantic_ratio = semantic_ratio,
                    vector_dims = query_vec.len(),
                    quantized = quantized,
                    "Executing hybrid search with index embedder"
                );

                // Configure semantic search with pre-computed query vector
                search.semantic(
                    embedder_name.to_string(),
                    embedder,
                    quantized,
                    Some(query_vec),
                    None, // no media
                );

                match search.execute_hybrid(semantic_ratio) {
                    Ok((result, semantic_hit_count)) => {
                        tracing::debug!(
                            total_candidates = result.candidates.len(),
                            doc_count = result.documents_ids.len(),
                            semantic_hits = ?semantic_hit_count,
                            "Hybrid search completed"
                        );
                        result
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "Hybrid search failed, falling back to keyword");
                        search.execute()?
                    }
                }
            }
            None => {
                tracing::warn!(
                    embedder_name = embedder_name,
                    "Embedder not configured in index, falling back to keyword search"
                );
                search.execute()?
            }
        }
    } else {
        search.execute()?
    };

    // Collect all hits with their scores
    let all_hits: Vec<SearchHit> = result
        .documents_ids
        .into_iter()
        .zip(result.document_scores)
        .map(|(doc_id, scores)| SearchHit { doc_id, scores })
        .collect();

    // Filter by minimum score if specified
    let (hits, total_hits) = if let Some(threshold) = min_score {
        let threshold_f64 = threshold as f64;
        let filtered: Vec<SearchHit> = all_hits
            .into_iter()
            .filter(|hit| compute_hit_score(&hit.scores) >= threshold_f64)
            .collect();
        let count = filtered.len();
        tracing::debug!(
            threshold = threshold,
            filtered_count = count,
            "Applied minimum score filter"
        );
        (filtered, count)
    } else {
        // No filtering - use candidates count as before for keyword search
        let count = if semantic_ratio > 0.0 {
            // For semantic search without min_score, count actual hits
            all_hits.len()
        } else {
            // For keyword search, use the full candidate count
            result.candidates.len() as usize
        };
        (all_hits, count)
    };

    Ok(SearchResults { hits, total_hits })
}

/// Extract a string field from an indexed document (opens its own transaction)
pub fn get_document_field_by_internal_id(
    index: &Index,
    doc_id: u32,
    field: &str,
) -> Result<Option<String>> {
    let rtxn = index.read_txn()?;
    get_document_field(index, &rtxn, doc_id, field)
}

/// Extract a string field from an indexed document using an existing transaction
pub fn get_document_field(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    doc_id: u32,
    field: &str,
) -> Result<Option<String>> {
    let fields_ids_map = index.fields_ids_map(rtxn)?;

    let docs = index.documents(rtxn, [doc_id])?;
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

/// Get the content of a document by its external ID
pub fn get_document_by_external_id(index: &Index, external_id: &str) -> Result<Option<String>> {
    let rtxn = index.read_txn()?;
    let fields_ids_map = index.fields_ids_map(&rtxn)?;

    let id_fid = match fields_ids_map.id("id") {
        Some(fid) => fid,
        None => return Ok(None),
    };
    let content_fid = match fields_ids_map.id("content") {
        Some(fid) => fid,
        None => return Ok(None),
    };

    // Iterate through all documents to find the one with matching external ID
    let all_doc_ids: Vec<u32> = index.documents_ids(&rtxn)?.iter().collect();

    for internal_id in all_doc_ids {
        let docs = index.documents(&rtxn, [internal_id])?;
        if let Some((_, obkv)) = docs.first() {
            if let Some(id_bytes) = obkv.get(id_fid) {
                if let Ok(doc_ext_id) = serde_json::from_slice::<String>(id_bytes) {
                    if doc_ext_id == external_id {
                        // Found the document, get its content
                        if let Some(content_bytes) = obkv.get(content_fid) {
                            if let Ok(content) = serde_json::from_slice::<String>(content_bytes) {
                                return Ok(Some(content));
                            }
                        }
                        return Ok(None);
                    }
                }
            }
        }
    }

    Ok(None)
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
                RuntimeEmbedders::default(),
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
            None, // No vectors for text-only search
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should find the document (keyword-only, no semantic)
        let results = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);

        // Verify we can retrieve the document fields
        let name =
            get_document_field_by_internal_id(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("test.pdf".to_string()));
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(
            &index,
            &config,
            "doc1",
            "test.pdf",
            "Hello world",
            "col1",
            None,
        )
        .unwrap();

        let results = search_index(
            &index,
            SearchParams {
                query: "nonexistent",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(results.hits.is_empty());
    }

    #[test]
    fn test_filter_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(
            &index,
            &config,
            "doc1",
            "a.pdf",
            "Climate research paper",
            "climate",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc2",
            "b.pdf",
            "Climate news article",
            "news",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc3",
            "c.pdf",
            "Climate policy document",
            "policy",
            None,
        )
        .unwrap();

        // Search all collections
        let all = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(all.hits.len(), 3);

        // Filter to single collection
        let climate_only = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                collection_ids: Some(&["climate".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(climate_only.hits.len(), 1);
        let name =
            get_document_field_by_internal_id(&index, climate_only.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("a.pdf".to_string()));

        // Filter to multiple collections
        let two = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                collection_ids: Some(&["climate".to_string(), "news".to_string()]),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(two.hits.len(), 2);
    }

    #[test]
    fn test_empty_filter_returns_all() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        index_document(
            &index,
            &config,
            "doc1",
            "a.pdf",
            "Test content",
            "col1",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc2",
            "b.pdf",
            "Test content",
            "col2",
            None,
        )
        .unwrap();

        // Empty filter should return all
        let results = search_index(
            &index,
            SearchParams {
                query: "test",
                limit: 10,
                collection_ids: Some(&[]),
                ..Default::default()
            },
        )
        .unwrap();
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
                vectors: None,
            },
            DocToIndex {
                id: "doc2".to_string(),
                name: "b.pdf".to_string(),
                content: "Second document about science".to_string(),
                collection_id: "col1".to_string(),
                vectors: None,
            },
            DocToIndex {
                id: "doc3".to_string(),
                name: "c.pdf".to_string(),
                content: "Third document about science".to_string(),
                collection_id: "col1".to_string(),
                vectors: None,
            },
        ];

        index_documents_batch(&index, &config, docs).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);

        let results = search_index(
            &index,
            SearchParams {
                query: "science",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 3);
    }

    #[test]
    fn test_delete_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index two documents
        index_document(
            &index,
            &config,
            "doc1",
            "a.pdf",
            "First document",
            "col1",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc2",
            "b.pdf",
            "Second document",
            "col1",
            None,
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        // Delete one document
        delete_document(&index, &config, "doc1").unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should only find the remaining document
        let results = search_index(
            &index,
            SearchParams {
                query: "document",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
        let name =
            get_document_field_by_internal_id(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("b.pdf".to_string()));
    }

    #[test]
    fn test_delete_documents_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index documents in different collections
        index_document(
            &index,
            &config,
            "doc1",
            "a.pdf",
            "Document alpha",
            "col1",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc2",
            "b.pdf",
            "Document beta",
            "col1",
            None,
        )
        .unwrap();
        index_document(
            &index,
            &config,
            "doc3",
            "c.pdf",
            "Document gamma",
            "col2",
            None,
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        // Delete all documents from col1
        let deleted = delete_documents_by_collection(&index, &config, "col1").unwrap();
        assert_eq!(deleted, 2);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Only col2 document should remain
        let results = search_index(
            &index,
            SearchParams {
                query: "document",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
        let name =
            get_document_field_by_internal_id(&index, results.hits[0].doc_id, "name").unwrap();
        assert_eq!(name, Some("c.pdf".to_string()));
    }

    #[test]
    fn test_configure_embedder() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Configure embedder
        configure_embedder(&index, &config, "default", 384).unwrap();

        // Verify embedder is registered
        let rtxn = index.read_txn().unwrap();
        let embedders = index.embedding_configs();
        let embedder_id = embedders.embedder_id(&rtxn, "default").unwrap();
        assert!(embedder_id.is_some(), "Embedder should have an ID assigned");

        let configs = embedders.embedding_configs(&rtxn).unwrap();
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "default");
    }

    #[test]
    fn test_semantic_search_with_vectors() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // IMPORTANT: Configure embedder BEFORE indexing documents with vectors
        configure_embedder(&index, &config, "default", 3).unwrap();

        // Create simple 3D vectors for testing
        // doc1: about cats [1, 0, 0]
        // doc2: about dogs [0, 1, 0]
        // doc3: also about cats [0.9, 0.1, 0]
        let cat_vec = vec![vec![1.0, 0.0, 0.0]];
        let dog_vec = vec![vec![0.0, 1.0, 0.0]];
        let also_cat_vec = vec![vec![0.9, 0.1, 0.0]];

        index_document(
            &index,
            &config,
            "doc1",
            "cats.pdf",
            "A document about cats and felines.",
            "collection1",
            Some(cat_vec),
        )
        .unwrap();

        index_document(
            &index,
            &config,
            "doc2",
            "dogs.pdf",
            "A document about dogs and canines.",
            "collection1",
            Some(dog_vec),
        )
        .unwrap();

        index_document(
            &index,
            &config,
            "doc3",
            "more_cats.pdf",
            "Another document about cats.",
            "collection1",
            Some(also_cat_vec),
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        // Query with a cat-like vector [1, 0, 0] - should rank cat documents higher
        let query_vector = vec![1.0, 0.0, 0.0];
        let results = search_index(
            &index,
            SearchParams {
                query: "document",
                limit: 10,
                query_vector: Some(query_vector),
                semantic_ratio: 0.8, // High semantic ratio
                ..Default::default()
            },
        )
        .unwrap();

        // Should find all 3 documents
        assert!(results.hits.len() >= 2, "Should find at least 2 documents");

        // The cat documents should be ranked higher (first)
        let first_name =
            get_document_field_by_internal_id(&index, results.hits[0].doc_id, "name").unwrap();
        assert!(
            first_name == Some("cats.pdf".to_string())
                || first_name == Some("more_cats.pdf".to_string()),
            "First result should be a cat document, got {:?}",
            first_name
        );
    }
}
