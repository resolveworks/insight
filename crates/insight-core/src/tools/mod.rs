//! Document tools for the Insight agent.
//!
//! Each tool implements rig's [`Tool`](rig::tool::Tool) trait with typed
//! arguments and model-facing JSON schemas.
//!
//! Each tool owns a clone of its per-run [`ToolState`]. Rig's tool context is
//! reserved for serializable values, while the search index and iroh storage
//! are live handles shared with the rest of the app.
//!
//! Outputs are typed serializable structs; errors are
//! [`ToolExecutionError`] with precise kinds (e.g. `not_found`) so the runtime
//! can classify outcomes and the model receives safe, actionable feedback.

use std::sync::Arc;

use rig::tool::{Tool, ToolContext, ToolExecutionError};
use rig_core::tool::args_schema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::search;
use crate::storage::Storage;
use crate::CollectionInfo;

/// Per-run state shared by the document tools.
#[derive(Clone)]
pub struct ToolState {
    /// milli search index (shared for reads; LMDB handles concurrency)
    pub search: Arc<milli::Index>,
    /// iroh storage, read-locked during tool calls
    pub storage: Arc<RwLock<Storage>>,
    /// The conversation's active collections. Never empty in practice —
    /// `send_message` rejects conversations without a scope before the agent
    /// is built.
    pub collections: Vec<CollectionInfo>,
}

impl ToolState {
    /// Build tool state from an [`crate::AppState`] and a collection scope.
    pub fn new(state: &crate::AppState, collections: Vec<CollectionInfo>) -> Self {
        Self {
            search: state.search.clone(),
            storage: state.storage.clone(),
            collections,
        }
    }

    /// IDs of the scoped collections, for milli collection filters.
    fn collection_ids(&self) -> Vec<String> {
        self.collections.iter().map(|c| c.id.clone()).collect()
    }

    /// Resolve a collection name from the active scope.
    fn name_of(&self, collection_id: &str) -> Result<String, ToolExecutionError> {
        self.collections
            .iter()
            .find(|c| c.id == collection_id)
            .map(|c| c.name.clone())
            .ok_or_else(|| {
                ToolExecutionError::not_found(format!(
                    "collection {collection_id} is not in the active scope"
                ))
            })
    }
}

/// Maximum number of passages returned by [`search`].
const SEARCH_LIMIT: usize = 15;
/// Maximum characters of a passage included in search results.
const PASSAGE_LIMIT: usize = 500;
/// Maximum document entries shown by [`list_documents`].
const MAX_DOCS_TO_SHOW: usize = 25;
/// Default and maximum term counts for [`get_collection_terms`].
const TERMS_DEFAULT_LIMIT: usize = 50;
const TERMS_MAX_LIMIT: usize = 200;

/// Wrap an internal failure for safe model-facing presentation.
fn infrastructure_error(error: impl std::fmt::Display) -> ToolExecutionError {
    ToolExecutionError::from_error(std::io::Error::other(error.to_string()))
}

// ==================== search ====================

/// One search hit with the fields the model needs for citation.
#[derive(Debug, Serialize)]
pub struct SearchHitResult {
    /// Name of the document the passage belongs to.
    pub document_name: String,
    /// Document ID (use with `read_chunk`).
    pub document_id: String,
    /// Chunk index within the document.
    pub chunk_index: i64,
    /// ID of the collection the document belongs to.
    pub collection_id: String,
    /// Collection name.
    pub collection_name: String,
    /// Passage content, truncated to ~500 characters.
    pub passage: String,
    /// Whether the passage was truncated.
    pub truncated: bool,
    /// First page the chunk appears on (0 if unknown).
    pub start_page: i64,
    /// Last page the chunk appears on (0 if unknown).
    pub end_page: i64,
    /// Total page count of the document.
    pub page_count: i64,
    /// Relevance score.
    pub score: f64,
}

/// Output of [`search`].
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    /// Total number of matching passages before the limit.
    pub total_hits: usize,
    /// The returned hits.
    pub hits: Vec<SearchHitResult>,
}

/// Keyword search tool over the per-run collection scope.
pub struct Search {
    state: ToolState,
}

impl Search {
    pub fn new(state: ToolState) -> Self {
        Self { state }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct SearchArgs {
    /// The keyword search query.
    pub query: String,
}

impl Tool for Search {
    const NAME: &'static str = "search";
    type Args = SearchArgs;
    type Output = SearchOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Keyword (BM25) full-text search over indexed document chunks. Returns up to 15 passages with document IDs, chunk indices, and page references. Use read_chunk with the document_id and chunk_index to read a full passage.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        args_schema::<SearchArgs>()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: SearchArgs,
    ) -> Result<SearchOutput, ToolExecutionError> {
        let state = &self.state;
        let query = args.query;
        tracing::info!(query = %query, "Executing keyword search");

        let index = &*state.search;
        let collection_ids = state.collection_ids();
        let params = search::SearchParams {
            query: &query,
            limit: SEARCH_LIMIT,
            collection_ids: Some(&collection_ids),
            ..Default::default()
        };

        let results = search::search_index(index, params).map_err(infrastructure_error)?;
        let rtxn = index.read_txn().map_err(infrastructure_error)?;
        let mut hits = Vec::with_capacity(results.hits.len());

        for hit in &results.hits {
            let chunk = search::get_indexed_chunk(index, &rtxn, hit.doc_id)
                .map_err(infrastructure_error)?
                .ok_or_else(|| {
                    infrastructure_error(format!(
                        "search hit references missing indexed chunk {}",
                        hit.doc_id
                    ))
                })?;

            let truncated = chunk.content.chars().count() > PASSAGE_LIMIT;
            let passage: String = chunk.content.chars().take(PASSAGE_LIMIT).collect();
            let collection_name = state.name_of(&chunk.collection_id)?;

            hits.push(SearchHitResult {
                document_name: chunk.parent_name,
                document_id: chunk.parent_id,
                chunk_index: chunk.chunk_index as i64,
                collection_name,
                collection_id: chunk.collection_id,
                passage,
                truncated,
                start_page: chunk.start_page as i64,
                end_page: chunk.end_page as i64,
                page_count: chunk.page_count as i64,
                score: search::compute_hit_score(&hit.scores),
            });
        }

        tracing::info!(query = %query, hits = hits.len(), "Search completed");

        Ok(SearchOutput {
            total_hits: results.total_hits,
            hits,
        })
    }
}

// ==================== read_chunk ====================

/// Output of [`read_chunk`].
#[derive(Debug, Serialize)]
pub struct ReadChunkOutput {
    /// The document the chunk belongs to.
    pub document_id: String,
    /// The chunk index within the document.
    pub chunk_index: usize,
    /// The full text content of the chunk.
    pub content: String,
}

/// Chunk-reading tool over the per-run collection scope.
pub struct ReadChunk {
    state: ToolState,
}

impl ReadChunk {
    pub fn new(state: ToolState) -> Self {
        Self { state }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadChunkArgs {
    /// The document ID from a search hit.
    pub document_id: String,
    /// The chunk index within the document.
    pub chunk_index: usize,
}

impl Tool for ReadChunk {
    const NAME: &'static str = "read_chunk";
    type Args = ReadChunkArgs;
    type Output = ReadChunkOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read the full text of a document chunk by document ID and chunk index, as returned by the search tool.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        args_schema::<ReadChunkArgs>()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: ReadChunkArgs,
    ) -> Result<ReadChunkOutput, ToolExecutionError> {
        let state = &self.state;
        let document_id = args.document_id;
        let chunk_index = args.chunk_index;
        tracing::info!(document_id = %document_id, chunk_index, "Reading chunk");

        let chunk_id = format!("{document_id}_chunk_{chunk_index}");
        let not_found = || {
            ToolExecutionError::not_found(format!(
                "chunk {chunk_index} not found for document {document_id}. Try a different chunk index."
            ))
        };

        let chunk = search::get_indexed_chunk_by_external_id(&state.search, &chunk_id)
            .map_err(infrastructure_error)?
            .ok_or_else(not_found)?;

        if !state
            .collections
            .iter()
            .any(|collection| collection.id == chunk.collection_id)
        {
            return Err(not_found());
        }

        Ok(ReadChunkOutput {
            document_id,
            chunk_index,
            content: chunk.content,
        })
    }
}

// ==================== list_documents ====================

/// One document entry.
#[derive(Debug, Serialize)]
pub struct DocumentEntry {
    /// Document file name.
    pub name: String,
    /// Document ID (use with read_chunk / search).
    pub id: String,
    /// Total page count.
    pub page_count: usize,
}

/// Documents grouped by collection.
#[derive(Debug, Serialize)]
pub struct CollectionGroup {
    /// Collection ID.
    pub collection_id: String,
    /// Collection name.
    pub collection_name: String,
    /// Documents in this collection (sorted by name).
    pub documents: Vec<DocumentEntry>,
}

/// Output of [`list_documents`].
#[derive(Debug, Serialize)]
pub struct ListDocumentsOutput {
    /// Total documents across the scope.
    pub total: usize,
    /// Number of entries included (capped at 25).
    pub shown: usize,
    /// Whether the document list was truncated.
    pub truncated: bool,
    /// Total pages across all documents.
    pub total_pages: usize,
    /// Per-collection groups, sorted by collection name.
    pub collections: Vec<CollectionGroup>,
}

/// Document-listing tool over the per-run collection scope.
pub struct ListDocuments {
    state: ToolState,
}

impl ListDocuments {
    pub fn new(state: ToolState) -> Self {
        Self { state }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ListDocumentsArgs {}

impl Tool for ListDocuments {
    const NAME: &'static str = "list_documents";
    type Args = ListDocumentsArgs;
    type Output = ListDocumentsOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "List all documents in the selected collections, grouped by collection, with document IDs and page counts. At most 25 entries are returned; use search to find specific documents beyond that.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        args_schema::<ListDocumentsArgs>()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: ListDocumentsArgs,
    ) -> Result<ListDocumentsOutput, ToolExecutionError> {
        let state = &self.state;
        tracing::info!("Listing documents");
        let storage = state.storage.read().await;

        // (collection_id, collection_name, document), sorted by collection then
        // document name.
        let mut all_documents = Vec::new();
        for collection in &state.collections {
            let namespace_id = collection
                .id
                .parse::<iroh_docs::NamespaceId>()
                .map_err(|_| {
                    ToolExecutionError::invalid_args(format!(
                        "collection {} is not a valid namespace ID",
                        collection.id
                    ))
                })?;
            let collection_name = state.name_of(&collection.id)?;

            let documents = storage
                .list_documents(namespace_id)
                .await
                .map_err(infrastructure_error)?;
            for doc in documents {
                all_documents.push((collection.id.clone(), collection_name.clone(), doc));
            }
        }

        all_documents.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.name.cmp(&b.2.name)));

        let total = all_documents.len();
        let total_pages: usize = all_documents.iter().map(|(_, _, d)| d.page_count).sum();
        let truncated = total > MAX_DOCS_TO_SHOW;

        // Group by collection, in sorted order.
        let mut groups: Vec<CollectionGroup> = Vec::new();
        for (collection_id, collection_name, doc) in
            all_documents.into_iter().take(MAX_DOCS_TO_SHOW)
        {
            let group = match groups.iter_mut().find(|g| g.collection_id == collection_id) {
                Some(g) => g,
                None => {
                    groups.push(CollectionGroup {
                        collection_id: collection_id.clone(),
                        collection_name,
                        documents: Vec::new(),
                    });
                    groups.last_mut().expect("group just pushed")
                }
            };
            group.documents.push(DocumentEntry {
                name: doc.name,
                id: doc.id,
                page_count: doc.page_count,
            });
        }

        let shown: usize = groups.iter().map(|g| g.documents.len()).sum();

        tracing::info!(document_count = total, total_pages, "Listed documents");

        Ok(ListDocumentsOutput {
            total,
            shown,
            truncated,
            total_pages,
            collections: groups,
        })
    }
}

// ==================== get_collection_terms ====================

/// One term with its document frequency.
#[derive(Debug, Serialize)]
pub struct TermCount {
    /// The indexed term.
    pub term: String,
    /// Number of documents containing the term.
    pub doc_count: u64,
}

/// Output of [`get_collection_terms`].
#[derive(Debug, Serialize)]
pub struct CollectionTermsOutput {
    /// Terms sorted by document frequency (descending).
    pub terms: Vec<TermCount>,
}

/// Collection-term tool over the per-run collection scope.
pub struct GetCollectionTerms {
    state: ToolState,
}

impl GetCollectionTerms {
    pub fn new(state: ToolState) -> Self {
        Self { state }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct GetCollectionTermsArgs {
    /// Maximum number of terms to return (default 50, max 200).
    #[serde(default)]
    pub limit: Option<usize>,
}

impl Tool for GetCollectionTerms {
    const NAME: &'static str = "get_collection_terms";
    type Args = GetCollectionTermsArgs;
    type Output = CollectionTermsOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Get the most common terms in the selected collections by document frequency, to understand what topics are covered. Returns up to `limit` terms (default 50, max 200).".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        args_schema::<GetCollectionTermsArgs>()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: GetCollectionTermsArgs,
    ) -> Result<CollectionTermsOutput, ToolExecutionError> {
        let state = &self.state;
        let limit = args
            .limit
            .unwrap_or(TERMS_DEFAULT_LIMIT)
            .min(TERMS_MAX_LIMIT);

        tracing::info!(limit, "Getting collection terms");

        let collection_ids = state.collection_ids();
        let terms = search::get_collection_terms(&state.search, Some(&collection_ids), limit)
            .map_err(infrastructure_error)?;

        tracing::info!(term_count = terms.len(), "Retrieved collection terms");

        Ok(CollectionTermsOutput {
            terms: terms
                .into_iter()
                .map(|t| TermCount {
                    term: t.term,
                    doc_count: t.doc_count,
                })
                .collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::ChunkToIndex;
    use milli::update::IndexerConfig;
    use rig::tool::{Tool, ToolContext, ToolErrorKind};
    use serde_json::{Map, Value};

    /// Tool state over a temporary index/storage, with an empty scope.
    async fn create_state() -> (tempfile::TempDir, ToolState) {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();
        let (state, _progress_rx) = crate::AppState::new(config).await.unwrap();
        (
            temp_dir,
            ToolState {
                search: state.search,
                storage: state.storage,
                collections: Vec::new(),
            },
        )
    }

    /// Helper to create a chunk for indexing.
    #[allow(clippy::too_many_arguments)]
    fn make_chunk(
        parent_id: &str,
        parent_name: &str,
        content: &str,
        collection_id: &str,
        chunk_index: usize,
        page_count: usize,
        start_page: usize,
        end_page: usize,
    ) -> ChunkToIndex {
        ChunkToIndex {
            id: format!("{parent_id}_chunk_{chunk_index}"),
            parent_id: parent_id.to_string(),
            parent_name: parent_name.to_string(),
            chunk_index,
            content: content.to_string(),
            collection_id: collection_id.to_string(),
            page_count,
            start_page,
            end_page,
        }
    }

    fn scope(id: &str, name: &str) -> Vec<CollectionInfo> {
        vec![CollectionInfo {
            id: id.to_string(),
            name: name.to_string(),
            document_count: 1,
            total_pages: 10,
            created_at: None,
        }]
    }

    /// A searchable row whose `parent_name` is missing, so it cannot be
    /// deserialized as an [`crate::search::IndexedChunk`].
    fn malformed_indexed_chunk() -> Map<String, Value> {
        let mut doc = Map::new();
        doc.insert("id".into(), Value::String("doc1_chunk_0".into()));
        doc.insert("parent_id".into(), Value::String("doc1".into()));
        doc.insert("chunk_index".into(), Value::Number(0.into()));
        doc.insert(
            "content".into(),
            Value::String("climate climate malformed row".into()),
        );
        doc.insert("collection_id".into(), Value::String("col1".into()));
        doc.insert("page_count".into(), Value::Number(1.into()));
        doc.insert("start_page".into(), Value::Number(1.into()));
        doc.insert("end_page".into(), Value::Number(1.into()));
        doc
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_no_hits() {
        let (_tmp, mut state) = create_state().await;
        state.collections = scope("col1", "Research");
        let out = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: String::new(),
                },
            )
            .await
            .unwrap();
        assert!(out.hits.is_empty());
    }

    #[tokio::test]
    async fn test_search_finds_indexed_document() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "Climate_Report.pdf",
                "Global temperatures are rising due to greenhouse gases.",
                "research",
                0,
                10,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("research", "Research Papers");

        let out = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: "climate temperatures".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(!out.hits.is_empty());
        let hit = &out.hits[0];
        assert_eq!(hit.document_name, "Climate_Report.pdf");
        assert_eq!(hit.document_id, "doc1");
        assert_eq!(hit.collection_name, "Research Papers");
        assert_eq!(hit.page_count, 10);
        assert!(hit.score > 0.0);
        assert!(!hit.truncated);
    }

    #[tokio::test]
    async fn test_search_filters_by_collection() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![
                make_chunk(
                    "doc1",
                    "Research_Paper.pdf",
                    "Scientific research on climate change.",
                    "research_col",
                    0,
                    10,
                    1,
                    1,
                ),
                make_chunk(
                    "doc2",
                    "Finance_Report.pdf",
                    "Financial analysis for climate initiatives.",
                    "finance_col",
                    0,
                    5,
                    1,
                    1,
                ),
            ],
        )
        .unwrap();
        state.collections = scope("research_col", "Research");

        let out = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: "climate".to_string(),
                },
            )
            .await
            .unwrap();

        let names: Vec<&str> = out.hits.iter().map(|h| h.document_name.as_str()).collect();
        assert!(names.contains(&"Research_Paper.pdf"));
        assert!(!names.contains(&"Finance_Report.pdf"));
    }

    #[tokio::test]
    async fn test_search_truncates_long_passages() {
        let (_tmp, mut state) = create_state().await;
        let long_content = "climate research findings ".repeat(30); // ~780 chars
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "Long.pdf",
                &long_content,
                "col1",
                0,
                1,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("col1", "Long Docs");

        let out = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: "climate".to_string(),
                },
            )
            .await
            .unwrap();

        assert!(!out.hits.is_empty());
        assert!(out.hits[0].truncated);
        assert!(out.hits[0].passage.chars().count() <= 500);
    }

    #[tokio::test]
    async fn test_search_page_references_preserved() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![
                make_chunk("doc1", "Single.pdf", "Content A", "col1", 0, 10, 5, 5),
                make_chunk("doc2", "Range.pdf", "Content B", "col1", 0, 20, 3, 7),
            ],
        )
        .unwrap();
        state.collections = scope("col1", "Mixed");

        let out = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: "Content".to_string(),
                },
            )
            .await
            .unwrap();

        let single = out
            .hits
            .iter()
            .find(|h| h.document_name == "Single.pdf")
            .unwrap();
        assert_eq!(
            (single.start_page, single.end_page, single.page_count),
            (5, 5, 10)
        );
        let range = out
            .hits
            .iter()
            .find(|h| h.document_name == "Range.pdf")
            .unwrap();
        assert_eq!(
            (range.start_page, range.end_page, range.page_count),
            (3, 7, 20)
        );
    }

    #[tokio::test]
    async fn test_search_rejects_malformed_indexed_chunk() {
        let (_tmp, mut state) = create_state().await;
        search::index_documents_for_test(&state.search, vec![malformed_indexed_chunk()]).unwrap();
        state.collections = scope("col1", "Malformed");

        let err = Search::new(state)
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: "climate".to_string(),
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::Other);
        assert!(err.to_string().contains("does not match the search schema"));
    }

    #[tokio::test]
    async fn test_read_chunk_success() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "test_doc",
                "Test_Document.pdf",
                "This is the content of chunk 0 with important information.",
                "collection1",
                0,
                5,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("collection1", "Collection One");

        let out = ReadChunk::new(state)
            .call(
                &mut ToolContext::new(),
                ReadChunkArgs {
                    document_id: "test_doc".to_string(),
                    chunk_index: 0,
                },
            )
            .await
            .unwrap();

        assert_eq!(out.document_id, "test_doc");
        assert_eq!(out.chunk_index, 0);
        assert!(out.content.contains("important information"));
    }

    #[tokio::test]
    async fn test_read_chunk_not_found() {
        let (_tmp, state) = create_state().await;
        let err = ReadChunk::new(state)
            .call(
                &mut ToolContext::new(),
                ReadChunkArgs {
                    document_id: "nonexistent_doc".to_string(),
                    chunk_index: 0,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::NotFound);
        assert!(err.to_string().contains("nonexistent_doc"));
    }

    #[tokio::test]
    async fn test_read_chunk_wrong_index() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "test_doc",
                "Test.pdf",
                "Content of chunk 0",
                "col1",
                0,
                1,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("col1", "Collection One");

        let err = ReadChunk::new(state)
            .call(
                &mut ToolContext::new(),
                ReadChunkArgs {
                    document_id: "test_doc".to_string(),
                    chunk_index: 5,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::NotFound);
        assert!(err.to_string().to_lowercase().contains("chunk 5"));
    }

    #[tokio::test]
    async fn test_read_chunk_rejects_chunk_outside_scope() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "test_doc",
                "Test.pdf",
                "Content in another collection",
                "other_collection",
                0,
                1,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("active_collection", "Active");

        let err = ReadChunk::new(state)
            .call(
                &mut ToolContext::new(),
                ReadChunkArgs {
                    document_id: "test_doc".to_string(),
                    chunk_index: 0,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::NotFound);
        assert!(err.to_string().contains("test_doc"));
    }

    #[tokio::test]
    async fn test_read_chunk_rejects_malformed_indexed_chunk() {
        let (_tmp, mut state) = create_state().await;
        search::index_documents_for_test(&state.search, vec![malformed_indexed_chunk()]).unwrap();
        state.collections = scope("col1", "Malformed");

        let err = ReadChunk::new(state)
            .call(
                &mut ToolContext::new(),
                ReadChunkArgs {
                    document_id: "doc1".to_string(),
                    chunk_index: 0,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::Other);
        assert!(err.to_string().contains("does not match the search schema"));
    }

    #[tokio::test]
    async fn test_list_documents_rejects_invalid_collection_id() {
        let (_tmp, mut state) = create_state().await;
        state.collections = scope("not-a-namespace-id", "Invalid");

        let err = ListDocuments::new(state)
            .call(&mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap_err();

        assert_eq!(err.kind(), ToolErrorKind::InvalidArgs);
        assert!(err.to_string().contains("not-a-namespace-id"));
    }

    #[tokio::test]
    async fn test_list_documents_with_valid_scope() {
        let (_tmp, mut state) = create_state().await;
        let namespace_id = {
            let storage = state.storage.write().await;
            storage
                .create_collection("Empty Collection")
                .await
                .unwrap()
                .0
        };
        state.collections = scope(&namespace_id.to_string(), "Empty Collection");

        let out = ListDocuments::new(state)
            .call(&mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap();

        assert_eq!(out.total, 0);
        assert!(out.collections.is_empty());
    }

    #[tokio::test]
    async fn test_collection_terms_respects_limit() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "varied.pdf",
                "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu",
                "col1",
                0,
                1,
                1,
                1,
            )],
        )
        .unwrap();
        state.collections = scope("col1", "Varied");

        let out = GetCollectionTerms::new(state)
            .call(
                &mut ToolContext::new(),
                GetCollectionTermsArgs { limit: Some(3) },
            )
            .await
            .unwrap();

        assert!(out.terms.len() <= 3);
    }

    #[tokio::test]
    async fn test_collection_terms_max_limit_enforced() {
        let (_tmp, mut state) = create_state().await;
        state.collections = scope("col1", "Empty");

        let out = GetCollectionTerms::new(state)
            .call(
                &mut ToolContext::new(),
                GetCollectionTermsArgs { limit: Some(500) },
            )
            .await
            .unwrap();
        assert!(out.terms.len() <= TERMS_MAX_LIMIT);
    }

    #[tokio::test]
    async fn test_collection_terms_empty_index() {
        let (_tmp, mut state) = create_state().await;
        state.collections = scope("col1", "Empty");

        let out = GetCollectionTerms::new(state)
            .call(
                &mut ToolContext::new(),
                GetCollectionTermsArgs { limit: None },
            )
            .await
            .unwrap();
        assert!(out.terms.is_empty());
    }

    #[tokio::test]
    async fn test_collection_terms_with_documents() {
        let (_tmp, mut state) = create_state().await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![
                make_chunk(
                    "doc1",
                    "Climate_Report.pdf",
                    "Climate change affects global temperatures and weather patterns.",
                    "research",
                    0,
                    10,
                    1,
                    1,
                ),
                make_chunk(
                    "doc2",
                    "Weather_Analysis.pdf",
                    "Weather patterns are shifting due to climate factors.",
                    "research",
                    0,
                    5,
                    1,
                    1,
                ),
            ],
        )
        .unwrap();
        state.collections = scope("research", "Research");

        let out = GetCollectionTerms::new(state)
            .call(
                &mut ToolContext::new(),
                GetCollectionTermsArgs { limit: Some(10) },
            )
            .await
            .unwrap();

        assert!(!out.terms.is_empty());
        let climate = out.terms.iter().find(|t| t.term == "climate").unwrap();
        assert_eq!(climate.doc_count, 2);
    }

    #[tokio::test]
    async fn tool_definitions_expose_names_and_required_params() {
        let (_tmp, state) = create_state().await;
        assert_eq!(<Search as Tool>::NAME, "search");
        assert_eq!(
            Search::new(state.clone()).parameters()["required"],
            serde_json::json!(["query"])
        );

        assert_eq!(<ReadChunk as Tool>::NAME, "read_chunk");
        assert_eq!(
            ReadChunk::new(state.clone()).parameters()["required"],
            serde_json::json!(["document_id", "chunk_index"])
        );

        assert_eq!(<ListDocuments as Tool>::NAME, "list_documents");
        assert_eq!(<GetCollectionTerms as Tool>::NAME, "get_collection_terms");
    }
}
