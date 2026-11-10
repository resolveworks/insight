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

/// Per-run state shared by the document tools.
#[derive(Clone)]
pub struct ToolState {
    /// milli search index (shared for reads; LMDB handles concurrency)
    pub search: Arc<milli::Index>,
    /// iroh storage, read-locked during tool calls
    pub storage: Arc<RwLock<Storage>>,
    /// The conversation's collection: the only collection the tools cover.
    pub collection_id: String,
}

impl ToolState {
    /// Build tool state from an [`crate::AppState`] and a collection id.
    pub fn new(state: &crate::AppState, collection_id: String) -> Self {
        Self {
            search: state.search.clone(),
            storage: state.storage.clone(),
            collection_id,
        }
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
    /// Passage content, truncated to ~500 characters.
    pub passage: String,
    /// Whether the passage was truncated.
    pub truncated: bool,
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
        "Keyword (BM25) full-text search over indexed document chunks. Returns up to 15 passages with document IDs and chunk indices. Use read_chunk with the document_id and chunk_index to read a full passage.".to_string()
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
        let params = search::SearchParams {
            query: &query,
            limit: SEARCH_LIMIT,
            offset: 0,
            collection_id: &state.collection_id,
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

            hits.push(SearchHitResult {
                document_name: chunk.parent_name,
                document_id: chunk.parent_id,
                chunk_index: chunk.chunk_index as i64,
                passage,
                truncated,
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

        let chunk_id = search::chunk_external_id(&state.collection_id, &document_id, chunk_index);
        let not_found = || {
            ToolExecutionError::not_found(format!(
                "chunk {chunk_index} not found for document {document_id}. Try a different chunk index."
            ))
        };

        let chunk = search::get_indexed_chunk_by_external_id(&state.search, &chunk_id)
            .map_err(infrastructure_error)?
            .ok_or_else(not_found)?;

        if chunk.collection_id != state.collection_id {
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
    /// Total page count, when the source is paginated.
    pub page_count: Option<usize>,
}

/// Output of [`list_documents`].
#[derive(Debug, Serialize)]
pub struct ListDocumentsOutput {
    /// Total documents in the collection.
    pub total: usize,
    /// Number of entries included (capped at 25).
    pub shown: usize,
    /// Whether the document list was truncated.
    pub truncated: bool,
    /// Total pages across all documents, when known.
    pub total_pages: Option<usize>,
    /// Documents in the collection, sorted by name (capped at 25).
    pub documents: Vec<DocumentEntry>,
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
        "List all documents in the collection, with document IDs and page counts where known. At most 25 entries are returned; use search to find specific documents beyond that.".to_string()
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

        let namespace_id = state
            .collection_id
            .parse::<iroh_docs::NamespaceId>()
            .map_err(|_| {
                ToolExecutionError::invalid_args(format!(
                    "collection {} is not a valid namespace ID",
                    state.collection_id
                ))
            })?;

        let mut documents = storage
            .list_documents(namespace_id)
            .await
            .map_err(infrastructure_error)?;
        documents.sort_by(|a, b| a.name.cmp(&b.name));

        let mut entries = Vec::with_capacity(documents.len().min(MAX_DOCS_TO_SHOW));
        let mut total_pages = None;
        for doc in &documents {
            let page_count = storage
                .page_count(namespace_id, &doc.id)
                .await
                .map_err(infrastructure_error)?;
            if let Some(pages) = page_count {
                *total_pages.get_or_insert(0) += pages;
            }
            if entries.len() < MAX_DOCS_TO_SHOW {
                entries.push(DocumentEntry {
                    page_count,
                    name: doc.name.clone(),
                    id: doc.id.clone(),
                });
            }
        }

        let total = documents.len();
        let shown = entries.len();
        let truncated = total > MAX_DOCS_TO_SHOW;

        tracing::info!(document_count = total, total_pages, "Listed documents");

        Ok(ListDocumentsOutput {
            total,
            shown,
            truncated,
            total_pages,
            documents: entries,
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

/// Collection-term tool over the conversation's collection.
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
        "Get the most common terms in the collection by document frequency, to understand what topics are covered. Returns up to `limit` terms (default 50, max 200).".to_string()
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

        let terms = search::get_collection_terms(&state.search, &state.collection_id, limit)
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
    use crate::text::{Extraction, ParserId, TextSpan, Unit, UnitKind};
    use milli::update::IndexerConfig;
    use rig::tool::{Tool, ToolContext, ToolErrorKind};

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
                collection_id: String::new(),
            },
        )
    }

    /// Create a real collection and return its namespace ID.
    async fn create_collection(state: &ToolState, name: &str) -> String {
        let storage = state.storage.write().await;
        let (namespace_id, _) = storage.create_collection(name).await.unwrap();
        namespace_id.to_string()
    }

    /// Store a document and, when a unit kind is given, an extraction whose
    /// pages cover `text` up to the given end offsets.
    async fn add_document(
        state: &ToolState,
        collection_id: &str,
        id: &str,
        name: &str,
        text: &str,
        page_ends: Vec<usize>,
        unit_kind: Option<UnitKind>,
    ) {
        let namespace_id = collection_id.parse::<iroh_docs::NamespaceId>().unwrap();
        let source = format!("source-{id}");
        let storage = state.storage.read().await;
        let doc_id = storage
            .add_document(namespace_id, name, "application/pdf", source.as_bytes())
            .await
            .unwrap()
            .id;

        if let Some(unit_kind) = unit_kind {
            let mut start = 0;
            let units = page_ends
                .into_iter()
                .enumerate()
                .map(|(index, end)| {
                    let unit = Unit {
                        number: index + 1,
                        spans: vec![TextSpan { start, end }],
                    };
                    start = end;
                    unit
                })
                .collect();
            let extraction = Extraction {
                parser: ParserId {
                    name: "test-parser".to_string(),
                    version: "1".to_string(),
                },
                text: text.to_string(),
                unit_kind: Some(unit_kind),
                units,
            };
            storage
                .write_extraction(namespace_id, &doc_id, &extraction)
                .await
                .unwrap();
            storage
                .set_canonical_extraction(namespace_id, &doc_id, &extraction.parser)
                .await
                .unwrap();
        }
    }

    /// Helper to create a chunk for indexing.
    #[allow(clippy::too_many_arguments)]
    fn make_chunk(
        parent_id: &str,
        parent_name: &str,
        content: &str,
        collection_id: &str,
        chunk_index: usize,
        text_offset_start: usize,
    ) -> ChunkToIndex {
        ChunkToIndex {
            id: crate::search::chunk_external_id(collection_id, parent_id, chunk_index),
            parent_id: parent_id.to_string(),
            parent_name: parent_name.to_string(),
            chunk_index,
            content: content.to_string(),
            collection_id: collection_id.to_string(),
            text_offset_start,
            text_offset_end: text_offset_start + content.len(),
        }
    }

    fn scope(id: &str) -> String {
        id.to_string()
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_no_hits() {
        let (_tmp, mut state) = create_state().await;
        state.collection_id = scope("col1");
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
        let content = "Global temperatures are rising due to greenhouse gases.";
        let collection = create_collection(&state, "Research Papers").await;
        add_document(
            &state,
            &collection,
            "doc1",
            "Climate_Report.pdf",
            content,
            vec![content.len()],
            Some(UnitKind::Page),
        )
        .await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "Climate_Report.pdf",
                content,
                &collection,
                0,
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope(&collection);

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
        assert!(hit.score > 0.0);
        assert!(!hit.truncated);
    }

    #[tokio::test]
    async fn test_search_filters_by_collection() {
        let (_tmp, mut state) = create_state().await;
        let research = create_collection(&state, "Research").await;
        let finance = create_collection(&state, "Finance").await;
        let research_content = "Scientific research on climate change.";
        let finance_content = "Financial analysis for climate initiatives.";
        add_document(
            &state,
            &research,
            "doc1",
            "Research_Paper.pdf",
            research_content,
            vec![],
            None,
        )
        .await;
        add_document(
            &state,
            &finance,
            "doc2",
            "Finance_Report.pdf",
            finance_content,
            vec![],
            None,
        )
        .await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![
                make_chunk(
                    "doc1",
                    "Research_Paper.pdf",
                    research_content,
                    &research,
                    0,
                    0,
                ),
                make_chunk(
                    "doc2",
                    "Finance_Report.pdf",
                    finance_content,
                    &finance,
                    0,
                    0,
                ),
            ],
        )
        .unwrap();
        state.collection_id = scope(&research);

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
        let collection = create_collection(&state, "Long Docs").await;
        add_document(
            &state,
            &collection,
            "doc1",
            "Long.pdf",
            &long_content,
            vec![],
            None,
        )
        .await;
        search::index_chunks_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "Long.pdf",
                &long_content,
                &collection,
                0,
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope(&collection);

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
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope("collection1");

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
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope("col1");

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
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope("active_collection");

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
    async fn test_list_documents_rejects_invalid_collection_id() {
        let (_tmp, mut state) = create_state().await;
        state.collection_id = scope("not-a-namespace-id");

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
        state.collection_id = scope(&namespace_id.to_string());

        let out = ListDocuments::new(state)
            .call(&mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap();

        assert_eq!(out.total, 0);
        assert!(out.documents.is_empty());
    }

    #[tokio::test]
    async fn test_list_documents_reports_page_counts() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Paged").await;
        add_document(
            &state,
            &collection,
            "doc-paged",
            "Paged.pdf",
            "0123456789",
            vec![4, 8, 10],
            Some(UnitKind::Page),
        )
        .await;
        add_document(
            &state,
            &collection,
            "doc-unknown",
            "Unknown.pdf",
            "no extraction",
            Vec::new(),
            None,
        )
        .await;
        state.collection_id = scope(&collection);

        let out = ListDocuments::new(state)
            .call(&mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap();

        assert_eq!(out.total, 2);
        assert_eq!(out.total_pages, Some(3));
        let documents = &out.documents;
        assert_eq!(documents.len(), 2);
        assert_eq!(documents[0].name, "Paged.pdf");
        assert_eq!(documents[0].page_count, Some(3));
        assert_eq!(documents[1].name, "Unknown.pdf");
        assert_eq!(documents[1].page_count, None);
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
                0,
            )],
        )
        .unwrap();
        state.collection_id = scope("col1");

        let out = GetCollectionTerms::new(state)
            .call(
                &mut ToolContext::new(),
                GetCollectionTermsArgs { limit: Some(3) },
            )
            .await
            .unwrap();

        assert_eq!(out.terms.len(), 3);
    }

    #[tokio::test]
    async fn test_collection_terms_empty_index() {
        let (_tmp, mut state) = create_state().await;
        state.collection_id = scope("col1");

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
                    0,
                ),
                make_chunk(
                    "doc2",
                    "Weather_Analysis.pdf",
                    "Weather patterns are shifting due to climate factors.",
                    "research",
                    0,
                    0,
                ),
            ],
        )
        .unwrap();
        state.collection_id = scope("research");

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
    async fn tool_definitions_declare_required_params() {
        let (_tmp, state) = create_state().await;
        assert_eq!(
            Search::new(state.clone()).parameters()["required"],
            serde_json::json!(["query"])
        );

        assert_eq!(
            ReadChunk::new(state.clone()).parameters()["required"],
            serde_json::json!(["document_id", "chunk_index"])
        );
    }
}
