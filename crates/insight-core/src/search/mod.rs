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
use milli::tokenizer::{SeparatorKind, TokenKind};
use milli::update::new::indexer::{self, IndexOperations};
use milli::update::new::tokenizer_builder;
use milli::update::{IndexerConfig, MissingDocumentPolicy};
use milli::vector::RuntimeEmbedders;
use milli::{
    Condition, CreateOrOpen, FilterableAttributesRule, Index, IndexFilter, IndexFilterCondition,
    MatcherBuilder, MustStopProcessing, TermsMatchingStrategy, Token,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use time::OffsetDateTime;

/// Fields indexed for keyword search, in relevance order.
const SEARCHABLE_FIELDS: [&str; 2] = ["document_name", "content"];

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

    let needs_setup = {
        let rtxn = index.read_txn()?;
        let searchable = index.user_defined_searchable_fields(&rtxn)?;
        let filterable = index.filterable_attributes_rules(&rtxn)?;
        let has_searchable = searchable.as_deref() == Some(SEARCHABLE_FIELDS.as_slice());
        let has_collection_filter = filterable.iter().any(|rule| {
            matches!(rule, FilterableAttributesRule::Field(field) if field == "collection_id")
        });
        let has_document_filter = filterable.iter().any(
            |rule| matches!(rule, FilterableAttributesRule::Field(field) if field == "document_id"),
        );
        !has_searchable || !has_collection_filter || !has_document_filter
    };

    if needs_setup {
        let indexer_config = IndexerConfig::default();
        let mut wtxn = index.write_txn()?;
        let mut settings = milli::update::Settings::new(&mut wtxn, &index, &indexer_config);
        settings.set_primary_key("id".to_string());
        settings.set_searchable_fields(
            SEARCHABLE_FIELDS
                .iter()
                .map(|field| field.to_string())
                .collect(),
        );
        settings.set_filterable_fields(vec![
            FilterableAttributesRule::Field("collection_id".to_string()),
            FilterableAttributesRule::Field("document_id".to_string()),
        ]);
        settings.execute(
            &MustStopProcessing::default(),
            &Progress::quiet(),
            &IpPolicy::danger_always_allow(),
            Arc::new(EmbedderStats::default()),
        )?;
        wtxn.commit()?;
        tracing::info!("Configured searchable and filterable attributes");
    }

    tracing::info!("Search index opened at {:?}", path);

    Ok(index)
}

// ============================================================================
// Indexing
// ============================================================================

/// External ID for an indexed document, unique within the search index.
///
/// The same document ID can occur in multiple collections, so the collection
/// is part of the identity.
pub fn document_external_id(collection_id: &str, document_id: &str) -> String {
    format!("{collection_id}_{document_id}")
}

/// One whole document to be indexed.
pub struct DocumentToIndex {
    /// Unique index ID, see [`document_external_id`].
    pub id: String,
    /// Original document ID.
    pub document_id: String,
    /// Document filename, indexed as its own searchable field.
    pub document_name: String,
    /// Parser key of the extraction the content came from.
    pub extraction: String,
    /// Complete canonical extraction text.
    pub content: String,
    /// Collection this document belongs to.
    pub collection_id: String,
}

/// A document as stored in the search index.
///
/// This mirrors the fields written by [`DocumentToIndex`], plus the derived
/// [`partially_indexed`](Self::partially_indexed) status, so a malformed or
/// stale row fails deserialization instead of yielding default values.
#[derive(Debug, Deserialize)]
pub struct IndexedDocument {
    /// Unique index ID, see [`document_external_id`].
    pub id: String,
    /// Original document ID.
    pub document_id: String,
    /// Document filename.
    pub document_name: String,
    /// Parser key of the extraction the content came from.
    pub extraction: String,
    /// Complete canonical extraction text.
    pub content: String,
    /// Collection this document belongs to.
    pub collection_id: String,
    /// Whether part of the content exceeded milli's per-field token-position
    /// limit and is therefore not searchable.
    pub partially_indexed: bool,
}

impl IndexedDocument {
    /// Deserialize an indexed document from its stored JSON fields.
    pub fn from_document(document: Map<String, Value>) -> Result<Self> {
        serde_json::from_value(Value::Object(document))
            .context("indexed document does not match the search schema")
    }
}

/// Maximum documents per indexing batch. Whole-document content is larger
/// than the passage-sized records it replaced, so batches stay small.
const BATCH_DOCUMENT_SIZE: usize = 20;

/// Index multiple whole documents in batches.
pub fn index_documents_batch(
    index: &Index,
    indexer_config: &IndexerConfig,
    documents: Vec<DocumentToIndex>,
) -> Result<()> {
    if documents.is_empty() {
        return Ok(());
    }

    let total = documents.len();
    let num_batches = total.div_ceil(BATCH_DOCUMENT_SIZE);
    for (batch_idx, batch) in documents.chunks(BATCH_DOCUMENT_SIZE).enumerate() {
        index_document_batch(index, indexer_config, batch)?;
        tracing::info!(
            "Indexed batch {}/{} ({} documents, {}% complete)",
            batch_idx + 1,
            num_batches,
            batch.len(),
            ((batch_idx + 1) * 100) / num_batches
        );
    }

    tracing::debug!("Indexed batch of {} documents total", total);

    Ok(())
}

/// Index a batch of whole documents.
fn index_document_batch(
    index: &Index,
    indexer_config: &IndexerConfig,
    documents: &[DocumentToIndex],
) -> Result<()> {
    let partially_indexed = {
        let rtxn = index.read_txn()?;
        documents
            .iter()
            .map(|document| exceeds_position_limit(index, &rtxn, &document.content))
            .collect::<Result<Vec<_>>>()?
    };

    let json_docs: Vec<Map<String, Value>> = documents
        .iter()
        .zip(partially_indexed)
        .map(|(document, partially_indexed)| {
            let mut m = Map::new();
            m.insert("id".to_string(), Value::String(document.id.clone()));
            m.insert(
                "document_id".to_string(),
                Value::String(document.document_id.clone()),
            );
            m.insert(
                "document_name".to_string(),
                Value::String(document.document_name.clone()),
            );
            m.insert(
                "extraction".to_string(),
                Value::String(document.extraction.clone()),
            );
            m.insert(
                "content".to_string(),
                Value::String(document.content.clone()),
            );
            m.insert(
                "collection_id".to_string(),
                Value::String(document.collection_id.clone()),
            );
            m.insert(
                "partially_indexed".to_string(),
                Value::Bool(partially_indexed),
            );
            m
        })
        .collect();

    index_documents(index, indexer_config, json_docs)
}

/// Whether any token in `text` would land beyond milli's per-field position
/// limit, leaving the rest of the text out of the search index.
///
/// milli drops indexable words whose token position reaches
/// [`MAX_POSITION_PER_ATTRIBUTE`](milli::MAX_POSITION_PER_ATTRIBUTE). The
/// position progression below mirrors milli's document tokenizer for one
/// string field: hard separators advance by eight positions, other token
/// transitions by one, and the first word starts at zero.
fn exceeds_position_limit(index: &Index, rtxn: &milli::heed::RoTxn, text: &str) -> Result<bool> {
    let stop_words = index.stop_words(rtxn)?;
    let separators = index.allowed_separators(rtxn)?;
    let separators: Option<Vec<_>> = separators
        .as_ref()
        .map(|separators| separators.iter().map(String::as_str).collect());
    let dictionary = index.dictionary(rtxn)?;
    let dictionary: Option<Vec<_>> = dictionary
        .as_ref()
        .map(|dictionary| dictionary.iter().map(String::as_str).collect());

    let mut builder = tokenizer_builder(
        stop_words.as_ref(),
        separators.as_deref(),
        dictionary.as_deref(),
    );
    let tokenizer = builder.build();

    Ok(token_positions_overflow(tokenizer.tokenize(text)))
}

/// Whether an indexable word at or beyond the position limit appears in the
/// token stream. Positions follow milli's `process_tokens` ordering.
fn token_positions_overflow<'a>(tokens: impl Iterator<Item = milli::tokenizer::Token<'a>>) -> bool {
    let mut position = 0_u32;
    let mut previous_kind = None;

    for token in tokens.skip_while(|token| token.is_separator()) {
        match token.kind {
            TokenKind::Word | TokenKind::StopWord if !token.lemma().is_empty() => {
                position += match previous_kind {
                    Some(TokenKind::Separator(SeparatorKind::Hard)) => 8,
                    Some(_) => 1,
                    None => 0,
                };
                previous_kind = Some(token.kind);
            }
            TokenKind::Separator(SeparatorKind::Hard) => {
                previous_kind = Some(token.kind);
            }
            TokenKind::Separator(SeparatorKind::Soft)
                if previous_kind != Some(TokenKind::Separator(SeparatorKind::Hard)) =>
            {
                previous_kind = Some(token.kind);
            }
            _ => {}
        }

        if token.is_word() {
            let lemma = token.lemma().trim();
            if !lemma.is_empty()
                && lemma.len() <= milli::MAX_WORD_LENGTH
                && position >= milli::MAX_POSITION_PER_ATTRIBUTE
            {
                return true;
            }
        }
    }

    false
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

// ============================================================================
// Search
// ============================================================================

/// Search result with document ID and score details.
pub struct SearchHit {
    /// Internal milli document ID.
    pub doc_id: u32,
    /// Relevance score details.
    pub scores: Vec<milli::score_details::ScoreDetails>,
    /// Original document ID.
    pub document_id: String,
    /// Document filename.
    pub document_name: String,
    /// Parser key of the extraction the content came from.
    pub extraction: String,
    /// Whether part of the content is outside the search index's coverage.
    pub partially_indexed: bool,
    /// An excerpt around the matching content, when the content matched.
    pub excerpt: Option<SearchExcerpt>,
}

/// A bounded passage of an indexed document containing a query match.
pub struct SearchExcerpt {
    /// Excerpt text.
    pub content: String,
    /// Zero-based character offset of the excerpt in the extraction.
    pub offset: usize,
    /// Character offset one past the excerpt.
    pub next_offset: usize,
    /// Total character length of the extraction.
    pub total_length: usize,
}

/// Search results with pagination info.
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    pub total_hits: usize,
}

/// Character length of one search excerpt.
const EXCERPT_LIMIT: usize = 500;
/// Characters of context kept before the matching text when possible.
const EXCERPT_CONTEXT_BEFORE: usize = 100;

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
    /// The collection whose documents are searched.
    pub collection_id: &'a str,
}

/// Execute a keyword search against the milli index.
///
/// Returns one ranked entry per whole document, with a bounded excerpt around
/// the first content match when the content (not only the name) matched.
pub fn search_index(index: &Index, params: SearchParams<'_>) -> Result<SearchResults> {
    let SearchParams {
        query,
        limit,
        offset,
        collection_id,
    } = params;

    let rtxn = index.read_txn()?;
    let result = execute_search(index, &rtxn, query, limit, offset, collection_id)?;
    let total_hits = result.candidates.len() as usize;

    let stop_words = index.stop_words(&rtxn)?;
    let separators = index.allowed_separators(&rtxn)?;
    let separators: Option<Vec<_>> = separators
        .as_ref()
        .map(|separators| separators.iter().map(String::as_str).collect());
    let dictionary = index.dictionary(&rtxn)?;
    let dictionary: Option<Vec<_>> = dictionary
        .as_ref()
        .map(|dictionary| dictionary.iter().map(String::as_str).collect());
    let mut tokenizer_builder = tokenizer_builder(
        stop_words.as_ref(),
        separators.as_deref(),
        dictionary.as_deref(),
    );
    let tokenizer = tokenizer_builder.build();
    let matcher_builder = MatcherBuilder::new(result.matching_words, tokenizer);

    let mut hits = Vec::with_capacity(result.documents_ids.len());
    for (doc_id, scores) in result.documents_ids.into_iter().zip(result.document_scores) {
        let document = get_indexed_document(index, &rtxn, doc_id)?
            .with_context(|| format!("search hit references missing indexed document {doc_id}"))?;
        let excerpt = excerpt_around_match(&matcher_builder, &document.content);

        hits.push(SearchHit {
            doc_id,
            scores,
            document_id: document.document_id,
            document_name: document.document_name,
            extraction: document.extraction,
            partially_indexed: document.partially_indexed,
            excerpt,
        });
    }

    Ok(SearchResults { hits, total_hits })
}

/// Execute a milli search, returning its raw result set.
fn execute_search(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    query: &str,
    limit: usize,
    offset: usize,
    collection_id: &str,
) -> Result<milli::SearchResult> {
    let fields_ids_map = index.fields_ids_map(rtxn)?;
    let progress = Progress::quiet();
    let mut search = milli::Search::new(
        rtxn,
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

    Ok(search.execute()?)
}

/// A bounded excerpt around the first content match, or `None` when the
/// document matched only by its name.
fn excerpt_around_match(builder: &MatcherBuilder<'_>, text: &str) -> Option<SearchExcerpt> {
    let mut matcher = builder.build(text, None);
    let bounds = matcher.matches(&[]);
    let first = bounds.first()?;

    let total_length = text.chars().count();
    let match_start = text[..first.start].chars().count();
    let mut start = match_start.saturating_sub(EXCERPT_CONTEXT_BEFORE);
    let mut end = (start + EXCERPT_LIMIT).min(total_length);
    if end - start < EXCERPT_LIMIT && total_length >= EXCERPT_LIMIT {
        start = end.saturating_sub(EXCERPT_LIMIT);
    }
    end = end.max(start);

    let content: String = text.chars().skip(start).take(end - start).collect();
    Some(SearchExcerpt {
        content,
        offset: start,
        next_offset: end,
        total_length,
    })
}

/// Get a document as a JSON object using an existing transaction
pub fn get_document(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    doc_id: u32,
) -> Result<Option<Map<String, Value>>> {
    let fields_ids_map = index.fields_ids_map(rtxn)?;

    let docs = index.documents(rtxn, [doc_id])?;
    if let Some((_id, obkv)) = docs.first() {
        let obj = milli::all_obkv_to_json(obkv, &fields_ids_map)?;
        Ok(Some(obj))
    } else {
        Ok(None)
    }
}

/// Get a typed indexed document using an existing transaction.
pub fn get_indexed_document(
    index: &Index,
    rtxn: &milli::heed::RoTxn,
    doc_id: u32,
) -> Result<Option<IndexedDocument>> {
    get_document(index, rtxn, doc_id)?
        .map(IndexedDocument::from_document)
        .transpose()
}

/// Get a typed indexed document by its external ID.
pub fn get_indexed_document_by_external_id(
    index: &Index,
    external_id: &str,
) -> Result<Option<IndexedDocument>> {
    let rtxn = index.read_txn()?;

    // O(1) lookup: external ID -> internal ID via B-tree
    let external_ids = index.external_documents_ids();
    match external_ids.get(&rtxn, external_id)? {
        Some(internal_id) => get_indexed_document(index, &rtxn, internal_id),
        None => Ok(None),
    }
}

/// Whether one document's search coverage is partial, when it is indexed.
///
/// Reads only the derived status field, not the document's content.
pub fn is_partially_indexed(
    index: &Index,
    collection_id: &str,
    document_id: &str,
) -> Result<Option<bool>> {
    let external_id = document_external_id(collection_id, document_id);
    let rtxn = index.read_txn()?;
    let external_ids = index.external_documents_ids();
    let Some(doc_id) = external_ids.get(&rtxn, &external_id)? else {
        return Ok(None);
    };

    let fields_ids_map = index.fields_ids_map(&rtxn)?;
    let field_id = fields_ids_map
        .id("partially_indexed")
        .context("indexed documents have no partially_indexed field")?;
    let documents = index.documents(&rtxn, [doc_id])?;
    let Some((_id, obkv)) = documents.first() else {
        return Ok(None);
    };
    let value = obkv
        .get(field_id)
        .context("indexed document has no partially_indexed value")?;
    let partially_indexed = serde_json::from_slice(value)
        .context("indexed document has an invalid partially_indexed value")?;
    Ok(Some(partially_indexed))
}

/// Internal IDs of every indexed document in exactly `collection_id`.
fn indexed_document_ids_in_collection(index: &Index, collection_id: &str) -> Result<Vec<u32>> {
    let rtxn = index.read_txn()?;
    let result = execute_search(index, &rtxn, "", usize::MAX, 0, collection_id)?;
    Ok(result.documents_ids)
}

/// Source document IDs that are currently indexed in exactly
/// `collection_id`.
///
/// Document IDs repeat across collections, so the collection scope is part of
/// the lookup.
pub fn indexed_document_ids(index: &Index, collection_id: &str) -> Result<HashSet<String>> {
    let rtxn = index.read_txn()?;
    let mut documents = HashSet::new();
    for doc_id in indexed_document_ids_in_collection(index, collection_id)? {
        let document = get_indexed_document(index, &rtxn, doc_id)?
            .context("indexed document row is missing")?;
        documents.insert(document.document_id);
    }
    Ok(documents)
}

/// Delete one document's index record in one collection.
pub fn delete_document(
    index: &Index,
    indexer_config: &IndexerConfig,
    collection_id: &str,
    document_id: &str,
) -> Result<bool> {
    let external_id = document_external_id(collection_id, document_id);
    delete_documents_by_id(index, indexer_config, &[external_id])
}

/// Delete every document index record in one collection.
pub fn delete_documents_by_collection(
    index: &Index,
    indexer_config: &IndexerConfig,
    collection_id: &str,
) -> Result<usize> {
    let ids = indexed_document_ids_in_collection(index, collection_id)?;
    if ids.is_empty() {
        return Ok(0);
    }

    let rtxn = index.read_txn()?;
    let external_ids: Vec<String> = ids
        .iter()
        .map(|&doc_id| {
            get_document(index, &rtxn, doc_id).and_then(|document| {
                document
                    .context("indexed document row is missing")?
                    .get("id")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .context("indexed document has no id")
            })
        })
        .collect::<Result<_>>()?;
    drop(rtxn);

    let count = external_ids.len();
    delete_documents_by_id(index, indexer_config, &external_ids)?;
    tracing::info!(
        "Deleted {} documents from index for collection {}",
        count,
        collection_id
    );

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

/// Delete documents by their external IDs.
#[allow(clippy::result_large_err)]
fn delete_documents_by_id(
    index: &Index,
    indexer_config: &IndexerConfig,
    external_ids: &[String],
) -> Result<bool> {
    if external_ids.is_empty() {
        return Ok(false);
    }

    let document_ids_refs: Vec<&str> = external_ids.iter().map(|s| s.as_str()).collect();

    let rtxn = index.read_txn()?;
    let db_fields_ids_map = index.fields_ids_map(&rtxn)?;
    let mut new_fields_ids_map = db_fields_ids_map.clone();

    let mut operation = IndexOperations::new();
    operation.delete_documents_by_external_ids(&document_ids_refs);

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
                &must_stop,
                &Progress::quiet(),
                &IpPolicy::danger_always_allow(),
                &EmbedderStats::default(),
            )
        })
        .map_err(|e| anyhow::anyhow!("Thread pool error: {}", e))??;

    wtxn.commit()?;

    tracing::debug!("Deleted {} documents from index", external_ids.len());

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_indexer_config() -> IndexerConfig {
        IndexerConfig::default()
    }

    /// Helper to create a whole-document record for testing.
    fn make_document(
        document_id: &str,
        document_name: &str,
        content: &str,
        collection_id: &str,
    ) -> DocumentToIndex {
        DocumentToIndex {
            id: document_external_id(collection_id, document_id),
            document_id: document_id.to_string(),
            document_name: document_name.to_string(),
            extraction: "test@1".to_string(),
            content: content.to_string(),
            collection_id: collection_id.to_string(),
        }
    }

    /// Helper to get a string field from a document (for testing)
    fn get_field(index: &Index, doc_id: u32, field: &str) -> Option<String> {
        let rtxn = index.read_txn().ok()?;
        let doc = get_document(index, &rtxn, doc_id).ok()??;
        doc.get(field).and_then(|v| v.as_str()).map(String::from)
    }

    fn search(index: &Index, collection_id: &str, query: &str) -> SearchResults {
        search_index(
            index,
            SearchParams {
                query,
                limit: 10,
                offset: 0,
                collection_id,
            },
        )
        .unwrap()
    }

    #[test]
    fn test_index_reopens() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = test_indexer_config();

        {
            let index = open_index(temp_dir.path()).unwrap();
            let document = make_document(
                "doc1",
                "test.pdf",
                "This is a test document about climate change.",
                "collection1",
            );
            index_documents_batch(&index, &config, vec![document]).unwrap();
        }

        let index = open_index(temp_dir.path()).unwrap();
        let results = search(&index, "collection1", "climate");
        assert_eq!(results.hits.len(), 1);
    }

    #[test]
    fn test_index_and_search_document() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let document = make_document(
            "doc1",
            "test.pdf",
            "This is a test document about climate change.",
            "collection1",
        );
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);

        let results = search(&index, "collection1", "climate");
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].document_name, "test.pdf");
        assert_eq!(results.hits[0].document_id, "doc1");
        assert!(!results.hits[0].partially_indexed);
    }

    #[test]
    fn test_document_name_is_a_separate_searchable_field() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let document = make_document(
            "doc1",
            "Climate_Report_2024.pdf",
            "Annual figures and tables.",
            "col1",
        );
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let results = search(&index, "col1", "Climate_Report");
        assert_eq!(results.hits.len(), 1);
        let hit = &results.hits[0];
        assert_eq!(hit.document_name, "Climate_Report_2024.pdf");
        assert!(
            hit.excerpt.is_none(),
            "a name-only match must not fake a content excerpt"
        );

        let rtxn = index.read_txn().unwrap();
        let stored = get_indexed_document(&index, &rtxn, hit.doc_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, "Annual figures and tables.");
    }

    #[test]
    fn test_search_no_results() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let document = make_document("doc1", "test.pdf", "Hello world", "col1");
        index_documents_batch(&index, &config, vec![document]).unwrap();

        assert!(search(&index, "col1", "nonexistent").hits.is_empty());
    }

    #[test]
    fn test_search_is_scoped_to_one_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let documents = vec![
            make_document("doc1", "a.pdf", "Climate research paper", "climate"),
            make_document("doc2", "b.pdf", "Climate news article", "news"),
            make_document("doc3", "c.pdf", "Climate policy document", "policy"),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let climate_only = search(&index, "climate", "climate");
        assert_eq!(climate_only.hits.len(), 1);
        assert_eq!(climate_only.hits[0].document_name, "a.pdf");

        let news_only = search(&index, "news", "climate");
        assert_eq!(news_only.hits.len(), 1);
        assert_eq!(news_only.hits[0].document_name, "b.pdf");
    }

    #[test]
    fn test_search_excerpt_is_bounded_and_aligned_with_match() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let mut content = "filler words before the match ".repeat(40);
        content.push_str("unique needle appears here. ");
        content.push_str(&"filler words after the match ".repeat(40));
        let document = make_document("doc1", "report.pdf", &content, "col1");
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let results = search(&index, "col1", "needle");
        assert_eq!(results.hits.len(), 1);
        let excerpt = results.hits[0]
            .excerpt
            .as_ref()
            .expect("content match has an excerpt");

        assert_eq!(excerpt.total_length, content.chars().count());
        assert!(excerpt.content.contains("needle"));
        assert!(excerpt.content.chars().count() <= EXCERPT_LIMIT);
        assert!(
            excerpt.offset > 0,
            "excerpt carries context before the match"
        );
        assert_eq!(
            excerpt.content.chars().count(),
            excerpt.next_offset - excerpt.offset
        );

        let reconstructed: String = content
            .chars()
            .skip(excerpt.offset)
            .take(excerpt.next_offset - excerpt.offset)
            .collect();
        assert_eq!(excerpt.content, reconstructed);
    }

    #[test]
    fn test_partially_indexed_document_keeps_full_content() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let mut content = "word ".repeat(66_000);
        content.push_str("tailmarker");
        let document = make_document("doc1", "long.pdf", &content, "col1");
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let results = search(&index, "col1", "word");
        assert_eq!(results.hits.len(), 1, "early content stays searchable");
        let hit = &results.hits[0];
        assert!(hit.partially_indexed);
        assert_eq!(
            is_partially_indexed(&index, "col1", "doc1").unwrap(),
            Some(true)
        );

        let rtxn = index.read_txn().unwrap();
        let stored = get_indexed_document(&index, &rtxn, hit.doc_id)
            .unwrap()
            .unwrap();
        assert_eq!(stored.content, content);
        assert!(search(&index, "col1", "tailmarker").hits.is_empty());
    }

    #[test]
    fn test_short_document_is_fully_indexed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let document = make_document("doc1", "short.pdf", &"word ".repeat(1_000), "col1");
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let results = search(&index, "col1", "word");
        assert_eq!(results.hits.len(), 1);
        assert!(!results.hits[0].partially_indexed);
        assert_eq!(
            is_partially_indexed(&index, "col1", "doc1").unwrap(),
            Some(false)
        );
        assert_eq!(
            is_partially_indexed(&index, "col1", "missing").unwrap(),
            None
        );
    }

    #[test]
    fn test_delete_document_record() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let documents = vec![
            make_document("doc1", "a.pdf", "First document", "col1"),
            make_document("doc2", "b.pdf", "Second document", "col1"),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        assert!(delete_document(&index, &config, "col1", "doc1").unwrap());

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);
        drop(rtxn);

        let results = search(&index, "col1", "document");
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].document_name, "b.pdf");
    }

    #[test]
    fn test_same_document_id_indexes_in_two_collections() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let documents = vec![
            make_document("doc1", "a.pdf", "Alpha findings", "col1"),
            make_document("doc1", "a.pdf", "Beta findings", "col2"),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        delete_document(&index, &config, "col1", "doc1").unwrap();

        assert!(search(&index, "col1", "alpha").hits.is_empty());

        let beta = search(&index, "col2", "beta");
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

        let documents = vec![
            make_document("doc1", "a.pdf", "Alpha findings", "col1"),
            make_document("doc1", "a.pdf", "Beta findings", "col2"),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        index_documents_batch(
            &index,
            &config,
            vec![make_document("doc1", "a.pdf", "Gamma findings", "col1")],
        )
        .unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 2);
        drop(rtxn);

        assert!(search(&index, "col1", "alpha").hits.is_empty());

        let gamma = search(&index, "col1", "gamma");
        assert_eq!(gamma.hits.len(), 1);
        assert_eq!(
            get_field(&index, gamma.hits[0].doc_id, "collection_id"),
            Some("col1".to_string())
        );

        let beta = search(&index, "col2", "beta");
        assert_eq!(beta.hits.len(), 1);
        assert_eq!(
            get_field(&index, beta.hits[0].doc_id, "collection_id"),
            Some("col2".to_string())
        );
    }

    #[test]
    fn test_delete_documents_by_collection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        let documents = vec![
            make_document("doc1", "a.pdf", "Document alpha", "col1"),
            make_document("doc2", "b.pdf", "Document beta", "col1"),
            make_document("doc3", "c.pdf", "Document gamma", "col2"),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 3);
        drop(rtxn);

        let deleted = delete_documents_by_collection(&index, &config, "col1").unwrap();
        assert_eq!(deleted, 2);

        let rtxn = index.read_txn().unwrap();
        assert_eq!(index.number_of_documents(&rtxn).unwrap(), 1);
        drop(rtxn);

        let results = search(&index, "col2", "document");
        assert_eq!(results.hits.len(), 1);
        assert_eq!(results.hits[0].document_name, "c.pdf");
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

        let documents = vec![
            make_document(
                "doc1",
                "climate.pdf",
                "Climate change affects global temperatures and weather patterns.",
                "research",
            ),
            make_document(
                "doc2",
                "weather.pdf",
                "Weather patterns are changing due to climate shifts.",
                "research",
            ),
            make_document(
                "doc3",
                "finance.pdf",
                "Financial markets respond to climate policy changes.",
                "research",
            ),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let terms = get_collection_terms(&index, "research", 10).unwrap();

        assert!(!terms.is_empty());
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

        let documents = vec![
            make_document(
                "doc1",
                "climate.pdf",
                "Climate research and environmental studies.",
                "research",
            ),
            make_document(
                "doc2",
                "finance.pdf",
                "Financial analysis and market trends.",
                "finance",
            ),
        ];
        index_documents_batch(&index, &config, documents).unwrap();

        let terms = get_collection_terms(&index, "research", 10).unwrap();

        assert!(!terms.is_empty());
        let has_climate = terms.iter().any(|t| t.term == "climate");
        assert!(has_climate, "Expected 'climate' in research terms");

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

        let document = make_document(
            "doc1",
            "varied.pdf",
            "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda",
            "col1",
        );
        index_documents_batch(&index, &config, vec![document]).unwrap();

        let terms = get_collection_terms(&index, "col1", 3).unwrap();
        assert!(terms.len() <= 3, "Should return at most 3 terms");
    }
}
