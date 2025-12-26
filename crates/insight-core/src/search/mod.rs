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

    // Configure filterable attributes for collection and document filtering
    let needs_setup = {
        let rtxn = index.read_txn()?;
        let current_rules = index.filterable_attributes_rules(&rtxn)?;
        // Check if both collection_id and parent_id are configured
        let has_collection = current_rules
            .iter()
            .any(|rule| matches!(rule, FilterableAttributesRule::Field(f) if f == "collection_id"));
        let has_parent = current_rules
            .iter()
            .any(|rule| matches!(rule, FilterableAttributesRule::Field(f) if f == "parent_id"));
        !has_collection || !has_parent
    };

    if needs_setup {
        let indexer_config = IndexerConfig::default();
        let mut wtxn = index.write_txn()?;
        let mut settings = milli::update::Settings::new(&mut wtxn, &index, &indexer_config);
        settings.set_primary_key("id".to_string());
        settings.set_filterable_fields(vec![
            FilterableAttributesRule::Field("collection_id".to_string()),
            FilterableAttributesRule::Field("parent_id".to_string()),
        ]);
        settings.execute(&|| false, &Progress::default(), Default::default())?;
        wtxn.commit()?;
        tracing::info!("Configured primary key and filterable attributes");
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
// Indexing
// ============================================================================

/// A text chunk to be indexed (for search)
///
/// Documents are split into chunks for more precise search results.
/// Each chunk is indexed separately with its own embedding vector.
pub struct ChunkToIndex {
    /// Unique chunk ID: "{parent_id}_chunk_{chunk_index}"
    pub id: String,
    /// Original document ID (for grouping and full-text retrieval)
    pub parent_id: String,
    /// Document filename (for display in results)
    pub parent_name: String,
    /// Position of this chunk in the document (0-indexed)
    pub chunk_index: usize,
    /// The chunk text content (~450 tokens)
    pub content: String,
    /// Collection this chunk belongs to
    pub collection_id: String,
    /// Number of pages in the parent document
    pub page_count: usize,
    /// First page this chunk appears on (1-indexed)
    pub start_page: usize,
    /// Last page this chunk appears on (1-indexed)
    pub end_page: usize,
    /// Pre-computed embedding vector for this chunk
    pub vector: Option<Vec<f32>>,
}

/// Maximum chunks per indexing batch
const BATCH_CHUNK_SIZE: usize = 50;

/// Index multiple chunks in a single batch operation
pub fn index_chunks_batch(
    index: &Index,
    indexer_config: &IndexerConfig,
    chunks: Vec<ChunkToIndex>,
) -> Result<()> {
    if chunks.is_empty() {
        return Ok(());
    }

    let total = chunks.len();

    // Process in batches to avoid stack overflow with large batches
    let num_batches = total.div_ceil(BATCH_CHUNK_SIZE);
    for (batch_idx, batch) in chunks.chunks(BATCH_CHUNK_SIZE).enumerate() {
        index_chunk_batch(index, indexer_config, batch)?;
        tracing::info!(
            "Indexed batch {}/{} ({} chunks, {}% complete)",
            batch_idx + 1,
            num_batches,
            batch.len(),
            ((batch_idx + 1) * 100) / num_batches
        );
    }

    tracing::debug!("Indexed batch of {} chunks total", total);

    Ok(())
}

/// Index a batch of chunks
fn index_chunk_batch(
    index: &Index,
    indexer_config: &IndexerConfig,
    chunks: &[ChunkToIndex],
) -> Result<()> {
    let json_docs: Vec<Map<String, Value>> = chunks
        .iter()
        .map(|chunk| {
            let mut m = Map::new();
            m.insert("id".to_string(), Value::String(chunk.id.clone()));
            m.insert(
                "parent_id".to_string(),
                Value::String(chunk.parent_id.clone()),
            );
            m.insert(
                "parent_name".to_string(),
                Value::String(chunk.parent_name.clone()),
            );
            m.insert(
                "chunk_index".to_string(),
                Value::Number(chunk.chunk_index.into()),
            );
            m.insert("content".to_string(), Value::String(chunk.content.clone()));
            m.insert(
                "collection_id".to_string(),
                Value::String(chunk.collection_id.clone()),
            );
            m.insert(
                "page_count".to_string(),
                Value::Number(chunk.page_count.into()),
            );
            m.insert(
                "start_page".to_string(),
                Value::Number(chunk.start_page.into()),
            );
            m.insert("end_page".to_string(), Value::Number(chunk.end_page.into()));
            // Add pre-computed vector if present (single vector, not array)
            if let Some(ref vector) = chunk.vector {
                m.insert("_vectors".to_string(), json!({ "default": [vector] }));
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

    // Create RuntimeEmbedders if any chunks have vectors
    // We need to tell milli about the embedder so it indexes the pre-computed vectors
    let embedders = chunks
        .iter()
        .find_map(|chunk| chunk.vector.as_ref().map(|v| v.len()))
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

/// Execute hybrid search with semantic + keyword scoring
fn execute_hybrid_search<'a>(
    index: &Index,
    rtxn: &'a milli::heed::RoTxn<'a>,
    search: &mut milli::Search<'a>,
    query_vector: Vec<f32>,
    semantic_ratio: f32,
) -> Result<milli::SearchResult> {
    const EMBEDDER_NAME: &str = "default";

    let Some((embedder, quantized)) = get_embedder_from_index(index, rtxn, EMBEDDER_NAME)? else {
        tracing::warn!("Embedder not configured, falling back to keyword search");
        return Ok(search.execute()?);
    };

    tracing::debug!(
        semantic_ratio = semantic_ratio,
        vector_dims = query_vector.len(),
        "Executing hybrid search"
    );

    search.semantic(
        EMBEDDER_NAME.to_string(),
        embedder,
        quantized,
        Some(query_vector),
        None,
    );

    match search.execute_hybrid(semantic_ratio) {
        Ok((result, _)) => Ok(result),
        Err(e) => {
            tracing::error!(error = %e, "Hybrid search failed, falling back to keyword");
            Ok(search.execute()?)
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

    // Apply collection filter
    let filter_str = collection_ids.filter(|ids| !ids.is_empty()).map(|ids| {
        let quoted: Vec<String> = ids.iter().map(|id| format!("\"{}\"", id)).collect();
        format!("collection_id IN [{}]", quoted.join(", "))
    });
    if let Some(ref fs) = filter_str {
        if let Some(f) =
            milli::Filter::from_str(fs).map_err(|e| anyhow::anyhow!("Filter error: {:?}", e))?
        {
            search.filter(f);
        }
    }

    // Execute search (hybrid if semantic enabled, otherwise keyword-only)
    let result = match query_vector.filter(|_| semantic_ratio > 0.0) {
        Some(vec) => execute_hybrid_search(index, &rtxn, &mut search, vec, semantic_ratio)?,
        None => search.execute()?,
    };

    // Collect hits
    let all_hits: Vec<SearchHit> = result
        .documents_ids
        .into_iter()
        .zip(result.document_scores)
        .map(|(doc_id, scores)| SearchHit { doc_id, scores })
        .collect();

    // Apply minimum score filter
    let (hits, total_hits) = match min_score {
        Some(threshold) => {
            let filtered: Vec<_> = all_hits
                .into_iter()
                .filter(|hit| compute_hit_score(&hit.scores) >= threshold as f64)
                .collect();
            let count = filtered.len();
            (filtered, count)
        }
        None => {
            let count = if semantic_ratio > 0.0 {
                all_hits.len()
            } else {
                result.candidates.len() as usize
            };
            (all_hits, count)
        }
    };

    Ok(SearchResults { hits, total_hits })
}

/// Get a document as a JSON object using an existing transaction
pub fn get_document(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    doc_id: u32,
) -> Result<Option<serde_json::Map<String, serde_json::Value>>> {
    let fields_ids_map = index.fields_ids_map(rtxn)?;

    let docs = index.documents(rtxn, [doc_id])?;
    if let Some((_id, obkv)) = docs.first() {
        let obj = milli::all_obkv_to_json(obkv, &fields_ids_map)?;
        Ok(Some(obj))
    } else {
        Ok(None)
    }
}

/// Get the content of a document by its external ID
pub fn get_document_by_external_id(index: &Index, external_id: &str) -> Result<Option<String>> {
    let rtxn = index.read_txn()?;

    // O(1) lookup: external ID -> internal ID via B-tree
    let external_ids = index.external_documents_ids();
    match external_ids.get(&rtxn, external_id)? {
        Some(internal_id) => {
            let doc = get_document(index, &rtxn, internal_id)?;
            Ok(doc.and_then(|d| d.get("content").and_then(|v| v.as_str()).map(String::from)))
        }
        None => Ok(None),
    }
}

/// Delete all chunks belonging to a document by its parent ID
pub fn delete_document_chunks(
    index: &Index,
    indexer_config: &IndexerConfig,
    parent_id: &str,
) -> Result<usize> {
    // Find all chunks with this parent_id
    let rtxn = index.read_txn()?;

    // Build filter for parent_id
    let filter_str = format!("parent_id = \"{}\"", parent_id);
    let mut search = milli::Search::new(&rtxn, index);
    search.query("");
    search.limit(usize::MAX);
    if let Some(f) = milli::Filter::from_str(&filter_str)
        .map_err(|e| anyhow::anyhow!("Filter error: {:?}", e))?
    {
        search.filter(f);
    }

    let result = search.execute()?;
    if result.documents_ids.is_empty() {
        return Ok(0);
    }

    // Get chunk IDs to delete
    let chunk_ids: Vec<String> = result
        .documents_ids
        .iter()
        .filter_map(|&doc_id| {
            get_document(index, &rtxn, doc_id)
                .ok()
                .flatten()
                .and_then(|d| d.get("id").and_then(|v| v.as_str()).map(String::from))
        })
        .collect();
    drop(rtxn);

    let count = chunk_ids.len();
    if count > 0 {
        delete_chunks_by_id(index, indexer_config, &chunk_ids)?;
        tracing::debug!("Deleted {} chunks for document {}", count, parent_id);
    }

    Ok(count)
}

/// Delete chunks by their IDs
fn delete_chunks_by_id(
    index: &Index,
    indexer_config: &IndexerConfig,
    chunk_ids: &[String],
) -> Result<()> {
    if chunk_ids.is_empty() {
        return Ok(());
    }

    let chunk_ids_refs: Vec<&str> = chunk_ids.iter().map(|s| s.as_str()).collect();

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let mut operation = DocumentOperation::new();
    operation.delete_documents(&chunk_ids_refs);

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
        anyhow::bail!("Chunk deletion error: {}", error);
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

    tracing::debug!("Deleted {} chunks from index", chunk_ids.len());

    Ok(())
}

/// Delete all chunks belonging to a specific collection
pub fn delete_chunks_by_collection(
    index: &Index,
    indexer_config: &IndexerConfig,
    collection_id: &str,
) -> Result<usize> {
    // Find all chunks in this collection
    let collection_ids = [collection_id.to_string()];
    let results = search_index(
        index,
        SearchParams {
            query: "",
            limit: usize::MAX,
            collection_ids: Some(&collection_ids),
            ..Default::default()
        },
    )?;

    if results.hits.is_empty() {
        return Ok(0);
    }

    // Extract chunk IDs from search results
    let rtxn = index.read_txn()?;
    let chunk_ids_to_delete: Vec<String> = results
        .hits
        .iter()
        .filter_map(|hit| {
            get_document(index, &rtxn, hit.doc_id)
                .ok()
                .flatten()
                .and_then(|d| d.get("id").and_then(|v| v.as_str()).map(String::from))
        })
        .collect();
    drop(rtxn);

    let count = chunk_ids_to_delete.len();
    if count > 0 {
        delete_chunks_by_id(index, indexer_config, &chunk_ids_to_delete)?;
        tracing::info!(
            "Deleted {} chunks from index for collection {}",
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

    /// Helper to create a single chunk for a document (for testing)
    fn make_chunk(
        parent_id: &str,
        parent_name: &str,
        content: &str,
        collection_id: &str,
        vector: Option<Vec<f32>>,
    ) -> ChunkToIndex {
        ChunkToIndex {
            id: format!("{}_chunk_0", parent_id),
            parent_id: parent_id.to_string(),
            parent_name: parent_name.to_string(),
            chunk_index: 0,
            content: content.to_string(),
            collection_id: collection_id.to_string(),
            page_count: 1,
            start_page: 1,
            end_page: 1,
            vector,
        }
    }

    /// Helper to get a string field from a document (for testing)
    fn get_field(index: &Index, doc_id: u32, field: &str) -> Option<String> {
        let rtxn = index.read_txn().ok()?;
        let doc = get_document(index, &rtxn, doc_id).ok()??;
        doc.get(field).and_then(|v| v.as_str()).map(String::from)
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
    fn test_index_and_search_chunk() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let chunk = make_chunk(
            "doc1",
            "test.pdf",
            "This is a test document about climate change.",
            "collection1",
            None,
        );
        index_chunks_batch(&index, &config, vec![chunk]).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should find the chunk (keyword-only, no semantic)
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

        // Verify we can retrieve the chunk fields
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("test.pdf".to_string()));
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let chunk = make_chunk("doc1", "test.pdf", "Hello world", "col1", None);
        index_chunks_batch(&index, &config, vec![chunk]).unwrap();

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

        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Climate research paper", "climate", None),
            make_chunk("doc2", "b.pdf", "Climate news article", "news", None),
            make_chunk("doc3", "c.pdf", "Climate policy document", "policy", None),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

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
        let name = get_field(&index, climate_only.hits[0].doc_id, "parent_name");
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

        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Test content", "col1", None),
            make_chunk("doc2", "b.pdf", "Test content", "col2", None),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

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

        let chunks = vec![
            make_chunk(
                "doc1",
                "a.pdf",
                "First document about science",
                "col1",
                None,
            ),
            make_chunk(
                "doc2",
                "b.pdf",
                "Second document about science",
                "col1",
                None,
            ),
            make_chunk(
                "doc3",
                "c.pdf",
                "Third document about science",
                "col1",
                None,
            ),
        ];

        index_chunks_batch(&index, &config, chunks).unwrap();

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
    fn test_delete_document_chunks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index chunks for two documents
        let chunks = vec![
            make_chunk("doc1", "a.pdf", "First document", "col1", None),
            make_chunk("doc2", "b.pdf", "Second document", "col1", None),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        // Delete chunks for one document
        let deleted = delete_document_chunks(&index, &config, "doc1").unwrap();
        assert_eq!(deleted, 1);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should only find the remaining document's chunks
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
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("b.pdf".to_string()));
    }

    #[test]
    fn test_delete_chunks_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index chunks in different collections
        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Document alpha", "col1", None),
            make_chunk("doc2", "b.pdf", "Document beta", "col1", None),
            make_chunk("doc3", "c.pdf", "Document gamma", "col2", None),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        // Delete all chunks from col1
        let deleted = delete_chunks_by_collection(&index, &config, "col1").unwrap();
        assert_eq!(deleted, 2);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Only col2 chunk should remain
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
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
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

        // IMPORTANT: Configure embedder BEFORE indexing chunks with vectors
        configure_embedder(&index, &config, "default", 3).unwrap();

        // Create simple 3D vectors for testing
        // doc1: about cats [1, 0, 0]
        // doc2: about dogs [0, 1, 0]
        // doc3: also about cats [0.9, 0.1, 0]
        let chunks = vec![
            make_chunk(
                "doc1",
                "cats.pdf",
                "A document about cats and felines.",
                "collection1",
                Some(vec![1.0, 0.0, 0.0]),
            ),
            make_chunk(
                "doc2",
                "dogs.pdf",
                "A document about dogs and canines.",
                "collection1",
                Some(vec![0.0, 1.0, 0.0]),
            ),
            make_chunk(
                "doc3",
                "more_cats.pdf",
                "Another document about cats.",
                "collection1",
                Some(vec![0.9, 0.1, 0.0]),
            ),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        // Query with a cat-like vector [1, 0, 0] - should rank cat chunks higher
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

        // Should find all 3 chunks
        assert!(results.hits.len() >= 2, "Should find at least 2 chunks");

        // The cat chunks should be ranked higher (first)
        let first_name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert!(
            first_name == Some("cats.pdf".to_string())
                || first_name == Some("more_cats.pdf".to_string()),
            "First result should be a cat document, got {:?}",
            first_name
        );
    }
}
