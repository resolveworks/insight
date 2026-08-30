//! Typed rig tools for the Insight agent.
//!
//! Each tool is a manual [`rig::tool::Tool`] implementation carrying shared
//! application state ([`ToolDeps`]) plus an optional collection scope
//! (`Option<Vec<CollectionInfo>>`; `None` or `Some(vec![])` means "all
//! collections" where that makes sense). Outputs are typed serializable
//! structs — rig converts them into structured model output at dispatch time.

use std::collections::HashMap;
use std::sync::Arc;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde::Serialize;
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::search;
use crate::storage::Storage;
use crate::CollectionInfo;

/// Shared state needed by the document tools.
#[derive(Clone)]
pub struct ToolDeps {
    /// milli search index (shared for reads; LMDB handles concurrency)
    pub search: Arc<milli::Index>,
    /// iroh storage, read-locked during tool calls
    pub storage: Arc<RwLock<Storage>>,
}

impl ToolDeps {
    /// Build tool dependencies from an [`crate::AppState`].
    pub fn from_state(state: &crate::AppState) -> Self {
        Self {
            search: state.search.clone(),
            storage: state.storage.clone(),
        }
    }
}

/// Build a `collection_id -> name` lookup from the scope, falling back to the
/// first 8 characters of the ID plus an ellipsis.
fn collection_name_lookup(scope: &Option<Vec<CollectionInfo>>) -> HashMap<String, String> {
    scope
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Resolve a collection name with the 8-character fallback.
fn resolve_collection_name(lookup: &HashMap<String, String>, collection_id: &str) -> String {
    lookup
        .get(collection_id)
        .cloned()
        .unwrap_or_else(|| collection_id.chars().take(8).collect::<String>() + "...")
}

/// Maximum number of passages returned by [`SearchTool`].
const SEARCH_LIMIT: usize = 15;
/// Maximum characters of a passage included in search results.
const PASSAGE_LIMIT: usize = 500;
/// Maximum document entries shown by [`ListDocumentsTool`].
const MAX_DOCS_TO_SHOW: usize = 25;
/// Default and maximum term counts for [`CollectionTermsTool`].
const TERMS_DEFAULT_LIMIT: usize = 50;
const TERMS_MAX_LIMIT: usize = 200;

// ==================== search ====================

/// Keyword (BM25) search over indexed document chunks.
#[derive(Clone)]
pub struct SearchTool {
    deps: Arc<ToolDeps>,
    scope: Option<Vec<CollectionInfo>>,
}

/// Arguments for [`SearchTool`].
#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    /// The search query.
    pub query: String,
}

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
    /// Collection name (8-char ID fallback when unknown).
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

/// Output of [`SearchTool`].
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    /// Total number of matching passages before the limit.
    pub total_hits: usize,
    /// The returned hits.
    pub hits: Vec<SearchHitResult>,
}

/// Errors from [`SearchTool`].
#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    /// The search index query failed.
    #[error("search error: {0}")]
    Search(#[from] anyhow::Error),
}

impl SearchTool {
    /// Create a search tool scoped to the given collections (`None` = all).
    pub fn new(deps: Arc<ToolDeps>, scope: Option<Vec<CollectionInfo>>) -> Self {
        Self { deps, scope }
    }
}

impl Tool for SearchTool {
    const NAME: &'static str = "search";
    type Args = SearchArgs;
    type Output = SearchOutput;
    type Error = SearchError;

    fn description(&self) -> String {
        "Keyword (BM25) full-text search over indexed document chunks. Returns up to 15 passages with document IDs, chunk indices, and page references. Use read_chunk with the document_id and chunk_index to read a full passage.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The keyword search query"
                }
            },
            "required": ["query"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        info!(query = %args.query, "Executing keyword search");

        let index = &*self.deps.search;
        let collection_ids: Option<Vec<String>> = self
            .scope
            .as_ref()
            .filter(|c| !c.is_empty())
            .map(|c| c.iter().map(|ci| ci.id.clone()).collect());

        let params = search::SearchParams {
            query: &args.query,
            limit: SEARCH_LIMIT,
            collection_ids: collection_ids.as_deref(),
            ..Default::default()
        };

        let results = search::search_index(index, params)?;
        let names = collection_name_lookup(&self.scope);
        let rtxn = index.read_txn().map_err(anyhow::Error::new)?;
        let mut hits = Vec::with_capacity(results.hits.len());

        for hit in &results.hits {
            let Some(doc) = search::get_document(index, &rtxn, hit.doc_id)? else {
                continue;
            };

            let get_str = |key: &str| -> String {
                doc.get(key)
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            let get_num =
                |key: &str| -> i64 { doc.get(key).and_then(|v| v.as_i64()).unwrap_or_default() };

            let content = get_str("content");
            let truncated = content.chars().count() > PASSAGE_LIMIT;
            let passage: String = content.chars().take(PASSAGE_LIMIT).collect();

            let collection_id = get_str("collection_id");
            hits.push(SearchHitResult {
                document_name: get_str("parent_name"),
                document_id: get_str("parent_id"),
                chunk_index: get_num("chunk_index"),
                collection_name: resolve_collection_name(&names, &collection_id),
                collection_id,
                passage,
                truncated,
                start_page: get_num("start_page"),
                end_page: get_num("end_page"),
                page_count: get_num("page_count"),
                score: search::compute_hit_score(&hit.scores),
            });
        }
        drop(rtxn);

        info!(query = %args.query, hits = hits.len(), "Search completed");

        Ok(SearchOutput {
            total_hits: results.total_hits,
            hits,
        })
    }
}

// ==================== read_chunk ====================

/// Read the full text of one indexed chunk by document ID and chunk index.
#[derive(Clone)]
pub struct ReadChunkTool {
    deps: Arc<ToolDeps>,
}

/// Arguments for [`ReadChunkTool`].
#[derive(Debug, Deserialize)]
pub struct ReadChunkArgs {
    /// The document ID from a search hit.
    pub document_id: String,
    /// The chunk index within the document.
    pub chunk_index: usize,
}

/// Output of [`ReadChunkTool`].
#[derive(Debug, Serialize)]
pub struct ReadChunkOutput {
    /// The document the chunk belongs to.
    pub document_id: String,
    /// The chunk index within the document.
    pub chunk_index: usize,
    /// The full text content of the chunk.
    pub content: String,
}

/// Errors from [`ReadChunkTool`].
#[derive(Debug, thiserror::Error)]
pub enum ReadChunkError {
    /// No chunk with the given document/chunk pair exists.
    #[error(
        "chunk {chunk_index} not found for document {document_id}. Try a different chunk index."
    )]
    NotFound {
        /// The requested document ID.
        document_id: String,
        /// The requested chunk index.
        chunk_index: usize,
    },
    /// The search index lookup failed.
    #[error("error reading chunk: {0}")]
    Search(#[from] anyhow::Error),
}

impl ReadChunkTool {
    /// Create a read_chunk tool. It is scope-independent: any indexed chunk
    /// can be read by ID.
    pub fn new(deps: Arc<ToolDeps>) -> Self {
        Self { deps }
    }
}

impl Tool for ReadChunkTool {
    const NAME: &'static str = "read_chunk";
    type Args = ReadChunkArgs;
    type Output = ReadChunkOutput;
    type Error = ReadChunkError;

    fn description(&self) -> String {
        "Read the full text of a document chunk by document ID and chunk index, as returned by the search tool.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "The document ID from a search hit"
                },
                "chunk_index": {
                    "type": "integer",
                    "description": "The chunk index within the document"
                }
            },
            "required": ["document_id", "chunk_index"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        info!(
            document_id = %args.document_id,
            chunk_index = args.chunk_index,
            "Reading chunk"
        );

        let chunk_id = format!("{}_chunk_{}", args.document_id, args.chunk_index);
        let index = &*self.deps.search;

        match search::get_document_by_external_id(index, &chunk_id)? {
            Some(content) => {
                info!(
                    document_id = %args.document_id,
                    chunk_index = args.chunk_index,
                    content_len = content.len(),
                    "Chunk read successfully"
                );
                Ok(ReadChunkOutput {
                    document_id: args.document_id,
                    chunk_index: args.chunk_index,
                    content,
                })
            }
            None => {
                warn!(
                    document_id = %args.document_id,
                    chunk_index = args.chunk_index,
                    "Chunk not found"
                );
                Err(ReadChunkError::NotFound {
                    document_id: args.document_id,
                    chunk_index: args.chunk_index,
                })
            }
        }
    }
}

// ==================== list_documents ====================

/// List documents across the scoped collections.
#[derive(Clone)]
pub struct ListDocumentsTool {
    deps: Arc<ToolDeps>,
    scope: Option<Vec<CollectionInfo>>,
}

/// Arguments for [`ListDocumentsTool`] (none; the scope is fixed at build time).
#[derive(Debug, Deserialize)]
pub struct ListDocumentsArgs {}

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
    /// Collection name (8-char ID fallback when unknown).
    pub collection_name: String,
    /// Documents in this collection (sorted by name).
    pub documents: Vec<DocumentEntry>,
}

/// Output of [`ListDocumentsTool`].
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

/// Errors from [`ListDocumentsTool`].
#[derive(Debug, thiserror::Error)]
pub enum ListDocumentsError {
    /// No collections are selected in the scope.
    #[error("no collections selected. Please select collections to list documents from.")]
    NoCollections,
    /// Listing documents from storage failed.
    #[error("storage error: {0}")]
    Storage(#[from] anyhow::Error),
}

impl ListDocumentsTool {
    /// Create a list_documents tool scoped to the given collections.
    pub fn new(deps: Arc<ToolDeps>, scope: Option<Vec<CollectionInfo>>) -> Self {
        Self { deps, scope }
    }
}

impl Tool for ListDocumentsTool {
    const NAME: &'static str = "list_documents";
    type Args = ListDocumentsArgs;
    type Output = ListDocumentsOutput;
    type Error = ListDocumentsError;

    fn description(&self) -> String {
        "List all documents in the selected collections, grouped by collection, with document IDs and page counts. At most 25 entries are returned; use search to find specific documents beyond that.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {}
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        info!("Listing documents");

        let Some(collections) = self.scope.as_ref().filter(|c| !c.is_empty()) else {
            warn!("List documents called with no collections selected");
            return Err(ListDocumentsError::NoCollections);
        };

        let names = collection_name_lookup(&self.scope);
        let storage = self.deps.storage.read().await;

        // (collection_name, document) pairs, sorted like the old formatter.
        let mut all_documents = Vec::new();
        for collection in collections {
            let Ok(namespace_id) = collection.id.parse::<iroh_docs::NamespaceId>() else {
                warn!(collection_id = %collection.id, "Invalid collection ID, skipping");
                continue;
            };

            let documents = storage.list_documents(namespace_id).await?;
            for doc in documents {
                all_documents.push((collection.id.clone(), doc));
            }
        }
        drop(storage);

        all_documents.sort_by(|a, b| {
            resolve_collection_name(&names, &a.0)
                .cmp(&resolve_collection_name(&names, &b.0))
                .then_with(|| a.1.name.cmp(&b.1.name))
        });

        let total = all_documents.len();
        let total_pages: usize = all_documents.iter().map(|(_, d)| d.page_count).sum();
        let truncated = total > MAX_DOCS_TO_SHOW;

        // Group by collection, in sorted order.
        let mut groups: Vec<CollectionGroup> = Vec::new();
        for (collection_id, doc) in all_documents.into_iter().take(MAX_DOCS_TO_SHOW) {
            let collection_name = resolve_collection_name(&names, &collection_id);
            let group = match groups
                .iter_mut()
                .find(|g| g.collection_id == collection_id && g.collection_name == collection_name)
            {
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

        info!(
            document_count = total,
            total_pages = total_pages,
            "Listed documents"
        );

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

/// Get the most common terms in the scoped collections.
#[derive(Clone)]
pub struct CollectionTermsTool {
    deps: Arc<ToolDeps>,
    scope: Option<Vec<CollectionInfo>>,
}

/// Arguments for [`CollectionTermsTool`].
#[derive(Debug, Deserialize)]
pub struct CollectionTermsArgs {
    /// Maximum number of terms to return (default 50, capped at 200).
    pub limit: Option<usize>,
}

/// One term with its document frequency.
#[derive(Debug, Serialize)]
pub struct TermCount {
    /// The indexed term.
    pub term: String,
    /// Number of documents containing the term.
    pub doc_count: u64,
}

/// Output of [`CollectionTermsTool`].
#[derive(Debug, Serialize)]
pub struct CollectionTermsOutput {
    /// Terms sorted by document frequency (descending).
    pub terms: Vec<TermCount>,
}

/// Errors from [`CollectionTermsTool`].
#[derive(Debug, thiserror::Error)]
pub enum CollectionTermsError {
    /// Reading terms from the index failed.
    #[error("error getting collection terms: {0}")]
    Search(#[from] anyhow::Error),
}

impl CollectionTermsTool {
    /// Create a get_collection_terms tool scoped to the given collections
    /// (`None` = all).
    pub fn new(deps: Arc<ToolDeps>, scope: Option<Vec<CollectionInfo>>) -> Self {
        Self { deps, scope }
    }
}

impl Tool for CollectionTermsTool {
    const NAME: &'static str = "get_collection_terms";
    type Args = CollectionTermsArgs;
    type Output = CollectionTermsOutput;
    type Error = CollectionTermsError;

    fn description(&self) -> String {
        "Get the most common terms in the selected collections by document frequency, to understand what topics are covered. Returns up to `limit` terms (default 50, max 200).".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of terms to return (default 50, max 200)"
                }
            }
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let limit = args
            .limit
            .unwrap_or(TERMS_DEFAULT_LIMIT)
            .min(TERMS_MAX_LIMIT);

        info!(limit = limit, "Getting collection terms");

        let index = &*self.deps.search;
        let collection_ids: Option<Vec<String>> = self
            .scope
            .as_ref()
            .filter(|c| !c.is_empty())
            .map(|c| c.iter().map(|ci| ci.id.clone()).collect());

        let terms = search::get_collection_terms(index, collection_ids.as_deref(), limit)?;
        info!(term_count = terms.len(), "Retrieved collection terms");

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

    /// Create tool deps over a temporary index/storage.
    async fn create_deps() -> (tempfile::TempDir, Arc<ToolDeps>) {
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
        (temp_dir, Arc::new(ToolDeps::from_state(&state)))
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
            id: format!("{}_chunk_{}", parent_id, chunk_index),
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

    async fn call_search(tool: &SearchTool, query: &str) -> Result<SearchOutput, SearchError> {
        Tool::call(
            tool,
            &mut ToolContext::new(),
            SearchArgs {
                query: query.to_string(),
            },
        )
        .await
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_no_hits() {
        let (_tmp, deps) = create_deps().await;
        let tool = SearchTool::new(deps, None);
        let out = call_search(&tool, "").await.unwrap();
        assert!(out.hits.is_empty());
    }

    #[tokio::test]
    async fn test_search_finds_indexed_document() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = SearchTool::new(deps, Some(scope("research", "Research Papers")));
        let out = call_search(&tool, "climate temperatures").await.unwrap();

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
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = SearchTool::new(deps, Some(scope("research_col", "Research")));
        let out = call_search(&tool, "climate").await.unwrap();

        let names: Vec<&str> = out.hits.iter().map(|h| h.document_name.as_str()).collect();
        assert!(names.contains(&"Research_Paper.pdf"));
        assert!(!names.contains(&"Finance_Report.pdf"));
    }

    #[tokio::test]
    async fn test_search_truncates_long_passages() {
        let (_tmp, deps) = create_deps().await;
        let long_content = "climate research findings ".repeat(30); // ~780 chars
        search::index_chunks_batch(
            &deps.search,
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

        let tool = SearchTool::new(deps, None);
        let out = call_search(&tool, "climate").await.unwrap();

        assert!(!out.hits.is_empty());
        assert!(out.hits[0].truncated);
        assert!(out.hits[0].passage.chars().count() <= 500);
    }

    #[tokio::test]
    async fn test_search_collection_name_fallback() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
            &IndexerConfig::default(),
            vec![make_chunk(
                "doc1",
                "Report.pdf",
                "Important findings about the topic.",
                "col_abcdefgh1234",
                0,
                10,
                1,
                1,
            )],
        )
        .unwrap();

        // No scope: search is unfiltered and the name lookup misses, so the
        // collection name falls back to the 8-char ID prefix.
        let tool = SearchTool::new(deps, None);
        let out = call_search(&tool, "findings").await.unwrap();

        assert!(!out.hits.is_empty());
        assert_eq!(out.hits[0].collection_name, "col_abcd...");
    }

    #[tokio::test]
    async fn test_search_page_references_preserved() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
            &IndexerConfig::default(),
            vec![
                make_chunk("doc1", "Single.pdf", "Content A", "col1", 0, 10, 5, 5),
                make_chunk("doc2", "Range.pdf", "Content B", "col1", 0, 20, 3, 7),
            ],
        )
        .unwrap();

        let tool = SearchTool::new(deps, None);
        let out = call_search(&tool, "Content").await.unwrap();

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
    async fn test_read_chunk_success() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = ReadChunkTool::new(deps);
        let out = Tool::call(
            &tool,
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
        let (_tmp, deps) = create_deps().await;
        let tool = ReadChunkTool::new(deps);
        let err = Tool::call(
            &tool,
            &mut ToolContext::new(),
            ReadChunkArgs {
                document_id: "nonexistent_doc".to_string(),
                chunk_index: 0,
            },
        )
        .await
        .unwrap_err();

        assert!(err.to_string().contains("not found"));
        assert!(err.to_string().contains("nonexistent_doc"));
    }

    #[tokio::test]
    async fn test_read_chunk_wrong_index() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = ReadChunkTool::new(deps);
        let err = Tool::call(
            &tool,
            &mut ToolContext::new(),
            ReadChunkArgs {
                document_id: "test_doc".to_string(),
                chunk_index: 5,
            },
        )
        .await
        .unwrap_err();

        assert!(err.to_string().to_lowercase().contains("chunk 5"));
        assert!(err.to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_documents_errors_without_scope() {
        let (_tmp, deps) = create_deps().await;
        let tool = ListDocumentsTool::new(deps, None);
        let err = Tool::call(&tool, &mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap_err();
        assert!(err
            .to_string()
            .to_lowercase()
            .contains("no collections selected"));
        assert!(matches!(err, ListDocumentsError::NoCollections));
    }

    #[tokio::test]
    async fn test_list_documents_errors_with_empty_scope() {
        let (_tmp, deps) = create_deps().await;
        let tool = ListDocumentsTool::new(deps, Some(vec![]));
        let err = Tool::call(&tool, &mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap_err();
        assert!(matches!(err, ListDocumentsError::NoCollections));
    }

    #[tokio::test]
    async fn test_list_documents_with_valid_scope() {
        let (_tmp, deps) = create_deps().await;
        // A real namespace ID so parsing succeeds; storage has no docs.
        let ns: iroh_docs::NamespaceId = (&rand::random::<[u8; 32]>()).into();
        let tool = ListDocumentsTool::new(deps, Some(scope(&ns.to_string(), "Empty Collection")));
        let out = Tool::call(&tool, &mut ToolContext::new(), ListDocumentsArgs {})
            .await
            .unwrap();

        assert_eq!(out.total, 0);
        assert!(out.collections.is_empty());
    }

    #[tokio::test]
    async fn test_collection_terms_respects_limit() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = CollectionTermsTool::new(deps, None);
        let out = Tool::call(
            &tool,
            &mut ToolContext::new(),
            CollectionTermsArgs { limit: Some(3) },
        )
        .await
        .unwrap();

        assert!(out.terms.len() <= 3);
    }

    #[tokio::test]
    async fn test_collection_terms_max_limit_enforced() {
        let (_tmp, deps) = create_deps().await;
        let tool = CollectionTermsTool::new(deps, None);
        let out = Tool::call(
            &tool,
            &mut ToolContext::new(),
            CollectionTermsArgs { limit: Some(500) },
        )
        .await
        .unwrap();
        assert!(out.terms.len() <= TERMS_MAX_LIMIT);
    }

    #[tokio::test]
    async fn test_collection_terms_empty_index() {
        let (_tmp, deps) = create_deps().await;
        let tool = CollectionTermsTool::new(deps, None);
        let out = Tool::call(
            &tool,
            &mut ToolContext::new(),
            CollectionTermsArgs { limit: None },
        )
        .await
        .unwrap();
        assert!(out.terms.is_empty());
    }

    #[tokio::test]
    async fn test_collection_terms_with_documents() {
        let (_tmp, deps) = create_deps().await;
        search::index_chunks_batch(
            &deps.search,
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

        let tool = CollectionTermsTool::new(deps, None);
        let out = Tool::call(
            &tool,
            &mut ToolContext::new(),
            CollectionTermsArgs { limit: Some(10) },
        )
        .await
        .unwrap();

        assert!(!out.terms.is_empty());
        let climate = out.terms.iter().find(|t| t.term == "climate").unwrap();
        assert_eq!(climate.doc_count, 2);
    }

    #[tokio::test]
    async fn test_search_output_is_serializable() {
        // The output struct is the model-facing contract; it must serialize.
        let out = SearchOutput {
            total_hits: 0,
            hits: vec![],
        };
        let json = serde_json::to_value(&out).unwrap();
        assert_eq!(json["total_hits"], 0);
    }
}
