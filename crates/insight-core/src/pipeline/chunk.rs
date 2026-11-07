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

/// A chunk of a document's extracted text.
pub struct Chunk {
    /// The chunk text.
    pub text: String,
    /// UTF-8 byte offset in the input text where the chunk starts.
    pub start: usize,
    /// Byte offset one past the chunk's end.
    pub end: usize,
}

/// Split text into overlapping chunks.
///
/// Trims the input first; empty (or whitespace-only) input yields no
/// chunks. Offsets index into the untrimmed input, so
/// `&text[chunk.start..chunk.end] == chunk.text`.
pub fn chunk_text(text: &str) -> Vec<Chunk> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let base = text.len() - text.trim_start().len();
    SPLITTER
        .chunk_indices(trimmed)
        .map(|(start, chunk)| Chunk {
            text: chunk.to_string(),
            start: base + start,
            end: base + start + chunk.len(),
        })
        .collect()
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
    use crate::text::{Extraction, ParserId, TextSpan, Unit, UnitKind};

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

        /// Store a document's source and return its ID.
        async fn store_document(&self, metadata: DocumentMetadata) -> String {
            let doc_id = metadata.id.clone();
            self.storage
                .read()
                .await
                .add_document(
                    self.namespace_id,
                    metadata,
                    format!("source-{doc_id}").as_bytes(),
                )
                .await
                .unwrap();
            doc_id
        }

        /// Write an extraction for a document.
        async fn write_extraction(&self, doc_id: &str, extraction: &Extraction) {
            self.storage
                .read()
                .await
                .write_extraction(self.namespace_id, doc_id, extraction)
                .await
                .unwrap();
        }

        /// Select a document's canonical extraction.
        async fn set_canonical(&self, doc_id: &str, parser: &ParserId) {
            self.storage
                .read()
                .await
                .set_canonical_extraction(self.namespace_id, doc_id, parser)
                .await
                .unwrap();
        }

        /// Queue an index job without waiting for it to run.
        async fn queue_index(&self, doc_id: &str) {
            self.progress
                .queue(&self.namespace_id.to_string(), Stage::Index)
                .await;
            self.job_tx
                .send(IndexJob {
                    namespace_id: self.namespace_id,
                    doc_id: doc_id.to_string(),
                })
                .unwrap();
        }

        /// Run a document through the index worker and wait until that job
        /// reports the index stage complete.
        async fn run_index(&mut self, doc_id: &str) {
            let collection_id = self.namespace_id.to_string();
            let baseline = self
                .progress
                .get(&collection_id)
                .await
                .map(|progress| progress.index.completed)
                .unwrap_or(0);

            self.queue_index(doc_id).await;

            tokio::time::timeout(Duration::from_secs(30), async {
                loop {
                    match self.events_rx.recv().await {
                        Some(AppEvent::Progress(p)) if p.index.failed > 0 => {
                            panic!("index stage failed for {doc_id}")
                        }
                        Some(AppEvent::Progress(p)) if p.index.completed > baseline => break,
                        Some(_) => continue,
                        None => panic!("pipeline event channel closed"),
                    }
                }
            })
            .await
            .expect("index worker did not finish");
        }

        /// Store a document with its canonical extraction and index it.
        async fn index_document(&mut self, metadata: DocumentMetadata, extraction: Extraction) {
            let doc_id = self.store_document(metadata).await;
            self.write_extraction(&doc_id, &extraction).await;
            self.set_canonical(&doc_id, &extraction.parser).await;
            self.run_index(&doc_id).await;
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

    fn document(id: &str, name: &str) -> DocumentMetadata {
        DocumentMetadata {
            id: id.to_string(),
            name: name.to_string(),
            file_type: "application/pdf".to_string(),
            extraction: None,
            tags: Vec::new(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    fn parser(name: &str) -> ParserId {
        ParserId {
            name: name.to_string(),
            version: "1".to_string(),
        }
    }

    fn extraction(parser: ParserId, text: &str, page_ends: Vec<usize>) -> Extraction {
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
        Extraction {
            parser,
            text: text.to_string(),
            unit_kind: Some(UnitKind::Page),
            units,
        }
    }

    #[tokio::test]
    async fn test_index_worker_makes_documents_searchable_by_name() {
        let mut harness = IndexHarness::new().await;
        let text = "Global temperatures continue to rise across the decade.";
        harness
            .index_document(
                document("doc1", "Climate_Report_2024.pdf"),
                extraction(parser("test"), text, vec![text.len()]),
            )
            .await;

        let chunks = harness.search("Climate_Report");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].parent_name, "Climate_Report_2024.pdf");
    }

    #[tokio::test]
    async fn test_index_worker_records_chunk_spans() {
        let mut harness = IndexHarness::new().await;
        // Three pages of equal length, so chunk boundaries fall inside pages
        // and at least one chunk spans a page boundary.
        let page = "report ".repeat(170);
        let text = format!("{page}{page}{page}");
        let p1 = page.len();
        let p2 = page.len() * 2;
        let p3 = text.len();
        let name = "pages.pdf";
        harness
            .index_document(
                document("doc1", name),
                extraction(parser("test"), &text, vec![p1, p2, p3]),
            )
            .await;

        let mut chunks: Vec<IndexedChunk> = harness.search("report");
        chunks.sort_by_key(|c| c.chunk_index);

        assert!(chunks.len() > 1, "text must produce multiple chunks");
        let prefix = format!("[{name}]\n\n");
        for chunk in &chunks {
            let raw = chunk
                .content
                .strip_prefix(&prefix)
                .expect("indexed content must carry the document-name prefix");
            assert_eq!(&text[chunk.text_offset_start..chunk.text_offset_end], raw);
        }
        assert_eq!(chunks.first().unwrap().text_offset_start, 0);
    }

    #[tokio::test]
    async fn test_index_worker_ignores_non_canonical_extraction() {
        let mut harness = IndexHarness::new().await;
        let draft = extraction(parser("draft"), "alpha draft content", vec![19]);
        harness.store_document(document("doc1", "Draft.pdf")).await;
        harness.write_extraction("doc1", &draft).await;
        harness.queue_index("doc1").await;

        let sentinel_text = "sentinel report content";
        harness
            .index_document(
                document("doc2", "Sentinel.pdf"),
                extraction(parser("final"), sentinel_text, vec![sentinel_text.len()]),
            )
            .await;

        assert!(harness.search("alpha").is_empty());
        assert_eq!(harness.search("sentinel").len(), 1);
    }

    #[tokio::test]
    async fn test_index_worker_reindexes_when_canonical_extraction_changes() {
        let mut harness = IndexHarness::new().await;
        harness.store_document(document("doc1", "Report.pdf")).await;

        let first = extraction(parser("first"), "alpha findings", vec![14]);
        harness.write_extraction("doc1", &first).await;
        harness.set_canonical("doc1", &first.parser).await;
        harness.run_index("doc1").await;
        assert_eq!(harness.search("alpha").len(), 1);

        let second = extraction(parser("second"), "beta findings", vec![13]);
        harness.write_extraction("doc1", &second).await;
        harness.set_canonical("doc1", &second.parser).await;
        harness.run_index("doc1").await;

        assert!(harness.search("alpha").is_empty());
        assert_eq!(harness.search("beta").len(), 1);
    }

    #[test]
    fn test_chunk_text_short() {
        let chunks = chunk_text("Hello world. This is a test.");
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].text.contains("Hello world"));
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
    fn test_chunk_spans_slice_input_text() {
        let input = format!("  \n{}\n\n  ", "日本語のテキストです。".repeat(500));
        let chunks = chunk_text(&input);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(!chunk.text.is_empty());
            assert_eq!(&input[chunk.start..chunk.end], chunk.text);
        }
    }
}
