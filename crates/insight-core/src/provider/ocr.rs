//! OCR provider role trait.
//!
//! OCR providers turn images of document pages into text. The pipeline's
//! OCR worker calls [`OcrProvider::ocr_pages`] with one whole document's
//! scanned pages at a time; the provider returns one string per input
//! image, in order. Per-page failures must surface as an empty string —
//! the worker still needs to merge the result with digital pages.

use anyhow::Result;
use async_trait::async_trait;
use image::DynamicImage;

use super::Provider;

/// OCR role trait.
#[async_trait]
pub trait OcrProvider: Provider {
    /// Run OCR over the given page images and return one string per image,
    /// in the same order. Implementations may batch internally; callers
    /// pass one document's scanned pages per call. A per-page failure must
    /// be reported as `String::new()` rather than aborting the whole call.
    async fn ocr_pages(&self, pages: Vec<DynamicImage>) -> Result<Vec<String>>;
}
