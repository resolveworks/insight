mod index_worker;

pub use index_worker::{spawn_index_worker, IndexWorkerHandle};

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use bumpalo::Bump;
use fst::Streamer;

use http_client::policy::IpPolicy;
use milli::documents::mmap_from_objects;
use milli::heed::EnvOpenOptions;
use milli::progress::{EmbedderStats, Progress};
use milli::score_details::ScoringStrategy;
use milli::update::new::indexer::{self, IndexOperations};
use milli::update::{IndexerConfig, MissingDocumentPolicy};
use milli::vector::RuntimeEmbedders;
use milli::{
    CreateOrOpen, Filter, FilterCondition, FilterableAttributesRule, Index, IndexFilter,
    IndexFilterCondition, TermsMatchingStrategy,
};
use roaring::RoaringBitmap;
use serde_json::{Map, Value};

/// Default map size for the LMDB environment (10 GB)
/// This is the maximum size the database can grow to
const DEFAULT_MAP_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// Parse a filter string directly into an `IndexFilter`.
///
/// milli's `Filter::from_str` produces a `Filter`, but `Search::filter` now wants
/// an `IndexFilter` (the variant without `Foreign` conditions). We only ever build
/// simple field/IN filters, so conversion is mechanical.
fn parse_index_filter(expr: &str) -> Result<Option<IndexFilter<'_>>> {
    let Some(filter) =
        Filter::from_str(expr).map_err(|e| anyhow::anyhow!("Filter error: {e:?}"))?
    else {
        return Ok(None);
    };
    Ok(Some(IndexFilter::from(condition_to_index_condition(
        filter.condition,
    ))))
}

fn condition_to_index_condition(cond: FilterCondition<'_>) -> IndexFilterCondition<'_> {
    match cond {
        FilterCondition::Not(inner) => {
            IndexFilterCondition::Not(Box::new(condition_to_index_condition(*inner)))
        }
        FilterCondition::Condition { fid, op } => IndexFilterCondition::Condition { fid, op },
        FilterCondition::In { fid, els } => IndexFilterCondition::In { fid, els },
        FilterCondition::Or(children) => IndexFilterCondition::Or(
            children
                .into_iter()
                .map(condition_to_index_condition)
                .collect(),
        ),
        FilterCondition::And(children) => IndexFilterCondition::And(
            children
                .into_iter()
                .map(condition_to_index_condition)
                .collect(),
        ),
        FilterCondition::VectorExists {
            fid,
            embedder,
            filter,
        } => IndexFilterCondition::VectorExists {
            fid,
            embedder,
            filter,
        },
        FilterCondition::GeoLowerThan {
            point,
            radius,
            resolution,
        } => IndexFilterCondition::GeoLowerThan {
            point,
            radius,
            resolution,
        },
        FilterCondition::GeoBoundingBox {
            top_right_point,
            bottom_left_point,
        } => IndexFilterCondition::GeoBoundingBox {
            top_right_point,
            bottom_left_point,
        },
        FilterCondition::GeoPolygon { points } => IndexFilterCondition::GeoPolygon { points },
        FilterCondition::Foreign { .. } => {
            unreachable!("foreign filters are never constructed by insight-core")
        }
    }
}

/// Open or create a milli search index
pub fn open_index(path: &Path) -> Result<Index> {
    std::fs::create_dir_all(path)?;

    let mut env_options = EnvOpenOptions::new();
    env_options.map_size(DEFAULT_MAP_SIZE);
    let env_options = env_options.read_txn_without_tls();

    let index = Index::new(env_options, path, CreateOrOpen::create_without_shards())
        .context("Failed to create milli index")?;

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
        settings.execute(
            &|| false,
            &Progress::default(),
            &IpPolicy::danger_always_allow(),
            Arc::new(EmbedderStats::default()),
        )?;
        wtxn.commit()?;
        tracing::info!("Configured primary key and filterable attributes");
    }

    tracing::info!("Search index opened at {:?}", path);

    Ok(index)
}

// ============================================================================
// Indexing
// ============================================================================

/// A text chunk to be indexed (for search)
///
/// Documents are split into chunks for more precise search results.
/// Each chunk is indexed separately as a BM25 keyword-search document.
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
#[allow(clippy::result_large_err)]
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
            m
        })
        .collect();

    let mmap = mmap_from_objects(json_docs);

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let mut operation = IndexOperations::new();
    operation.replace_documents(&mmap, MissingDocumentPolicy::Create)?;

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
                RuntimeEmbedders::default(), // no embedders: keyword-only index
                &|| false,
                &Progress::default(),
                &IpPolicy::danger_always_allow(),
                &EmbedderStats::default(),
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
pub fn compute_hit_score(scores: &[milli::score_details::ScoreDetails]) -> f64 {
    milli::score_details::ScoreDetails::global_score(scores.iter())
}

/// Parameters for searching the index
pub struct SearchParams<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub offset: usize,
    pub collection_ids: Option<&'a [String]>,
}

impl Default for SearchParams<'_> {
    fn default() -> Self {
        Self {
            query: "",
            limit: 20,
            offset: 0,
            collection_ids: None,
        }
    }
}

/// Execute a keyword (BM25) search against the milli index.
pub fn search_index(index: &Index, params: SearchParams<'_>) -> Result<SearchResults> {
    let SearchParams {
        query,
        limit,
        offset,
        collection_ids,
    } = params;

    let rtxn = index.read_txn()?;
    let progress = Progress::default();
    let mut search = milli::Search::new(&rtxn, index, &progress);
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
        if let Some(f) = parse_index_filter(fs)? {
            search.filter(f);
        }
    }

    // Execute keyword (BM25) search
    let result = search.execute()?;

    // Collect hits
    let hits: Vec<SearchHit> = result
        .documents_ids
        .into_iter()
        .zip(result.document_scores)
        .map(|(doc_id, scores)| SearchHit { doc_id, scores })
        .collect();
    let total_hits = result.candidates.len() as usize;

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
    let progress = Progress::default();
    let mut search = milli::Search::new(&rtxn, index, &progress);
    search.query("");
    search.limit(usize::MAX);
    if let Some(f) = parse_index_filter(&filter_str)? {
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

/// A term with its document frequency
#[derive(Debug, Clone)]
pub struct TermFrequency {
    pub term: String,
    pub doc_count: u64,
}

/// Get the most common terms in a collection (by document frequency)
///
/// If `collection_ids` is None, returns terms across all collections.
/// Terms are sorted by frequency (descending) and limited to `limit` results.
///
/// This is useful for:
/// - Giving the LLM context about what topics the collection covers
/// - Suggesting auto-tags for documents
/// - Showing users what they can search for
pub fn get_collection_terms(
    index: &Index,
    collection_ids: Option<&[String]>,
    limit: usize,
) -> Result<Vec<TermFrequency>> {
    let rtxn = index.read_txn()?;

    // Get document IDs in the target collections (if filtered)
    let target_docs: Option<RoaringBitmap> = if let Some(ids) = collection_ids {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Build filter for collection_id
        let quoted: Vec<String> = ids.iter().map(|id| format!("\"{}\"", id)).collect();
        let filter_str = format!("collection_id IN [{}]", quoted.join(", "));

        let progress = Progress::default();
        let mut search = milli::Search::new(&rtxn, index, &progress);
        search.query("");
        search.limit(usize::MAX);
        if let Some(f) = parse_index_filter(&filter_str)? {
            search.filter(f);
        }

        let result = search.execute()?;
        Some(RoaringBitmap::from_iter(result.documents_ids))
    } else {
        None
    };

    // Get the words FST (contains all indexed words)
    let fst = index.words_fst(&rtxn)?;

    let mut term_counts: Vec<TermFrequency> = Vec::new();

    // Iterate all words in the FST
    let mut stream = fst.stream();
    while let Some(word_bytes) = stream.next() {
        let term = match std::str::from_utf8(word_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => continue, // Skip non-UTF8 words
        };

        // Skip very short terms (likely noise)
        if term.len() < 3 {
            continue;
        }

        // Look up document frequency in word_docids
        if let Some(word_bitmap) = index.word_docids.get(&rtxn, &term)? {
            let doc_count = match &target_docs {
                Some(target) => {
                    // Count intersection with target collection docs
                    word_bitmap.intersection_len(target)
                }
                None => word_bitmap.len(),
            };

            if doc_count > 0 {
                term_counts.push(TermFrequency { term, doc_count });
            }
        }
    }

    // Sort by frequency (descending) and take top N
    term_counts.sort_by_key(|t| std::cmp::Reverse(t.doc_count));
    term_counts.truncate(limit);

    Ok(term_counts)
}

/// Delete chunks by their IDs
#[allow(clippy::result_large_err)]
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

    let mut operation = IndexOperations::new();
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
                &IpPolicy::danger_always_allow(),
                &EmbedderStats::default(),
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
            ),
            make_chunk(
                "doc2",
                "b.pdf",
                "Second document about science",
                "col1",
            ),
            make_chunk(
                "doc3",
                "c.pdf",
                "Third document about science",
                "col1",
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
    fn test_search_by_document_name_in_enriched_content() {
        // Test that enriched content (with document name prefix) allows
        // searching by document name, not just content keywords.
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Simulate enriched content as produced by jobs/index.rs
        // Format: "[document_name]\n\n{content}"
        let chunks = vec![
            ChunkToIndex {
                id: "doc1_chunk_0".to_string(),
                parent_id: "doc1".to_string(),
                parent_name: "Climate_Report_2024.pdf".to_string(),
                chunk_index: 0,
                content: "[Climate_Report_2024.pdf]\n\nGlobal temperatures continue to rise."
                    .to_string(),
                collection_id: "research".to_string(),
                page_count: 10,
                start_page: 1,
                end_page: 1,
            },
            ChunkToIndex {
                id: "doc2_chunk_0".to_string(),
                parent_id: "doc2".to_string(),
                parent_name: "Financial_Summary.pdf".to_string(),
                chunk_index: 0,
                content: "[Financial_Summary.pdf]\n\nQ4 revenue exceeded expectations.".to_string(),
                collection_id: "finance".to_string(),
                page_count: 5,
                start_page: 1,
                end_page: 1,
            },
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        // Search by document name - should find the climate report
        let results = search_index(
            &index,
            SearchParams {
                query: "Climate_Report",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("Climate_Report_2024.pdf".to_string()));

        // Search by content should still work
        let results = search_index(
            &index,
            SearchParams {
                query: "revenue",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("Financial_Summary.pdf".to_string()));
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
    fn test_get_collection_terms_empty_index() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();

        let terms = get_collection_terms(&index, None, 50).unwrap();
        assert!(terms.is_empty());
    }

    #[test]
    fn test_get_collection_terms_basic() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index documents with overlapping terms
        let chunks = vec![
            make_chunk(
                "doc1",
                "climate.pdf",
                "Climate change affects global temperatures and weather patterns.",
                "research",
            ),
            make_chunk(
                "doc2",
                "weather.pdf",
                "Weather patterns are changing due to climate shifts.",
                "research",
            ),
            make_chunk(
                "doc3",
                "finance.pdf",
                "Financial markets respond to climate policy changes.",
                "finance",
            ),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        // Get top terms across all collections
        let terms = get_collection_terms(&index, None, 10).unwrap();

        // Should have terms (climate appears in all 3)
        assert!(!terms.is_empty());

        // "climate" should be a top term (appears in all 3 docs)
        let climate_term = terms.iter().find(|t| t.term == "climate");
        assert!(
            climate_term.is_some(),
            "Expected 'climate' to be in top terms"
        );
        assert_eq!(climate_term.unwrap().doc_count, 3);
    }

    #[test]
    fn test_get_collection_terms_filtered_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index documents in different collections
        let chunks = vec![
            make_chunk(
                "doc1",
                "climate.pdf",
                "Climate research and environmental studies.",
                "research",
            ),
            make_chunk(
                "doc2",
                "finance.pdf",
                "Financial analysis and market trends.",
                "finance",
            ),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        // Get terms only from "research" collection
        let collection_ids = vec!["research".to_string()];
        let terms = get_collection_terms(&index, Some(&collection_ids), 10).unwrap();

        // Should have terms from research collection
        assert!(!terms.is_empty());

        // "climate" should be present (from research)
        let has_climate = terms.iter().any(|t| t.term == "climate");
        assert!(has_climate, "Expected 'climate' in research terms");

        // "financial" should NOT be present (from finance collection only)
        let has_financial = terms.iter().any(|t| t.term == "financial");
        assert!(
            !has_financial,
            "Did not expect 'financial' in research terms"
        );
    }

    #[test]
    fn test_get_collection_terms_respects_limit() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index a document with many unique terms
        let chunk = make_chunk(
            "doc1",
            "varied.pdf",
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda",
            "col1",
        );
        index_chunks_batch(&index, &config, vec![chunk]).unwrap();

        // Request only 3 terms
        let terms = get_collection_terms(&index, None, 3).unwrap();
        assert!(terms.len() <= 3, "Should return at most 3 terms");
    }
}
