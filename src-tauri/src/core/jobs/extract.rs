//! PDF extraction worker.
//!
//! Spawns blocking tasks to extract text from PDFs concurrently.

use std::path::PathBuf;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::core::pdf;

use super::types::{DocumentFailed, Extracted};

/// Request to extract a PDF
pub struct ExtractRequest {
    /// Path to the PDF file
    pub path: PathBuf,
    /// Collection to add this document to
    pub collection_id: String,
}

/// Maximum concurrent PDF extractions.
/// PDF parsing is CPU-bound, so we limit concurrency to avoid overwhelming the system.
const MAX_CONCURRENT: usize = 8;

/// Spawns the extraction worker.
///
/// Returns a sender to submit extraction requests.
/// Extracted documents are sent to `output_tx`.
/// Failed extractions are sent to `error_tx`.
pub fn spawn(
    cancel: CancellationToken,
    output_tx: mpsc::Sender<Extracted>,
    error_tx: mpsc::Sender<DocumentFailed>,
) -> mpsc::Sender<ExtractRequest> {
    let (tx, mut rx) = mpsc::channel::<ExtractRequest>(64);

    tokio::spawn(async move {
        let mut in_flight: JoinSet<()> = JoinSet::new();

        loop {
            tokio::select! {
                biased;

                // Check cancellation first
                _ = cancel.cancelled() => {
                    tracing::debug!("Extraction worker cancelled");
                    break;
                }

                // Accept new work if under concurrency limit
                Some(req) = rx.recv(), if in_flight.len() < MAX_CONCURRENT => {
                    let output_tx = output_tx.clone();
                    let error_tx = error_tx.clone();

                    in_flight.spawn_blocking(move || {
                        let file_name = req.path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown.pdf".to_string());

                        match pdf::extract_text(&req.path) {
                            Ok(doc) => {
                                let extracted = Extracted {
                                    name: file_name,
                                    collection_id: req.collection_id,
                                    pdf_bytes: doc.pdf_bytes,
                                    text: doc.text,
                                    page_count: doc.page_count,
                                };

                                if output_tx.blocking_send(extracted).is_err() {
                                    tracing::warn!("Failed to send extracted document - channel closed");
                                }
                            }
                            Err(e) => {
                                tracing::error!("Extraction failed for {:?}: {}", req.path, e);

                                let failed = DocumentFailed {
                                    path: req.path.to_string_lossy().to_string(),
                                    error: e.to_string(),
                                };
                                if error_tx.blocking_send(failed).is_err() {
                                    tracing::warn!("Failed to send error - channel closed");
                                }
                            }
                        }
                    });
                }

                // Reap completed tasks
                Some(result) = in_flight.join_next() => {
                    if let Err(e) = result {
                        tracing::error!("Extraction task panicked: {}", e);
                    }
                }

                // Channel closed and no work in flight
                else => {
                    if in_flight.is_empty() {
                        tracing::debug!("Extraction worker shutting down - no more work");
                        break;
                    }
                }
            }
        }

        // Wait for any remaining in-flight tasks
        while let Some(result) = in_flight.join_next().await {
            if let Err(e) = result {
                tracing::error!("Extraction task panicked during shutdown: {}", e);
            }
        }

        tracing::debug!("Extraction worker stopped");
    });

    tx
}
