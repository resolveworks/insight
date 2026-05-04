//! OCR provider role trait.
//!
//! OCR providers turn images of document pages into text. The pipeline's
//! OCR worker calls [`OcrProvider::ocr_page`] one page at a time so it can
//! report per-page progress and keep the model-activity timestamp fresh
//! during long documents.

use anyhow::Result;
use async_trait::async_trait;
use image::DynamicImage;

use super::Provider;

/// OCR role trait.
#[async_trait]
pub trait OcrProvider: Provider {
    /// Run OCR over a single page image.
    ///
    /// Errors are per-page by design: the OCR worker substitutes an empty
    /// string so a single bad scan doesn't kill the whole document.
    async fn ocr_page(&self, image: DynamicImage) -> Result<String>;

    /// Run OCR over all given page images.
    ///
    /// Default implementation calls [`ocr_page`](Self::ocr_page) in a
    /// loop. Providers with native batch support may override.
    async fn ocr_pages(&self, pages: Vec<DynamicImage>) -> Result<Vec<String>> {
        let mut out = Vec::with_capacity(pages.len());
        for image in pages {
            match self.ocr_page(image).await {
                Ok(text) => out.push(text),
                Err(e) => {
                    tracing::warn!(error = %e, "OCR page failed; substituting empty");
                    out.push(String::new());
                }
            }
        }
        Ok(out)
    }
}
