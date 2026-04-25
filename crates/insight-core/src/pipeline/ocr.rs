//! OCR worker: turns `files/{id}/ocr_task` entries into final `text`.
//!
//! For each job:
//! 1. Wait for research focus to release (chat preempts ingest).
//! 2. Read the parked task + the PDF source from iroh.
//! 3. Acquire the OCR provider lease — fails fast if no model is
//!    configured, leaving the task entry in place for retry.
//! 4. Rasterize each `NeedsOcr` page on the blocking pool.
//! 5. Run inference, then merge per-page results with the digital text
//!    we already had into the final `(text, page_boundaries)` pair.
//! 6. Write `text` + updated meta, then delete the `ocr_task` entry.
//!
//! Per-doc failures emit `ProgressUpdate::Failed` and leave `ocr_task` in
//! place — the next startup orphan scan retries.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::manager::ModelManager;
use crate::pdf::{rasterize_page, OcrTask, PageDecision, RASTER_DPI};
use crate::storage::Storage;

use super::progress::ProgressTracker;
use super::types::{OcrJob, ProgressUpdate, Stage};
use super::workers::SharedReceiver;

/// Spawn the OCR worker pool.
///
/// `count` should be 1 in production — OCR is GPU-bound under
/// `coexist=false`, so a second worker would only contend for the lease.
/// The parameter exists for tests that want concurrency disabled
/// differently.
pub fn spawn_ocr_workers(
    count: usize,
    rx: SharedReceiver<OcrJob>,
    storage: Arc<RwLock<Storage>>,
    models: Arc<ModelManager>,
    progress: ProgressTracker,
) {
    for i in 0..count {
        let rx = rx.clone();
        let storage = storage.clone();
        let models = models.clone();
        let progress = progress.clone();
        let mut focus_guard = models.focus_guard();

        tokio::spawn(async move {
            tracing::debug!(worker = i, "OCR worker started");
            while let Some(job) = rx.recv().await {
                focus_guard.wait_until_released().await;
                let collection_id = job.namespace_id.to_string();

                progress
                    .apply(ProgressUpdate::Started {
                        collection_id: collection_id.clone(),
                        stage: Stage::Ocr,
                    })
                    .await;

                match run_ocr_job(&job, &storage, &models).await {
                    Ok(()) => {
                        progress
                            .apply(ProgressUpdate::Completed {
                                collection_id,
                                stage: Stage::Ocr,
                            })
                            .await;
                    }
                    Err(e) => {
                        tracing::error!(
                            doc_id = %job.doc_id,
                            error = %e,
                            "OCR failed; leaving ocr_task in place for retry",
                        );
                        progress
                            .apply(ProgressUpdate::Failed {
                                collection_id,
                                stage: Stage::Ocr,
                                error: e.to_string(),
                            })
                            .await;
                    }
                }
            }
            tracing::debug!(worker = i, "OCR worker stopped");
        });
    }
}

async fn run_ocr_job(
    job: &OcrJob,
    storage: &Arc<RwLock<Storage>>,
    models: &Arc<ModelManager>,
) -> anyhow::Result<()> {
    // Snapshot what we need from storage. Releasing the read lock before
    // the slow inference call keeps other pipeline stages responsive.
    let (task, source_bytes) = {
        let s = storage.read().await;
        let task = s
            .get_ocr_task(job.namespace_id, &job.doc_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("OCR task entry not found"))?;
        let source = s
            .get_document_source(job.namespace_id, &job.doc_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("PDF source not found"))?;
        (task, source)
    };

    let lease = models
        .acquire_ocr()
        .await?
        .ok_or_else(|| anyhow::anyhow!("OCR model not configured"))?;

    // Rasterize scanned pages on the blocking pool — mupdf is CPU-bound.
    let scanned_indices: Vec<usize> = task
        .pages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.decision == PageDecision::NeedsOcr)
        .map(|(i, _)| i)
        .collect();

    let source_for_blocking = source_bytes.clone();
    let scanned_for_blocking = scanned_indices.clone();
    let images: Vec<image::DynamicImage> = tokio::task::spawn_blocking(move || {
        scanned_for_blocking
            .iter()
            .map(|&idx| {
                let png = rasterize_page(&source_for_blocking, idx, RASTER_DPI)?;
                image::load_from_memory(&png).map_err(anyhow::Error::from)
            })
            .collect::<anyhow::Result<Vec<_>>>()
    })
    .await??;

    models.touch_ocr();
    let ocr_texts = lease.ocr_pages(images).await?;
    models.touch_ocr();

    if ocr_texts.len() != scanned_indices.len() {
        return Err(anyhow::anyhow!(
            "OCR returned {} pages, expected {}",
            ocr_texts.len(),
            scanned_indices.len()
        ));
    }

    let (full_text, page_boundaries) = merge_pages(&task, ocr_texts);

    {
        let s = storage.read().await;
        s.write_text_and_meta(job.namespace_id, &job.doc_id, &full_text, &page_boundaries)
            .await?;
        s.delete_ocr_task(job.namespace_id, &job.doc_id).await?;
    }

    tracing::info!(
        doc_id = %job.doc_id,
        page_count = task.page_count,
        ocr_pages = scanned_indices.len(),
        text_len = full_text.len(),
        "OCR completed and text written",
    );
    Ok(())
}

/// Merge the per-page extraction with OCR results into the final
/// document text and page boundaries.
///
/// For each page, the digital text (if any) is used; pages that needed
/// OCR consume the next entry from `ocr_texts` in order. Trailing
/// newlines are added between non-empty pages to match the digital-only
/// extractor's output shape (so embed's char-offset-to-page math
/// continues to work).
pub(crate) fn merge_pages(task: &OcrTask, ocr_texts: Vec<String>) -> (String, Vec<usize>) {
    let mut full = String::new();
    let mut boundaries = Vec::with_capacity(task.page_count);
    let mut ocr_iter = ocr_texts.into_iter();

    for page in &task.pages {
        let page_text = match page.decision {
            PageDecision::Digital | PageDecision::Blank => page.digital_text.clone(),
            PageDecision::NeedsOcr => ocr_iter.next().unwrap_or_default(),
        };
        full.push_str(&page_text);
        if !page_text.ends_with('\n') && !page_text.is_empty() {
            full.push('\n');
        }
        boundaries.push(full.len());
    }

    (full, boundaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdf::{OcrTask, PageDecision, PageExtraction};

    fn page(text: &str, decision: PageDecision) -> PageExtraction {
        PageExtraction {
            digital_text: if decision == PageDecision::Digital {
                let mut s = text.to_string();
                if !s.ends_with('\n') && !s.is_empty() {
                    s.push('\n');
                }
                s
            } else {
                String::new()
            },
            decision,
            has_image_xobjects: decision == PageDecision::NeedsOcr,
            digital_char_count: text.chars().filter(|c| c.is_alphanumeric()).count(),
        }
    }

    #[test]
    fn merge_all_digital_no_ocr_consumed() {
        let task = OcrTask {
            doc_id: "d".into(),
            page_count: 2,
            pages: vec![
                page("hello world enough text", PageDecision::Digital),
                page("second page also long enough", PageDecision::Digital),
            ],
        };
        let (text, b) = merge_pages(&task, vec![]);
        assert!(text.contains("hello"));
        assert!(text.contains("second"));
        assert_eq!(b.len(), 2);
        assert_eq!(b[1], text.len());
    }

    #[test]
    fn merge_mixed_digital_and_ocr() {
        let task = OcrTask {
            doc_id: "d".into(),
            page_count: 3,
            pages: vec![
                page("digital cover page", PageDecision::Digital),
                page("", PageDecision::NeedsOcr),
                page("digital appendix", PageDecision::Digital),
            ],
        };
        let (text, b) = merge_pages(&task, vec!["scanned middle content".to_string()]);
        assert!(text.contains("digital cover"));
        assert!(text.contains("scanned middle"));
        assert!(text.contains("digital appendix"));
        assert_eq!(b.len(), 3);
        // Each boundary monotonic.
        assert!(b[0] < b[1] && b[1] < b[2]);
    }

    #[test]
    fn merge_blank_pages_emit_empty_with_zero_advance() {
        let task = OcrTask {
            doc_id: "d".into(),
            page_count: 3,
            pages: vec![
                page("first page text", PageDecision::Digital),
                page("", PageDecision::Blank),
                page("third page text", PageDecision::Digital),
            ],
        };
        let (text, b) = merge_pages(&task, vec![]);
        assert!(text.contains("first"));
        assert!(text.contains("third"));
        // Blank page contributes nothing — the second boundary equals the
        // first.
        assert_eq!(b[0], b[1]);
        assert!(b[2] > b[1]);
    }

    #[test]
    fn merge_empty_ocr_result_falls_back_to_empty() {
        let task = OcrTask {
            doc_id: "d".into(),
            page_count: 1,
            pages: vec![page("", PageDecision::NeedsOcr)],
        };
        // OCR returned empty (e.g. per-page failure swallowed).
        let (text, b) = merge_pages(&task, vec![String::new()]);
        assert!(text.is_empty());
        assert_eq!(b, vec![0]);
    }
}
