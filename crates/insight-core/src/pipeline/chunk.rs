//! Char-based text chunking.
//!
//! Splits extracted document text into overlapping chunks for keyword
//! indexing. Uses `text_splitter` with a char-based config (~1500 chars
//! per chunk, ~200 char overlap).

use std::sync::LazyLock;

const CHUNK_SIZE: usize = 1500;
const CHUNK_OVERLAP: usize = 200;

static SPLITTER: LazyLock<text_splitter::TextSplitter<text_splitter::Characters>> =
    LazyLock::new(|| {
        text_splitter::TextSplitter::new(
            text_splitter::ChunkConfig::new(CHUNK_SIZE)
                .with_overlap(CHUNK_OVERLAP)
                .expect("overlap must be smaller than chunk size"),
        )
    });

/// Split text into overlapping chunks.
///
/// Trims the input first; empty (or whitespace-only) input yields no
/// chunks.
pub fn chunk_text(text: &str) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    SPLITTER.chunks(trimmed).map(str::to_string).collect()
}

/// Find the starting byte offset of each chunk in the original text.
pub(crate) fn find_chunk_offsets(text: &str, chunks: &[String]) -> Vec<usize> {
    let mut offsets = Vec::with_capacity(chunks.len());
    let mut search_start_byte = 0;

    for chunk in chunks {
        if let Some(pos) = text[search_start_byte..].find(chunk.as_str()) {
            let absolute_byte = search_start_byte + pos;
            offsets.push(absolute_byte);
            search_start_byte = absolute_byte + chunk.len().saturating_sub(100);
            while search_start_byte < text.len() && !text.is_char_boundary(search_start_byte) {
                search_start_byte += 1;
            }
        } else {
            offsets.push(search_start_byte);
        }
    }

    offsets
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use milli::update::IndexerConfig;
    use tokio::sync::{mpsc, RwLock};

    use super::super::workers::{spawn_index_worker, SharedReceiver};
    use super::*;
    use crate::pipeline::{AppEvent, IndexJob, ProgressTracker, Stage};
    use crate::search::{get_indexed_chunk, search_index, IndexedChunk, SearchParams};
    use crate::storage::{DocumentMetadata, Storage};

    /// Drives the real pipeline index worker over a temporary storage and
    /// search index, the way the pipeline's watcher does.
    struct IndexHarness {
        storage: Arc<RwLock<Storage>>,
        index: Arc<milli::Index>,
        namespace_id: iroh_docs::NamespaceId,
        job_tx: mpsc::UnboundedSender<IndexJob>,
        events_rx: mpsc::UnboundedReceiver<AppEvent>,
        progress: ProgressTracker,
        _temp: tempfile::TempDir,
    }

    impl IndexHarness {
        async fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let storage = Storage::open(&temp.path().join("iroh")).await.unwrap();
            let storage = Arc::new(RwLock::new(storage));
            let index = Arc::new(crate::search::open_index(&temp.path().join("search")).unwrap());

            let index_worker =
                crate::search::spawn_index_worker(index.clone(), IndexerConfig::default());
            let (events_tx, events_rx) = mpsc::unbounded_channel();
            let progress = ProgressTracker::new(events_tx);
            let (job_tx, job_rx) = mpsc::unbounded_channel();
            spawn_index_worker(
                SharedReceiver::new_unbounded(job_rx),
                storage.clone(),
                index_worker,
                progress.clone(),
            );

            let (namespace_id, _) = storage
                .write()
                .await
                .create_collection("Research")
                .await
                .unwrap();

            Self {
                storage,
                index,
                namespace_id,
                job_tx,
                events_rx,
                progress,
                _temp: temp,
            }
        }

        /// Store a document, then run it through the index worker and wait
        /// until the worker reports the index stage complete.
        async fn index_document(&mut self, metadata: DocumentMetadata, text: &str) {
            let doc_id = metadata.id.clone();
            self.storage
                .read()
                .await
                .add_document(self.namespace_id, metadata, text.as_bytes(), b"source")
                .await
                .unwrap();

            self.progress
                .queue(&self.namespace_id.to_string(), Stage::Index)
                .await;
            self.job_tx
                .send(IndexJob {
                    namespace_id: self.namespace_id,
                    doc_id: doc_id.clone(),
                })
                .unwrap();

            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    match self.events_rx.recv().await {
                        Some(AppEvent::Progress(p)) if p.index.failed > 0 => {
                            panic!("index stage failed for {doc_id}")
                        }
                        Some(AppEvent::Progress(p)) if p.index.completed > 0 => break,
                        Some(_) => continue,
                        None => panic!("pipeline event channel closed"),
                    }
                }
            })
            .await
            .expect("index worker did not finish");
        }

        fn search(&self, query: &str) -> Vec<IndexedChunk> {
            let results = search_index(
                &self.index,
                SearchParams {
                    query,
                    limit: 100,
                    ..Default::default()
                },
            )
            .unwrap();
            let rtxn = self.index.read_txn().unwrap();
            results
                .hits
                .iter()
                .map(|hit| {
                    get_indexed_chunk(&self.index, &rtxn, hit.doc_id)
                        .unwrap()
                        .expect("hit must deserialize as an indexed chunk")
                })
                .collect()
        }
    }

    fn document(name: &str, page_boundaries: Vec<usize>, page_count: usize) -> DocumentMetadata {
        DocumentMetadata {
            id: "doc1".to_string(),
            name: name.to_string(),
            file_type: "application/pdf".to_string(),
            page_count,
            tags: Vec::new(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            page_boundaries,
        }
    }

    #[tokio::test]
    async fn test_index_worker_makes_documents_searchable_by_name() {
        let mut harness = IndexHarness::new().await;
        let text = "Global temperatures continue to rise across the decade.";
        harness
            .index_document(
                document("Climate_Report_2024.pdf", vec![text.len()], 1),
                text,
            )
            .await;

        let chunks = harness.search("Climate_Report");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].parent_name, "Climate_Report_2024.pdf");
    }

    #[tokio::test]
    async fn test_index_worker_maps_chunks_to_pages() {
        let mut harness = IndexHarness::new().await;
        // Three pages of equal length, so chunk boundaries fall inside pages
        // and at least one chunk spans a page boundary.
        let page = "report ".repeat(170);
        let text = format!("{page}{page}{page}");
        let p1 = page.len();
        let p2 = page.len() * 2;
        let p3 = text.len();
        harness
            .index_document(document("pages.pdf", vec![p1, p2, p3], 3), &text)
            .await;

        let mut chunks: Vec<IndexedChunk> = harness.search("report");
        chunks.sort_by_key(|c| c.chunk_index);

        assert!(chunks.len() > 1, "text must produce multiple chunks");
        assert_eq!(chunks.first().unwrap().start_page, 1);
        assert_eq!(chunks.last().unwrap().end_page, 3);
        assert!(
            chunks.iter().any(|c| c.start_page == 1 && c.end_page == 2),
            "expected a chunk spanning pages 1-2: {:?}",
            chunks
                .iter()
                .map(|c| (c.start_page, c.end_page))
                .collect::<Vec<_>>()
        );
        assert!(
            chunks.iter().any(|c| c.start_page == 2 && c.end_page == 3),
            "expected a chunk spanning pages 2-3: {:?}",
            chunks
                .iter()
                .map(|c| (c.start_page, c.end_page))
                .collect::<Vec<_>>()
        );
        for chunk in &chunks {
            assert!(chunk.start_page <= chunk.end_page);
        }
    }

    #[test]
    fn test_chunk_text_short() {
        let chunks = chunk_text("Hello world. This is a test.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Hello world"));
    }

    #[test]
    fn test_chunk_text_empty() {
        assert!(chunk_text("").is_empty());
        assert!(chunk_text("   \n\t  ").is_empty());
    }

    #[test]
    fn test_chunk_text_long() {
        let text = "word ".repeat(2000); // ~10k chars
        let chunks = chunk_text(&text);
        assert!(chunks.len() > 1);
    }

    #[test]
    fn test_chunk_text_utf8_safety() {
        let input = "日本語のテキストです。".repeat(500);
        let trimmed = input.trim();
        let chunks = chunk_text(&input);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(!chunk.is_empty());
            assert!(
                trimmed.contains(chunk.as_str()),
                "chunk is not a substring of the input: {chunk:?}"
            );
        }
    }
}
