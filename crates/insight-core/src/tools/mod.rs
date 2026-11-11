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
use crate::text::{Extraction, ParserId};

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

/// Maximum number of documents returned by [`Search`].
const SEARCH_LIMIT: usize = 15;
/// Maximum characters read by [`ReadDocument`] when `max_chars` is omitted.
const DEFAULT_READ_CHARS: usize = 4_000;
/// Hard cap on characters returned by [`ReadDocument`].
const MAX_READ_CHARS: usize = 8_000;
/// Maximum document entries shown by [`ListDocuments`].
const MAX_DOCS_TO_SHOW: usize = 25;

/// Wrap an internal failure for safe model-facing presentation.
fn infrastructure_error(error: impl std::fmt::Display) -> ToolExecutionError {
    ToolExecutionError::from_error(std::io::Error::other(error.to_string()))
}

/// Parse the run's collection id, reporting an invalid scope as tool args.
fn namespace(state: &ToolState) -> Result<iroh_docs::NamespaceId, ToolExecutionError> {
    state
        .collection_id
        .parse::<iroh_docs::NamespaceId>()
        .map_err(|_| {
            ToolExecutionError::invalid_args(format!(
                "collection {} is not a valid namespace ID",
                state.collection_id
            ))
        })
}

/// Parse a stored `{name}@{version}` parser key.
fn parser_id(key: &str) -> Result<ParserId, ToolExecutionError> {
    let (name, version) = key.split_once('@').ok_or_else(|| {
        ToolExecutionError::invalid_args(format!(
            "extraction {key} is not a parser key of the form name@version"
        ))
    })?;
    if name.is_empty() || version.is_empty() {
        return Err(ToolExecutionError::invalid_args(format!(
            "extraction {key} is not a parser key of the form name@version"
        )));
    }
    Ok(ParserId {
        name: name.to_string(),
        version: version.to_string(),
    })
}

/// Convert a zero-based character offset to a UTF-8 byte offset.
///
/// `Some(text.len())` is returned for an offset equal to the character count.
fn char_to_byte(text: &str, char_offset: usize) -> Option<usize> {
    if char_offset == 0 {
        return Some(0);
    }
    if let Some((byte, _)) = text.char_indices().nth(char_offset) {
        return Some(byte);
    }
    (char_offset == text.chars().count()).then_some(text.len())
}

/// Inclusive source unit numbers (pages) covering the character range.
fn source_range(
    extraction: &Extraction,
    text: &str,
    start_char: usize,
    end_char: usize,
) -> Option<SourceRange> {
    let start_byte = char_to_byte(text, start_char)?;
    let end_byte = char_to_byte(text, end_char)?;
    if end_byte <= start_byte {
        return None;
    }

    let mut range: Option<SourceRange> = None;
    for unit in &extraction.units {
        let overlaps = unit
            .spans
            .iter()
            .any(|span| span.start < end_byte && span.end > start_byte);
        if overlaps {
            match &mut range {
                Some(range) => range.end = unit.number,
                None => {
                    range = Some(SourceRange {
                        start: unit.number,
                        end: unit.number,
                    })
                }
            }
        }
    }
    range
}

/// An inclusive range of source units (pages) in an extraction.
#[derive(Debug, Clone, Serialize)]
pub struct SourceRange {
    /// First source unit number.
    pub start: usize,
    /// Last source unit number.
    pub end: usize,
}

// ==================== search ====================

/// A bounded excerpt around the matching content of one document.
#[derive(Debug, Serialize)]
pub struct SearchExcerptResult {
    /// Excerpt text.
    pub content: String,
    /// Zero-based character offset of the excerpt in the extraction.
    pub offset: usize,
    /// Character offset one past the excerpt.
    pub next_offset: usize,
    /// Total character length of the extraction.
    pub total_length: usize,
    /// Source pages covered by the excerpt, when the extraction maps pages.
    pub source_range: Option<SourceRange>,
}

/// One ranked document search hit.
#[derive(Debug, Serialize)]
pub struct SearchHitResult {
    /// Name of the matched document.
    pub document_name: String,
    /// Document ID (use with `read_document`).
    pub document_id: String,
    /// Parser key of the extraction the content came from.
    pub extraction: String,
    /// Whether part of the document is outside the search index's coverage.
    pub partially_indexed: bool,
    /// Excerpt around the content match; `None` for a filename-only match.
    pub excerpt: Option<SearchExcerptResult>,
    /// Relevance score.
    pub score: f64,
}

/// Output of [`Search`].
#[derive(Debug, Serialize)]
pub struct SearchOutput {
    /// Total number of matching documents before the limit.
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
        "Keyword full-text search over whole documents in the collection. Returns up to 15 ranked documents with document IDs and an optional excerpt around the matching text. Use read_document with the document_id, extraction key and excerpt offset to read further.".to_string()
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

        let namespace_id = namespace(state)?;
        let index = &*state.search;
        let params = search::SearchParams {
            query: &query,
            limit: SEARCH_LIMIT,
            offset: 0,
            collection_id: &state.collection_id,
        };

        let results = search::search_index(index, params).map_err(infrastructure_error)?;
        let storage = state.storage.read().await;
        let mut hits = Vec::with_capacity(results.hits.len());

        for hit in results.hits {
            // Index records outlive a reversible delete until reconciliation
            // evicts them; the document's active state decides whether they
            // are searchable now.
            let active = storage
                .get_document(namespace_id, &hit.document_id)
                .await
                .map_err(infrastructure_error)?;
            if active.is_none() {
                continue;
            }

            let parser = parser_id(&hit.extraction)?;
            let extraction = storage
                .get_extraction(namespace_id, &hit.document_id, &parser)
                .await
                .map_err(infrastructure_error)?;
            let Some(extraction) = extraction else {
                // The index record points at an extraction that is no longer
                // selectable; reconciliation removes it.
                tracing::debug!(
                    document_id = %hit.document_id,
                    extraction = %hit.extraction,
                    "Skipping search hit with missing extraction"
                );
                continue;
            };

            let excerpt = hit.excerpt.map(|excerpt| SearchExcerptResult {
                source_range: source_range(
                    &extraction,
                    &extraction.text,
                    excerpt.offset,
                    excerpt.next_offset,
                ),
                content: excerpt.content,
                offset: excerpt.offset,
                next_offset: excerpt.next_offset,
                total_length: excerpt.total_length,
            });

            hits.push(SearchHitResult {
                document_name: hit.document_name,
                document_id: hit.document_id,
                extraction: hit.extraction,
                partially_indexed: hit.partially_indexed,
                excerpt,
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

// ==================== read_document ====================

/// Output of [`ReadDocument`].
#[derive(Debug, Serialize)]
pub struct ReadDocumentOutput {
    /// The document the extraction belongs to.
    pub document_id: String,
    /// Parser key of the extraction that was read.
    pub extraction: String,
    /// Bounded extraction text starting at `offset`.
    pub content: String,
    /// Zero-based character offset of `content` in the extraction.
    pub offset: usize,
    /// Character offset one past `content`.
    pub next_offset: usize,
    /// Total character length of the extraction.
    pub total_length: usize,
    /// Whether text exists before `offset`.
    pub has_more_before: bool,
    /// Whether text exists after `next_offset`.
    pub has_more_after: bool,
    /// Source pages covered by `content`, when the extraction maps pages.
    pub source_range: Option<SourceRange>,
}

/// Document-reading tool over the per-run collection scope.
pub struct ReadDocument {
    state: ToolState,
}

impl ReadDocument {
    pub fn new(state: ToolState) -> Self {
        Self { state }
    }
}

#[derive(Deserialize, schemars::JsonSchema)]
pub struct ReadDocumentArgs {
    /// The document ID from a search hit.
    pub document_id: String,
    /// The extraction parser key from the search hit.
    pub extraction: String,
    /// Zero-based character offset into the extraction.
    pub offset: usize,
    /// Maximum characters to return (default 4000, hard maximum 8000).
    #[serde(default)]
    pub max_chars: Option<usize>,
}

impl Tool for ReadDocument {
    const NAME: &'static str = "read_document";
    type Args = ReadDocumentArgs;
    type Output = ReadDocumentOutput;
    type Error = ToolExecutionError;

    fn description(&self) -> String {
        "Read a bounded passage of a whole document by document ID, extraction key and zero-based character offset. Returns at most 8000 characters (default 4000), with offsets and source pages for reading further.".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        args_schema::<ReadDocumentArgs>()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: ReadDocumentArgs,
    ) -> Result<ReadDocumentOutput, ToolExecutionError> {
        let state = &self.state;
        let document_id = args.document_id;
        let extraction_key = args.extraction;
        let offset = args.offset;
        let max_chars = args.max_chars.unwrap_or(DEFAULT_READ_CHARS);
        tracing::info!(
            document_id = %document_id,
            extraction = %extraction_key,
            offset,
            max_chars,
            "Reading document"
        );

        if max_chars == 0 || max_chars > MAX_READ_CHARS {
            return Err(ToolExecutionError::invalid_args(format!(
                "max_chars must be between 1 and {MAX_READ_CHARS}, got {max_chars}"
            )));
        }

        let namespace_id = namespace(state)?;
        let parser = parser_id(&extraction_key)?;
        let storage = state.storage.read().await;
        let extraction = storage
            .get_extraction(namespace_id, &document_id, &parser)
            .await
            .map_err(infrastructure_error)?
            .ok_or_else(|| {
                ToolExecutionError::not_found(format!(
                    "extraction {extraction_key} not found for document {document_id}"
                ))
            })?;

        let total_length = extraction.text.chars().count();
        if offset > total_length {
            return Err(ToolExecutionError::invalid_args(format!(
                "offset {offset} is beyond the extraction's {total_length} characters"
            )));
        }
        let next_offset = offset.saturating_add(max_chars).min(total_length);
        let start_byte = char_to_byte(&extraction.text, offset).ok_or_else(|| {
            ToolExecutionError::invalid_args(format!("offset {offset} is not a character boundary"))
        })?;
        let end_byte = char_to_byte(&extraction.text, next_offset).ok_or_else(|| {
            ToolExecutionError::invalid_args(format!(
                "offset {next_offset} is not a character boundary"
            ))
        })?;

        Ok(ReadDocumentOutput {
            document_id,
            extraction: extraction_key,
            content: extraction.text[start_byte..end_byte].to_string(),
            offset,
            next_offset,
            total_length,
            has_more_before: offset > 0,
            has_more_after: next_offset < total_length,
            source_range: source_range(&extraction, &extraction.text, offset, next_offset),
        })
    }
}

// ==================== list_documents ====================

/// One document entry.
#[derive(Debug, Serialize)]
pub struct DocumentEntry {
    /// Document file name.
    pub name: String,
    /// Document ID (use with read_document / search).
    pub id: String,
    /// Total page count, when the source is paginated.
    pub page_count: Option<usize>,
}

/// Output of [`ListDocuments`].
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

        let namespace_id = namespace(state)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::DocumentToIndex;
    use crate::text::{Extraction, ParserId, TextSpan, Unit, UnitKind};
    use milli::update::IndexerConfig;
    use rig::tool::{Tool, ToolContext, ToolErrorKind};

    const TEST_PARSER: &str = "test-parser@1";

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
    /// pages cover `text` up to the given end offsets. Returns the stored
    /// document id.
    async fn add_document(
        state: &ToolState,
        collection_id: &str,
        name: &str,
        text: &str,
        page_ends: Vec<usize>,
        unit_kind: Option<UnitKind>,
    ) -> String {
        let namespace_id = collection_id.parse::<iroh_docs::NamespaceId>().unwrap();
        let source = format!("source-{name}");
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

        doc_id
    }

    /// Index an already-stored document as a whole extraction.
    fn index_document(
        state: &ToolState,
        collection_id: &str,
        document_id: &str,
        name: &str,
        content: &str,
    ) {
        search::index_documents_batch(
            &state.search,
            &IndexerConfig::default(),
            vec![DocumentToIndex {
                id: search::document_external_id(collection_id, document_id),
                document_id: document_id.to_string(),
                document_name: name.to_string(),
                extraction: TEST_PARSER.to_string(),
                content: content.to_string(),
                collection_id: collection_id.to_string(),
            }],
        )
        .unwrap();
    }

    fn scope(id: &str) -> String {
        id.to_string()
    }

    async fn search(state: &ToolState, query: &str) -> Result<SearchOutput, ToolExecutionError> {
        Search::new(state.clone())
            .call(
                &mut ToolContext::new(),
                SearchArgs {
                    query: query.to_string(),
                },
            )
            .await
    }

    async fn read(
        state: &ToolState,
        document_id: &str,
        extraction: &str,
        offset: usize,
        max_chars: Option<usize>,
    ) -> Result<ReadDocumentOutput, ToolExecutionError> {
        ReadDocument::new(state.clone())
            .call(
                &mut ToolContext::new(),
                ReadDocumentArgs {
                    document_id: document_id.to_string(),
                    extraction: extraction.to_string(),
                    offset,
                    max_chars,
                },
            )
            .await
    }

    #[tokio::test]
    async fn test_search_empty_query_returns_no_hits() {
        let (_tmp, mut state) = create_state().await;
        state.collection_id = create_collection(&state, "Empty").await;

        let out = search(&state, "").await.unwrap();
        assert!(out.hits.is_empty());
    }

    #[tokio::test]
    async fn test_search_finds_indexed_document() {
        let (_tmp, mut state) = create_state().await;
        let content = "Global temperatures are rising due to greenhouse gases.";
        let collection = create_collection(&state, "Research Papers").await;
        let document_id = add_document(
            &state,
            &collection,
            "Climate_Report.pdf",
            content,
            vec![content.len()],
            Some(UnitKind::Page),
        )
        .await;
        index_document(
            &state,
            &collection,
            &document_id,
            "Climate_Report.pdf",
            content,
        );
        state.collection_id = scope(&collection);

        let out = search(&state, "climate temperatures").await.unwrap();

        assert!(!out.hits.is_empty());
        let hit = &out.hits[0];
        assert_eq!(hit.document_name, "Climate_Report.pdf");
        assert_eq!(hit.document_id, document_id);
        assert_eq!(hit.extraction, TEST_PARSER);
        assert!(!hit.partially_indexed);
        assert!(hit.score > 0.0);
        let excerpt = hit.excerpt.as_ref().expect("content match has an excerpt");
        assert!(excerpt.content.to_lowercase().contains("temperatures"));
        assert_eq!(excerpt.source_range.as_ref().unwrap().start, 1);
    }

    #[tokio::test]
    async fn test_search_filename_only_hit_has_no_excerpt() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Names").await;
        let document_id = add_document(
            &state,
            &collection,
            "Budget_Report.pdf",
            "Annual figures.",
            vec![14],
            Some(UnitKind::Page),
        )
        .await;
        index_document(
            &state,
            &collection,
            &document_id,
            "Budget_Report.pdf",
            "Annual figures.",
        );
        state.collection_id = scope(&collection);

        let out = search(&state, "Budget_Report").await.unwrap();
        assert_eq!(out.hits.len(), 1);
        assert!(out.hits[0].excerpt.is_none());
        assert_eq!(out.hits[0].extraction, TEST_PARSER);
    }

    #[tokio::test]
    async fn test_search_filters_by_collection() {
        let (_tmp, mut state) = create_state().await;
        let research = create_collection(&state, "Research").await;
        let finance = create_collection(&state, "Finance").await;
        let research_content = "Scientific research on climate change.";
        let finance_content = "Financial analysis for climate initiatives.";
        let research_id = add_document(
            &state,
            &research,
            "Research_Paper.pdf",
            research_content,
            vec![research_content.len()],
            Some(UnitKind::Page),
        )
        .await;
        let finance_id = add_document(
            &state,
            &finance,
            "Finance_Report.pdf",
            finance_content,
            vec![finance_content.len()],
            Some(UnitKind::Page),
        )
        .await;
        index_document(
            &state,
            &research,
            &research_id,
            "Research_Paper.pdf",
            research_content,
        );
        index_document(
            &state,
            &finance,
            &finance_id,
            "Finance_Report.pdf",
            finance_content,
        );
        state.collection_id = scope(&research);

        let out = search(&state, "climate").await.unwrap();

        let names: Vec<&str> = out.hits.iter().map(|h| h.document_name.as_str()).collect();
        assert!(names.contains(&"Research_Paper.pdf"));
        assert!(!names.contains(&"Finance_Report.pdf"));
    }

    #[tokio::test]
    async fn test_search_excerpt_is_bounded() {
        let (_tmp, mut state) = create_state().await;
        let long_content = "climate research findings ".repeat(30);
        let collection = create_collection(&state, "Long Docs").await;
        let document_id = add_document(
            &state,
            &collection,
            "Long.pdf",
            &long_content,
            vec![long_content.len()],
            Some(UnitKind::Page),
        )
        .await;
        index_document(&state, &collection, &document_id, "Long.pdf", &long_content);
        state.collection_id = scope(&collection);

        let out = search(&state, "climate").await.unwrap();

        let excerpt = out.hits[0].excerpt.as_ref().unwrap();
        assert!(excerpt.content.chars().count() <= 500);
        assert_eq!(excerpt.total_length, long_content.chars().count());
        assert_eq!(
            excerpt.content.chars().count(),
            excerpt.next_offset - excerpt.offset
        );
    }

    #[tokio::test]
    async fn test_read_document_success() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Documents").await;
        let content = "This is the content of the document with important information.";
        let document_id = add_document(
            &state,
            &collection,
            "Test_Document.pdf",
            content,
            vec![content.len()],
            Some(UnitKind::Page),
        )
        .await;
        state.collection_id = scope(&collection);

        let out = read(&state, &document_id, TEST_PARSER, 0, None)
            .await
            .unwrap();

        assert_eq!(out.document_id, document_id);
        assert_eq!(out.extraction, TEST_PARSER);
        assert_eq!(out.content, content);
        assert_eq!(out.offset, 0);
        assert_eq!(out.next_offset, content.chars().count());
        assert_eq!(out.total_length, content.chars().count());
        assert!(!out.has_more_before);
        assert!(!out.has_more_after);
        assert_eq!(out.source_range.as_ref().unwrap().start, 1);
    }

    #[tokio::test]
    async fn test_read_document_honours_max_chars_and_char_offsets() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Unicode").await;
        let content = "日本a語b";
        let document_id = add_document(
            &state,
            &collection,
            "Unicode.pdf",
            content,
            vec![content.len()],
            Some(UnitKind::Page),
        )
        .await;
        state.collection_id = scope(&collection);

        let out = read(&state, &document_id, TEST_PARSER, 2, Some(2))
            .await
            .unwrap();

        assert_eq!(out.content, "a語");
        assert_eq!(out.offset, 2);
        assert_eq!(out.next_offset, 4);
        assert_eq!(out.total_length, 5);
        assert!(out.has_more_before);
        assert!(out.has_more_after);
    }

    #[tokio::test]
    async fn test_read_document_reports_cross_page_range() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Paged").await;
        let document_id = add_document(
            &state,
            &collection,
            "Paged.pdf",
            "0123456789",
            vec![4, 8, 10],
            Some(UnitKind::Page),
        )
        .await;
        state.collection_id = scope(&collection);

        let out = read(&state, &document_id, TEST_PARSER, 3, Some(3))
            .await
            .unwrap();

        assert_eq!(out.content, "345");
        let range = out.source_range.expect("cross-page source range");
        assert_eq!((range.start, range.end), (1, 2));
    }

    #[tokio::test]
    async fn test_read_document_rejects_invalid_arguments() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Invalid").await;
        let document_id = add_document(
            &state,
            &collection,
            "Short.pdf",
            "short",
            vec![5],
            Some(UnitKind::Page),
        )
        .await;
        state.collection_id = scope(&collection);

        let err = read(&state, &document_id, TEST_PARSER, 99, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::InvalidArgs);
        assert!(err.to_string().contains("99"));

        let err = read(&state, &document_id, TEST_PARSER, 0, Some(0))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::InvalidArgs);

        let err = read(&state, &document_id, TEST_PARSER, 0, Some(8001))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::InvalidArgs);

        let err = read(&state, &document_id, "not-a-parser-key", 0, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::InvalidArgs);
    }

    #[tokio::test]
    async fn test_read_document_empty_extraction() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Empty Extraction").await;
        let document_id = add_document(
            &state,
            &collection,
            "Empty.pdf",
            "",
            vec![0],
            Some(UnitKind::Page),
        )
        .await;
        state.collection_id = scope(&collection);

        let out = read(&state, &document_id, TEST_PARSER, 0, None)
            .await
            .unwrap();

        assert_eq!(out.content, "");
        assert_eq!(out.total_length, 0);
        assert_eq!(out.next_offset, 0);
        assert!(!out.has_more_before);
        assert!(!out.has_more_after);
        assert!(out.source_range.is_none());
    }

    #[tokio::test]
    async fn test_read_document_rejects_extraction_outside_scope() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Owned").await;
        let document_id = add_document(
            &state,
            &collection,
            "Owned.pdf",
            "owned content",
            Vec::new(),
            None,
        )
        .await;
        let other = create_collection(&state, "Other").await;
        state.collection_id = scope(&other);

        let err = read(&state, &document_id, TEST_PARSER, 0, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::NotFound);
        assert!(err.to_string().contains(&document_id));
    }

    #[tokio::test]
    async fn test_deleted_documents_are_hidden_from_search_and_read_document() {
        let (_tmp, mut state) = create_state().await;
        let collection = create_collection(&state, "Reversible").await;
        let namespace_id = collection.parse::<iroh_docs::NamespaceId>().unwrap();
        let active_content = "climate temperatures rising over land";
        let deleted_content = "climate temperatures recorded at sea";
        let active_id = add_document(
            &state,
            &collection,
            "Active.pdf",
            active_content,
            vec![active_content.len()],
            Some(UnitKind::Page),
        )
        .await;
        let deleted_id = add_document(
            &state,
            &collection,
            "Deleted.pdf",
            deleted_content,
            vec![deleted_content.len()],
            Some(UnitKind::Page),
        )
        .await;
        index_document(
            &state,
            &collection,
            &active_id,
            "Active.pdf",
            active_content,
        );
        index_document(
            &state,
            &collection,
            &deleted_id,
            "Deleted.pdf",
            deleted_content,
        );
        state.collection_id = scope(&collection);

        state
            .storage
            .read()
            .await
            .delete_document(namespace_id, &deleted_id)
            .await
            .unwrap();

        // The deleted document's index record is still present here; the
        // tools must hide it before reconciliation evicts it.
        let deleted_record = search::document_external_id(&collection, &deleted_id);
        assert!(
            search::get_indexed_document_by_external_id(&state.search, &deleted_record)
                .unwrap()
                .is_some()
        );

        let out = search(&state, "climate").await.unwrap();
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.hits[0].document_id, active_id);
        assert!(!out.hits.iter().any(|hit| hit.document_id == deleted_id));

        let err = read(&state, &deleted_id, TEST_PARSER, 0, None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ToolErrorKind::NotFound);

        state
            .storage
            .read()
            .await
            .undelete_document(namespace_id, &deleted_id)
            .await
            .unwrap();

        let out = search(&state, "climate").await.unwrap();
        assert_eq!(out.total_hits, 2);
        let ids: Vec<&str> = out
            .hits
            .iter()
            .map(|hit| hit.document_id.as_str())
            .collect();
        assert!(ids.contains(&deleted_id.as_str()));

        let out = read(&state, &deleted_id, TEST_PARSER, 0, None)
            .await
            .unwrap();
        assert!(out.content.contains("recorded at sea"));
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
            "Paged.pdf",
            "0123456789",
            vec![4, 8, 10],
            Some(UnitKind::Page),
        )
        .await;
        add_document(
            &state,
            &collection,
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
    async fn tool_definitions_declare_required_params() {
        let (_tmp, state) = create_state().await;
        assert_eq!(
            Search::new(state.clone()).parameters()["required"],
            serde_json::json!(["query"])
        );

        assert_eq!(
            ReadDocument::new(state.clone()).parameters()["required"],
            serde_json::json!(["document_id", "extraction", "offset"])
        );
    }
}
