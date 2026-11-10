mod index_worker;

pub use index_worker::{spawn_index_worker, IndexWorkerHandle};

use std::collections::HashSet;
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
    Condition, CreateOrOpen, FilterableAttributesRule, Index, IndexFilter, IndexFilterCondition,
    MustStopProcessing, TermsMatchingStrategy, Token,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;

/// Default map size for the LMDB environment (10 GB)
/// This is the maximum size the database can grow to
const DEFAULT_MAP_SIZE: usize = 10 * 1024 * 1024 * 1024;

/// An equality condition on an indexed field.
fn indexed_field_equals(field: &str, value: &str) -> IndexFilterCondition {
    IndexFilterCondition::Condition {
        fid: Token::from_raw_text(field.to_string()),
        op: Condition::Equal(Token::from_raw_text(value.to_string())),
    }
}

/// Restrict a search to one collection.
fn collection_filter(collection_id: &str) -> IndexFilter {
    IndexFilter::from(indexed_field_equals("collection_id", collection_id))
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
            &MustStopProcessing::default(),
            &Progress::quiet(),
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

/// External ID for a document chunk, unique within the search index.
///
/// The same document ID can occur in multiple collections, so the collection
/// is part of the identity.
pub fn chunk_external_id(collection_id: &str, parent_id: &str, chunk_index: usize) -> String {
    format!("{collection_id}_{parent_id}_chunk_{chunk_index}")
}

/// A text chunk to be indexed (for search)
///
/// Documents are split into chunks for more precise search results.
/// Each chunk is indexed separately as a BM25 keyword-search document.
pub struct ChunkToIndex {
    /// Unique chunk ID, see [`chunk_external_id`].
    pub id: String,
    /// Original document ID.
    pub parent_id: String,
    /// Document filename, for display and retrieval in the UI.
    pub parent_name: String,
    /// Position of this chunk in the document (0-indexed).
    pub chunk_index: usize,
    /// Text as indexed (may carry a document-name prefix for relevance).
    pub content: String,
    /// Collection this chunk belongs to.
    pub collection_id: String,
    /// UTF-8 byte offset in the document's extracted text where the chunk starts.
    pub text_offset_start: usize,
    /// Byte offset one past the chunk's end.
    pub text_offset_end: usize,
}

/// A chunk as stored in the search index.
///
/// This mirrors the fields written by [`ChunkToIndex`], so a malformed or
/// stale row fails deserialization instead of yielding default values.
#[derive(Debug, Deserialize)]
pub struct IndexedChunk {
    /// Unique chunk ID, see [`chunk_external_id`].
    pub id: String,
    /// Original document ID.
    pub parent_id: String,
    /// Document filename.
    pub parent_name: String,
    /// Position of this chunk in the document (0-indexed).
    pub chunk_index: usize,
    /// Text as indexed (may carry a document-name prefix for relevance).
    pub content: String,
    /// Collection this chunk belongs to.
    pub collection_id: String,
    /// UTF-8 byte offset in the document's extracted text where the chunk starts.
    pub text_offset_start: usize,
    /// Byte offset one past the chunk's end.
    pub text_offset_end: usize,
}

impl IndexedChunk {
    /// Deserialize an indexed chunk from its stored JSON fields.
    pub fn from_document(document: Map<String, Value>) -> Result<Self> {
        serde_json::from_value(Value::Object(document))
            .context("indexed chunk does not match the search schema")
    }
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
                "text_offset_start".to_string(),
                Value::Number(chunk.text_offset_start.into()),
            );
            m.insert(
                "text_offset_end".to_string(),
                Value::Number(chunk.text_offset_end.into()),
            );
            m
        })
        .collect();

    index_documents(index, indexer_config, json_docs)
}

#[allow(clippy::result_large_err)]
fn index_documents(
    index: &Index,
    indexer_config: &IndexerConfig,
    json_docs: Vec<Map<String, Value>>,
) -> Result<()> {
    let mmap = mmap_from_objects(json_docs);

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let mut operation = IndexOperations::new();
    operation.replace_documents(&mmap, MissingDocumentPolicy::Create)?;

    let indexer_alloc = Bump::new();
    let must_stop = MustStopProcessing::default();
    let (document_changes, operation_stats, primary_key) = operation.into_changes(
        &indexer_alloc,
        index,
        &rtxn,
        None,
        &mut new_fields_ids_map,
        &must_stop,
        Progress::quiet(),
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
                &must_stop,
                &Progress::quiet(),
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

/// Parameters for searching the index.
pub struct SearchParams<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub offset: usize,
    /// The collection whose chunks are searched.
    pub collection_id: &'a str,
}

/// Execute a keyword (BM25) search against the milli index.
pub fn search_index(index: &Index, params: SearchParams<'_>) -> Result<SearchResults> {
    let SearchParams {
        query,
        limit,
        offset,
        collection_id,
    } = params;

    let rtxn = index.read_txn()?;
    let fields_ids_map = index.fields_ids_map(&rtxn)?;
    let progress = Progress::quiet();
    let mut search = milli::Search::new(
        &rtxn,
        index,
        &fields_ids_map,
        "insight",
        OffsetDateTime::now_utc(),
        &progress,
    );
    search.query(query);
    search.limit(limit);
    search.offset(offset);
    search.scoring_strategy(ScoringStrategy::Detailed);
    search.exhaustive_number_hits(true);
    search.terms_matching_strategy(TermsMatchingStrategy::Last);

    search.filter(Some(collection_filter(collection_id)));

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

/// Get a typed indexed chunk using an existing transaction.
pub fn get_indexed_chunk(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    doc_id: u32,
) -> Result<Option<IndexedChunk>> {
    get_document(index, rtxn, doc_id)?
        .map(IndexedChunk::from_document)
        .transpose()
}

/// Get a typed indexed chunk by its external ID.
pub fn get_indexed_chunk_by_external_id(
    index: &Index,
    external_id: &str,
) -> Result<Option<IndexedChunk>> {
    let rtxn = index.read_txn()?;

    // O(1) lookup: external ID -> internal ID via B-tree
    let external_ids = index.external_documents_ids();
    match external_ids.get(&rtxn, external_id)? {
        Some(internal_id) => get_indexed_chunk(index, &rtxn, internal_id),
        None => Ok(None),
    }
}

/// Distinct parent document IDs that currently have indexed chunks in exactly
/// `collection_id`.
///
/// Document IDs repeat across collections, so the collection scope is part of
/// the lookup.
pub fn indexed_parent_ids(index: &Index, collection_id: &str) -> Result<HashSet<String>> {
    let rtxn = index.read_txn()?;
    let progress = Progress::quiet();
    let fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut search = milli::Search::new(
        &rtxn,
        index,
        &fields_ids_map,
        "insight",
        OffsetDateTime::now_utc(),
        &progress,
    );
    search.query("");
    search.limit(usize::MAX);
    search.filter(Some(collection_filter(collection_id)));

    let result = search.execute()?;
    let mut parents = HashSet::new();
    for doc_id in result.documents_ids.iter() {
        let document =
            get_document(index, &rtxn, *doc_id)?.context("indexed chunk has no stored document")?;
        let parent_id = document
            .get("parent_id")
            .and_then(Value::as_str)
            .context("indexed chunk has no parent_id")?;
        parents.insert(parent_id.to_string());
    }

    Ok(parents)
}

/// Delete all chunks belonging to a document in one collection.
pub fn delete_document_chunks(
    index: &Index,
    indexer_config: &IndexerConfig,
    collection_id: &str,
    parent_id: &str,
) -> Result<usize> {
    // Find all chunks with this collection and parent
    let rtxn = index.read_txn()?;

    let progress = Progress::quiet();
    let fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut search = milli::Search::new(
        &rtxn,
        index,
        &fields_ids_map,
        "insight",
        OffsetDateTime::now_utc(),
        &progress,
    );
    search.query("");
    search.limit(usize::MAX);
    search.filter(Some(IndexFilter::from(IndexFilterCondition::And(vec![
        indexed_field_equals("collection_id", collection_id),
        indexed_field_equals("parent_id", parent_id),
    ]))));

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
        tracing::debug!(
            "Deleted {} chunks for document {} in collection {}",
            count,
            parent_id,
            collection_id
        );
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
/// Terms are sorted by frequency (descending) and limited to `limit` results.
///
/// This is useful for:
/// - Giving the LLM context about what topics the collection covers
/// - Suggesting auto-tags for documents
/// - Showing users what they can search for
pub fn get_collection_terms(
    index: &Index,
    collection_id: &str,
    limit: usize,
) -> Result<Vec<TermFrequency>> {
    let rtxn = index.read_txn()?;

    let fields_ids_map = index.fields_ids_map(&rtxn)?;
    let target_docs = collection_filter(collection_id).evaluate(&rtxn, index, &fields_ids_map)?;

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
            let doc_count = word_bitmap.intersection_len(&target_docs);

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
    operation.delete_documents_by_external_ids(&chunk_ids_refs);

    let indexer_alloc = Bump::new();
    let must_stop = MustStopProcessing::default();
    let (document_changes, operation_stats, primary_key) = operation.into_changes(
        &indexer_alloc,
        index,
        &rtxn,
        None,
        &mut new_fields_ids_map,
        &must_stop,
        Progress::quiet(),
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
                &must_stop,
                &Progress::quiet(),
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
    let results = search_index(
        index,
        SearchParams {
            query: "",
            limit: usize::MAX,
            offset: 0,
            collection_id,
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
            id: chunk_external_id(collection_id, parent_id, 0),
            parent_id: parent_id.to_string(),
            parent_name: parent_name.to_string(),
            chunk_index: 0,
            content: content.to_string(),
            collection_id: collection_id.to_string(),
            text_offset_start: 0,
            text_offset_end: content.len(),
        }
    }

    /// Helper to get a string field from a document (for testing)
    fn get_field(index: &Index, doc_id: u32, field: &str) -> Option<String> {
        let rtxn = index.read_txn().ok()?;
        let doc = get_document(index, &rtxn, doc_id).ok()??;
        doc.get(field).and_then(|v| v.as_str()).map(String::from)
    }

    #[test]
    fn test_index_reopens() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = test_indexer_config();

        {
            let index = open_index(temp_dir.path()).unwrap();
            let chunk = make_chunk(
                "doc1",
                "test.pdf",
                "This is a test document about climate change.",
                "collection1",
            );
            index_chunks_batch(&index, &config, vec![chunk]).unwrap();
        }

        let index = open_index(temp_dir.path()).unwrap();
        let results = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                offset: 0,
                collection_id: "collection1",
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
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
                offset: 0,
                collection_id: "collection1",
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

        let chunk = make_chunk("doc1", "test.pdf", "Hello world", "col1");
        index_chunks_batch(&index, &config, vec![chunk]).unwrap();

        let results = search_index(
            &index,
            SearchParams {
                query: "nonexistent",
                limit: 10,
                offset: 0,
                collection_id: "col1",
            },
        )
        .unwrap();
        assert!(results.hits.is_empty());
    }

    #[test]
    fn test_search_is_scoped_to_one_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Climate research paper", "climate"),
            make_chunk("doc2", "b.pdf", "Climate news article", "news"),
            make_chunk("doc3", "c.pdf", "Climate policy document", "policy"),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let climate_only = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                offset: 0,
                collection_id: "climate",
            },
        )
        .unwrap();
        assert_eq!(climate_only.hits.len(), 1);
        let name = get_field(&index, climate_only.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("a.pdf".to_string()));

        let news_only = search_index(
            &index,
            SearchParams {
                query: "climate",
                limit: 10,
                offset: 0,
                collection_id: "news",
            },
        )
        .unwrap();
        assert_eq!(news_only.hits.len(), 1);
        let name = get_field(&index, news_only.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("b.pdf".to_string()));
    }

    #[test]
    fn test_delete_document_chunks() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index chunks for two documents
        let chunks = vec![
            make_chunk("doc1", "a.pdf", "First document", "col1"),
            make_chunk("doc2", "b.pdf", "Second document", "col1"),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        // Delete chunks for one document
        let deleted = delete_document_chunks(&index, &config, "col1", "doc1").unwrap();
        assert_eq!(deleted, 1);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        // Search should only find the remaining document's chunks
        let results = search_index(
            &index,
            SearchParams {
                query: "document",
                limit: 10,
                offset: 0,
                collection_id: "col1",
            },
        )
        .unwrap();
        assert_eq!(results.hits.len(), 1);
        let name = get_field(&index, results.hits[0].doc_id, "parent_name");
        assert_eq!(name, Some("b.pdf".to_string()));
    }

    #[test]
    fn test_same_document_id_indexes_in_two_collections() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Alpha findings", "col1"),
            make_chunk("doc1", "a.pdf", "Beta findings", "col2"),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        let deleted = delete_document_chunks(&index, &config, "col1", "doc1").unwrap();
        assert_eq!(deleted, 1);

        assert!(search_index(
            &index,
            SearchParams {
                query: "alpha",
                limit: 10,
                offset: 0,
                collection_id: "col1",
            },
        )
        .unwrap()
        .hits
        .is_empty());

        let beta = search_index(
            &index,
            SearchParams {
                query: "beta",
                limit: 10,
                offset: 0,
                collection_id: "col2",
            },
        )
        .unwrap();
        assert_eq!(beta.hits.len(), 1);
        assert_eq!(
            get_field(&index, beta.hits[0].doc_id, "collection_id"),
            Some("col2".to_string())
        );
    }

    #[test]
    fn test_replacing_document_in_one_collection_leaves_the_other_intact() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Alpha findings", "col1"),
            make_chunk("doc1", "a.pdf", "Beta findings", "col2"),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        index_chunks_batch(
            &index,
            &config,
            vec![make_chunk("doc1", "a.pdf", "Gamma findings", "col1")],
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        assert!(search_index(
            &index,
            SearchParams {
                query: "alpha",
                limit: 10,
                offset: 0,
                collection_id: "col1",
            },
        )
        .unwrap()
        .hits
        .is_empty());

        let gamma = search_index(
            &index,
            SearchParams {
                query: "gamma",
                limit: 10,
                offset: 0,
                collection_id: "col1",
            },
        )
        .unwrap();
        assert_eq!(gamma.hits.len(), 1);
        assert_eq!(
            get_field(&index, gamma.hits[0].doc_id, "collection_id"),
            Some("col1".to_string())
        );

        let beta = search_index(
            &index,
            SearchParams {
                query: "beta",
                limit: 10,
                offset: 0,
                collection_id: "col2",
            },
        )
        .unwrap();
        assert_eq!(beta.hits.len(), 1);
        assert_eq!(
            get_field(&index, beta.hits[0].doc_id, "collection_id"),
            Some("col2".to_string())
        );
    }

    #[test]
    fn test_delete_chunks_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index chunks in different collections
        let chunks = vec![
            make_chunk("doc1", "a.pdf", "Document alpha", "col1"),
            make_chunk("doc2", "b.pdf", "Document beta", "col1"),
            make_chunk("doc3", "c.pdf", "Document gamma", "col2"),
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
                offset: 0,
                collection_id: "col2",
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

        let terms = get_collection_terms(&index, "col1", 50).unwrap();
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
                "research",
            ),
        ];
        index_chunks_batch(&index, &config, chunks).unwrap();

        let terms = get_collection_terms(&index, "research", 10).unwrap();

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
    fn test_get_collection_terms_only_covers_the_collection() {
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

        let terms = get_collection_terms(&index, "research", 10).unwrap();

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
        let terms = get_collection_terms(&index, "col1", 3).unwrap();
        assert!(terms.len() <= 3, "Should return at most 3 terms");
    }
}
