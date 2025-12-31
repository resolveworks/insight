use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use super::AgentContext;
use crate::search;

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Result of executing a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
}

/// Execute a tool call and return the result
pub async fn execute_tool(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    match tool_call.name.as_str() {
        "search" => execute_search(tool_call, ctx).await,
        "read_chunk" => execute_read_chunk(tool_call, ctx).await,
        "list_documents" => execute_list_documents(tool_call, ctx).await,
        "get_collection_terms" => execute_get_collection_terms(tool_call, ctx).await,
        _ => ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: format!("Unknown tool: {}", tool_call.name),
            is_error: true,
        },
    }
}

/// Hybrid search combining keyword (BM25) and semantic matching.
/// Falls back to keyword-only if no embedder is configured.
async fn execute_search(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    let query = tool_call.arguments["query"].as_str().unwrap_or("");

    info!(query = %query, "Executing hybrid search");

    // Try to get query embedding for semantic component
    let embedder_guard = ctx.state.embedder.read().await;
    let (query_vector, semantic_ratio) = match embedder_guard.as_ref() {
        Some(embedder) => match embedder.embed(query).await {
            Ok(vec) => {
                debug!(dimensions = vec.len(), "Query embedded for hybrid search");
                (Some(vec), 0.6) // 60% semantic, 40% keyword
            }
            Err(e) => {
                warn!(error = %e, "Failed to embed query, using keyword-only search");
                (None, 0.0)
            }
        },
        None => {
            debug!("No embedder configured, using keyword-only search");
            (None, 0.0)
        }
    };
    drop(embedder_guard);

    let index = &*ctx.state.search;
    let collection_ids = ctx.collection_ids();

    let search_params = search::SearchParams {
        query,
        limit: 15,
        query_vector,
        semantic_ratio,
        min_score: if semantic_ratio > 0.0 {
            Some(0.15)
        } else {
            None
        },
        collection_ids: collection_ids.as_deref(),
        ..Default::default()
    };

    match search::search_index(index, search_params) {
        Ok(results) => {
            info!(
                query = %query,
                hits = results.hits.len(),
                hybrid = semantic_ratio > 0.0,
                "Search completed"
            );
            let formatted = format_search_results(index, &results.hits, ctx);
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: formatted,
                is_error: false,
            }
        }
        Err(e) => {
            warn!(query = %query, error = %e, "Search failed");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Search error: {}", e),
                is_error: true,
            }
        }
    }
}

fn format_search_results(
    index: &milli::Index,
    hits: &[search::SearchHit],
    ctx: &AgentContext,
) -> String {
    if hits.is_empty() {
        return "No matching passages found.".to_string();
    }

    // Build a lookup map from collection_id -> collection_name
    let collection_names: std::collections::HashMap<String, String> = ctx
        .collections
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let mut results = Vec::new();
    let rtxn = match index.read_txn() {
        Ok(t) => t,
        Err(e) => return format!("Error reading index: {}", e),
    };

    for hit in hits {
        let doc = match search::get_document(index, &rtxn, hit.doc_id) {
            Ok(Some(d)) => d,
            _ => continue,
        };

        let score = search::compute_hit_score(&hit.scores);

        let get_str = |key: &str| -> String {
            doc.get(key)
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        let get_num =
            |key: &str| -> i64 { doc.get(key).and_then(|v| v.as_i64()).unwrap_or_default() };

        let parent_id = get_str("parent_id");
        let parent_name = get_str("parent_name");
        let chunk_index = get_num("chunk_index");
        let page_count = get_num("page_count");
        let start_page = get_num("start_page");
        let end_page = get_num("end_page");
        let collection_id = get_str("collection_id");
        let content = get_str("content");

        // Look up collection name, fall back to ID if not found
        let collection_name = collection_names
            .get(&collection_id)
            .cloned()
            .unwrap_or_else(|| collection_id.chars().take(8).collect::<String>() + "...");

        // Truncate long passages
        let passage: String = content.chars().take(500).collect();
        let passage = if content.len() > 500 {
            format!("{}...", passage)
        } else {
            passage
        };

        // Format page reference - show specific page(s) if available, otherwise total page count
        let page_ref = if start_page > 0 && end_page > 0 {
            if start_page == end_page {
                format!("p. {}", start_page)
            } else {
                format!("pp. {}-{}", start_page, end_page)
            }
        } else if page_count == 1 {
            "1 page".to_string()
        } else {
            format!("{} pages", page_count)
        };

        results.push(format!(
            "- Document: {} ({}) [score: {:.2}]\n  Collection: {}\n  ID: {} | Chunk: {}\n  Passage: {}",
            parent_name, page_ref, score, collection_name, parent_id, chunk_index, passage
        ));
    }

    format!(
        "Found {} relevant passages:\n\n{}",
        hits.len(),
        results.join("\n\n")
    )
}

async fn execute_read_chunk(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    let doc_id = tool_call.arguments["document_id"].as_str().unwrap_or("");
    let chunk_index = tool_call.arguments["chunk_index"].as_u64().unwrap_or(0) as usize;

    info!(document_id = %doc_id, chunk_index = chunk_index, "Reading chunk");

    // Build the chunk ID: "{parent_id}_chunk_{chunk_index}"
    let chunk_id = format!("{}_chunk_{}", doc_id, chunk_index);

    let index = &*ctx.state.search;

    match search::get_document_by_external_id(index, &chunk_id) {
        Ok(Some(content)) => {
            info!(
                document_id = %doc_id,
                chunk_index = chunk_index,
                content_len = content.len(),
                "Chunk read successfully"
            );

            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content,
                is_error: false,
            }
        }
        Ok(None) => {
            warn!(document_id = %doc_id, chunk_index = chunk_index, "Chunk not found");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!(
                    "Chunk {} not found for document {}. Try a different chunk index.",
                    chunk_index, doc_id
                ),
                is_error: true,
            }
        }
        Err(e) => {
            warn!(document_id = %doc_id, chunk_index = chunk_index, error = %e, "Error reading chunk");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error reading chunk: {}", e),
                is_error: true,
            }
        }
    }
}

async fn execute_list_documents(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    info!("Listing documents");

    // Get collection IDs to list from
    let collection_ids = ctx.collection_ids();

    if collection_ids.is_none()
        || collection_ids
            .as_ref()
            .map(|c| c.is_empty())
            .unwrap_or(true)
    {
        return ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: "No collections selected. Please select collections to list documents from."
                .to_string(),
            is_error: true,
        };
    }

    let collection_ids = collection_ids.unwrap();

    // Build a lookup map from collection_id -> collection_name
    let collection_names: std::collections::HashMap<String, String> = ctx
        .collections
        .as_ref()
        .map(|cols| {
            cols.iter()
                .map(|c| (c.id.clone(), c.name.clone()))
                .collect()
        })
        .unwrap_or_default();

    let storage = ctx.state.storage.read().await;
    let mut all_documents = Vec::new();

    for collection_id in &collection_ids {
        let namespace_id: iroh_docs::NamespaceId = match collection_id.parse() {
            Ok(id) => id,
            Err(_) => {
                warn!(collection_id = %collection_id, "Invalid collection ID, skipping");
                continue;
            }
        };

        let collection_name = collection_names
            .get(collection_id)
            .cloned()
            .unwrap_or_else(|| collection_id.chars().take(8).collect::<String>() + "...");

        match storage.list_documents(namespace_id).await {
            Ok(documents) => {
                for doc in documents {
                    all_documents.push((collection_name.clone(), doc));
                }
            }
            Err(e) => {
                warn!(collection_id = %collection_id, error = %e, "Failed to list documents");
            }
        }
    }

    drop(storage);

    if all_documents.is_empty() {
        return ToolResult {
            tool_call_id: tool_call.id.clone(),
            content: "No documents found in the selected collections.".to_string(),
            is_error: false,
        };
    }

    // Sort by collection name, then document name
    all_documents.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.name.cmp(&b.1.name)));

    let total_docs = all_documents.len();
    let total_pages: usize = all_documents.iter().map(|(_, d)| d.page_count).sum();

    // Limit output to avoid overwhelming the model
    const MAX_DOCS_TO_SHOW: usize = 25;
    let showing_all = total_docs <= MAX_DOCS_TO_SHOW;
    let docs_to_show: Vec<_> = all_documents.iter().take(MAX_DOCS_TO_SHOW).collect();

    // Format results
    let mut results = Vec::new();
    let mut current_collection = String::new();

    for (collection_name, doc) in docs_to_show {
        // Add collection header when it changes
        if *collection_name != current_collection {
            if !current_collection.is_empty() {
                results.push(String::new()); // Blank line between collections
            }
            results.push(format!("## {}", collection_name));
            current_collection = collection_name.clone();
        }

        // Format page info
        let page_info = if doc.page_count == 1 {
            "1p".to_string()
        } else {
            format!("{}p", doc.page_count)
        };

        // Full ID needed for read_chunk tool
        results.push(format!("- {} ({}) [{}]", doc.name, page_info, doc.id));
    }

    let summary = if showing_all {
        format!(
            "Found {} document{} ({} total pages):\n\n{}",
            total_docs,
            if total_docs == 1 { "" } else { "s" },
            total_pages,
            results.join("\n")
        )
    } else {
        format!(
            "Found {} documents ({} total pages). Showing first {}:\n\n{}\n\n... and {} more documents. Use search to find specific documents.",
            total_docs,
            total_pages,
            MAX_DOCS_TO_SHOW,
            results.join("\n"),
            total_docs - MAX_DOCS_TO_SHOW
        )
    };

    info!(
        document_count = total_docs,
        total_pages = total_pages,
        "Listed documents"
    );

    ToolResult {
        tool_call_id: tool_call.id.clone(),
        content: summary,
        is_error: false,
    }
}

/// Get common terms in the collection(s) to understand what topics are covered
async fn execute_get_collection_terms(tool_call: &ToolCall, ctx: &AgentContext) -> ToolResult {
    // Parse limit parameter (default 50, max 200)
    let limit = tool_call.arguments["limit"].as_u64().unwrap_or(50).min(200) as usize;

    info!(limit = limit, "Getting collection terms");

    let collection_ids = ctx.collection_ids();
    let index = &*ctx.state.search;

    match search::get_collection_terms(index, collection_ids.as_deref(), limit) {
        Ok(terms) => {
            if terms.is_empty() {
                return ToolResult {
                    tool_call_id: tool_call.id.clone(),
                    content: "No terms found in the collection(s). The collection may be empty."
                        .to_string(),
                    is_error: false,
                };
            }

            // Format terms as a readable list
            let mut output = format!("Top {} terms by document frequency:\n\n", terms.len());

            for term in &terms {
                output.push_str(&format!("- {} ({} docs)\n", term.term, term.doc_count));
            }

            info!(term_count = terms.len(), "Retrieved collection terms");

            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: output,
                is_error: false,
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to get collection terms");
            ToolResult {
                tool_call_id: tool_call.id.clone(),
                content: format!("Error getting collection terms: {}", e),
                is_error: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::{self, ChunkToIndex};
    use crate::AppState;
    use crate::CollectionInfo;
    use milli::update::IndexerConfig;

    /// Create a minimal AppState for testing (no storage operations)
    async fn create_test_state() -> AppState {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };

        // Create directories
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        std::fs::create_dir_all(&config.search_dir).unwrap();

        let (state, _progress_rx) = AppState::new(config).await.unwrap();
        state
    }

    fn test_indexer_config() -> IndexerConfig {
        IndexerConfig::default()
    }

    /// Helper to create a chunk for indexing
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
            vector: None,
        }
    }

    // ==================== ToolCall / ToolResult Tests ====================

    #[test]
    fn test_tool_call_serialization() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "climate report"}),
        };

        let json = serde_json::to_string(&tool_call).unwrap();
        let parsed: ToolCall = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, "call_123");
        assert_eq!(parsed.name, "search");
        assert_eq!(parsed.arguments["query"], "climate report");
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult {
            tool_call_id: "call_123".to_string(),
            content: "Found 5 documents".to_string(),
            is_error: false,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool_call_id, "call_123");
        assert_eq!(parsed.content, "Found 5 documents");
        assert!(!parsed.is_error);
    }

    #[test]
    fn test_tool_result_error_serialization() {
        let result = ToolResult {
            tool_call_id: "call_456".to_string(),
            content: "Search error: index not found".to_string(),
            is_error: true,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert!(parsed.is_error);
        assert!(parsed.content.contains("error"));
    }

    // ==================== execute_tool Tests ====================

    #[tokio::test]
    async fn test_execute_tool_unknown_tool() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_unknown".to_string(),
            name: "nonexistent_tool".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = execute_tool(&tool_call, &ctx).await;

        assert!(result.is_error);
        assert_eq!(result.tool_call_id, "call_unknown");
        assert!(result.content.contains("Unknown tool"));
        assert!(result.content.contains("nonexistent_tool"));
    }

    #[tokio::test]
    async fn test_execute_tool_dispatches_to_search() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_search".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test query"}),
        };

        let result = execute_tool(&tool_call, &ctx).await;

        // Should not error (though may find no results)
        assert!(!result.is_error);
        assert_eq!(result.tool_call_id, "call_search");
    }

    #[tokio::test]
    async fn test_execute_tool_dispatches_to_read_chunk() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_read".to_string(),
            name: "read_chunk".to_string(),
            arguments: serde_json::json!({"document_id": "doc123", "chunk_index": 0}),
        };

        let result = execute_tool(&tool_call, &ctx).await;

        // Should error because chunk doesn't exist
        assert!(result.is_error);
        assert_eq!(result.tool_call_id, "call_read");
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_execute_tool_dispatches_to_list_documents() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None, // No collections selected
        };

        let tool_call = ToolCall {
            id: "call_list".to_string(),
            name: "list_documents".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = execute_tool(&tool_call, &ctx).await;

        // Should error because no collections selected
        assert!(result.is_error);
        assert_eq!(result.tool_call_id, "call_list");
        assert!(result.content.contains("No collections selected"));
    }

    // ==================== execute_search Tests ====================

    #[tokio::test]
    async fn test_search_empty_query() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({}), // Missing query
        };

        let result = execute_search(&tool_call, &ctx).await;

        // Empty query should still work (returns no results)
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_search_with_indexed_documents() {
        let state = create_test_state().await;

        // Index some test documents
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunks = vec![
                make_chunk(
                    "doc1",
                    "Climate_Report.pdf",
                    "Global temperatures are rising due to greenhouse gases.",
                    "research",
                    0,
                    10,
                    1,
                    1,
                ),
                make_chunk(
                    "doc2",
                    "Financial_Summary.pdf",
                    "Q4 revenue exceeded expectations with strong growth.",
                    "finance",
                    0,
                    5,
                    1,
                    1,
                ),
            ];
            search::index_chunks_batch(index, &config, chunks).unwrap();
        }

        let ctx = AgentContext {
            state,
            collections: Some(vec![CollectionInfo {
                id: "research".to_string(),
                name: "Research Papers".to_string(),
                document_count: 1,
                total_pages: 10,
                created_at: None,
            }]),
        };

        let tool_call = ToolCall {
            id: "call_search".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "climate temperatures"}),
        };

        let result = execute_search(&tool_call, &ctx).await;

        assert!(!result.is_error);
        assert!(result.content.contains("Climate_Report.pdf"));
        assert!(result.content.contains("relevant passages"));
    }

    #[tokio::test]
    async fn test_search_no_results() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "xyznonexistent123"}),
        };

        let result = execute_search(&tool_call, &ctx).await;

        assert!(!result.is_error);
        assert!(result.content.contains("No matching passages"));
    }

    #[tokio::test]
    async fn test_search_filters_by_collection() {
        let state = create_test_state().await;

        // Index documents in different collections
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunks = vec![
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
            ];
            search::index_chunks_batch(&index, &config, chunks).unwrap();
        }

        // Only search in research collection
        let ctx = AgentContext {
            state,
            collections: Some(vec![CollectionInfo {
                id: "research_col".to_string(),
                name: "Research".to_string(),
                document_count: 1,
                total_pages: 10,
                created_at: None,
            }]),
        };

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "climate"}),
        };

        let result = execute_search(&tool_call, &ctx).await;

        assert!(!result.is_error);
        // Should find research paper but not finance report
        assert!(result.content.contains("Research_Paper.pdf"));
        assert!(!result.content.contains("Finance_Report.pdf"));
    }

    // ==================== execute_read_chunk Tests ====================

    #[tokio::test]
    async fn test_read_chunk_not_found() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_read".to_string(),
            name: "read_chunk".to_string(),
            arguments: serde_json::json!({
                "document_id": "nonexistent_doc",
                "chunk_index": 0
            }),
        };

        let result = execute_read_chunk(&tool_call, &ctx).await;

        assert!(result.is_error);
        assert!(result.content.contains("not found"));
        assert!(result.content.contains("nonexistent_doc"));
    }

    #[tokio::test]
    async fn test_read_chunk_success() {
        let state = create_test_state().await;

        // Index a document with content
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunks = vec![make_chunk(
                "test_doc",
                "Test_Document.pdf",
                "This is the content of chunk 0 with important information.",
                "collection1",
                0,
                5,
                1,
                1,
            )];
            search::index_chunks_batch(index, &config, chunks).unwrap();
        }

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_read".to_string(),
            name: "read_chunk".to_string(),
            arguments: serde_json::json!({
                "document_id": "test_doc",
                "chunk_index": 0
            }),
        };

        let result = execute_read_chunk(&tool_call, &ctx).await;

        assert!(!result.is_error);
        assert!(result.content.contains("important information"));
    }

    #[tokio::test]
    async fn test_read_chunk_wrong_index() {
        let state = create_test_state().await;

        // Index only chunk 0
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunks = vec![make_chunk(
                "test_doc",
                "Test.pdf",
                "Content of chunk 0",
                "col1",
                0,
                1,
                1,
                1,
            )];
            search::index_chunks_batch(index, &config, chunks).unwrap();
        }

        let ctx = AgentContext {
            state,
            collections: None,
        };

        // Try to read chunk 5 which doesn't exist
        let tool_call = ToolCall {
            id: "call_read".to_string(),
            name: "read_chunk".to_string(),
            arguments: serde_json::json!({
                "document_id": "test_doc",
                "chunk_index": 5
            }),
        };

        let result = execute_read_chunk(&tool_call, &ctx).await;

        assert!(result.is_error);
        assert!(result.content.contains("not found"));
        // Error message format: "Chunk 5 not found for document test_doc"
        assert!(result.content.contains("Chunk 5"));
    }

    #[tokio::test]
    async fn test_read_chunk_missing_arguments() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        // Missing document_id
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "read_chunk".to_string(),
            arguments: serde_json::json!({"chunk_index": 0}),
        };

        let result = execute_read_chunk(&tool_call, &ctx).await;

        // Should handle gracefully (empty doc_id)
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    // ==================== execute_list_documents Tests ====================

    #[tokio::test]
    async fn test_list_documents_no_collections() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_list".to_string(),
            name: "list_documents".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = execute_list_documents(&tool_call, &ctx).await;

        assert!(result.is_error);
        assert!(result.content.contains("No collections selected"));
    }

    #[tokio::test]
    async fn test_list_documents_empty_collections() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: Some(vec![]), // Empty list
        };

        let tool_call = ToolCall {
            id: "call_list".to_string(),
            name: "list_documents".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = execute_list_documents(&tool_call, &ctx).await;

        assert!(result.is_error);
        assert!(result.content.contains("No collections selected"));
    }

    // ==================== format_search_results Tests ====================

    #[tokio::test]
    async fn test_format_search_results_empty() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = search::open_index(temp_dir.path()).unwrap();

        let config = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&config.iroh_dir).unwrap();
        let (state, _progress_rx) = AppState::new(config).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let hits: Vec<search::SearchHit> = vec![];
        let result = format_search_results(&index, &hits, &ctx);

        assert_eq!(result, "No matching passages found.");
    }

    #[tokio::test]
    async fn test_format_search_results_with_collection_names() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = search::open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Index a document
        let chunks = vec![make_chunk(
            "doc1",
            "Report.pdf",
            "Important findings about the topic.",
            "col_123",
            0,
            10,
            1,
            1,
        )];
        search::index_chunks_batch(&index, &config, chunks).unwrap();

        // Search to get a hit
        let results = search::search_index(
            &index,
            search::SearchParams {
                query: "findings",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();

        let cfg = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search2"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&cfg.iroh_dir).unwrap();
        std::fs::create_dir_all(&cfg.search_dir).unwrap();
        let (state, _progress_rx) = AppState::new(cfg).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: Some(vec![CollectionInfo {
                id: "col_123".to_string(),
                name: "Research Collection".to_string(),
                document_count: 1,
                total_pages: 10,
                created_at: None,
            }]),
        };

        let formatted = format_search_results(&index, &results.hits, &ctx);

        assert!(formatted.contains("Report.pdf"));
        assert!(formatted.contains("Research Collection"));
        assert!(formatted.contains("relevant passages"));
    }

    #[tokio::test]
    async fn test_format_search_results_truncates_long_content() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = search::open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Create content longer than 500 chars with real words for milli to tokenize
        let long_content = "climate research findings ".repeat(30); // ~780 chars
        let chunks = vec![make_chunk(
            "doc1",
            "Long.pdf",
            &long_content,
            "col1",
            0,
            1,
            1,
            1,
        )];
        search::index_chunks_batch(&index, &config, chunks).unwrap();

        let results = search::search_index(
            &index,
            search::SearchParams {
                query: "climate",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();

        assert!(!results.hits.is_empty(), "Search should find the document");

        let cfg = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search2"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&cfg.iroh_dir).unwrap();
        std::fs::create_dir_all(&cfg.search_dir).unwrap();
        let (state, _progress_rx) = AppState::new(cfg).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let formatted = format_search_results(&index, &results.hits, &ctx);

        // Should be truncated with "..."
        assert!(
            formatted.contains("..."),
            "Long content should be truncated. Got: {}",
            formatted
        );
        // Should not contain full 780 chars of content
        assert!(formatted.len() < 1000);
    }

    #[tokio::test]
    async fn test_format_search_results_page_references() {
        let temp_dir = tempfile::tempdir().unwrap();
        let index = search::open_index(temp_dir.path()).unwrap();
        let config = test_indexer_config();

        // Test different page scenarios
        let chunks = vec![
            // Single page reference
            make_chunk("doc1", "Single.pdf", "Content A", "col1", 0, 10, 5, 5),
            // Page range
            make_chunk("doc2", "Range.pdf", "Content B", "col1", 0, 20, 3, 7),
        ];
        search::index_chunks_batch(&index, &config, chunks).unwrap();

        let results = search::search_index(
            &index,
            search::SearchParams {
                query: "Content",
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();

        let cfg = crate::Config {
            data_dir: temp_dir.path().to_path_buf(),
            iroh_dir: temp_dir.path().join("iroh"),
            search_dir: temp_dir.path().join("search2"),
            settings_file: temp_dir.path().join("settings.json"),
            conversations_dir: temp_dir.path().join("conversations"),
        };
        std::fs::create_dir_all(&cfg.iroh_dir).unwrap();
        std::fs::create_dir_all(&cfg.search_dir).unwrap();
        let (state, _progress_rx) = AppState::new(cfg).await.unwrap();

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let formatted = format_search_results(&index, &results.hits, &ctx);

        // Should contain page references
        assert!(formatted.contains("p. 5") || formatted.contains("pp. 3-7"));
    }

    // ==================== execute_get_collection_terms Tests ====================

    #[tokio::test]
    async fn test_get_collection_terms_empty_index() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_terms".to_string(),
            name: "get_collection_terms".to_string(),
            arguments: serde_json::json!({}),
        };

        let result = execute_get_collection_terms(&tool_call, &ctx).await;

        assert!(!result.is_error);
        assert!(result.content.contains("No terms found"));
    }

    #[tokio::test]
    async fn test_get_collection_terms_with_indexed_documents() {
        let state = create_test_state().await;

        // Index some test documents
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunks = vec![
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
            ];
            search::index_chunks_batch(index, &config, chunks).unwrap();
        }

        let ctx = AgentContext {
            state,
            collections: None, // All collections
        };

        let tool_call = ToolCall {
            id: "call_terms".to_string(),
            name: "get_collection_terms".to_string(),
            arguments: serde_json::json!({"limit": 10}),
        };

        let result = execute_get_collection_terms(&tool_call, &ctx).await;

        assert!(!result.is_error);
        assert!(result.content.contains("Top"));
        assert!(result.content.contains("terms by document frequency"));
        // "climate" appears in both documents
        assert!(result.content.contains("climate"));
    }

    #[tokio::test]
    async fn test_get_collection_terms_respects_limit() {
        let state = create_test_state().await;

        // Index a document with many terms
        {
            let index = &*state.search;
            let config = test_indexer_config();

            let chunk = make_chunk(
                "doc1",
                "varied.pdf",
                "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu nu",
                "col1",
                0,
                1,
                1,
                1,
            );
            search::index_chunks_batch(index, &config, vec![chunk]).unwrap();
        }

        let ctx = AgentContext {
            state,
            collections: None,
        };

        let tool_call = ToolCall {
            id: "call_terms".to_string(),
            name: "get_collection_terms".to_string(),
            arguments: serde_json::json!({"limit": 3}),
        };

        let result = execute_get_collection_terms(&tool_call, &ctx).await;

        assert!(!result.is_error);
        // Count how many terms are listed (each term is on its own line starting with "- ")
        let term_count = result
            .content
            .lines()
            .filter(|l| l.starts_with("- "))
            .count();
        assert!(
            term_count <= 3,
            "Should return at most 3 terms, got {}",
            term_count
        );
    }

    #[tokio::test]
    async fn test_get_collection_terms_max_limit_enforced() {
        let state = create_test_state().await;
        let ctx = AgentContext {
            state,
            collections: None,
        };

        // Request more than max allowed (200)
        let tool_call = ToolCall {
            id: "call_terms".to_string(),
            name: "get_collection_terms".to_string(),
            arguments: serde_json::json!({"limit": 500}),
        };

        let result = execute_get_collection_terms(&tool_call, &ctx).await;

        // Should not error - limit is capped to 200 internally
        assert!(!result.is_error);
    }
}
