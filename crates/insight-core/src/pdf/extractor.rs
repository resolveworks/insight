//! Per-page PDF text extraction and OCR triage.
//!
//! For each page we run lopdf for digital text and mupdf to detect image
//! XObjects. Pages with enough alphanumeric text from lopdf go straight to
//! the digital path; pages with images but no usable text get queued for
//! OCR; truly blank pages are skipped. The pipeline only writes
//! `files/{id}/text` once *all* pages have content (so embed never sees
//! mixed digital + missing-OCR).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};
use mupdf::device::NativeDevice;
use mupdf::{
    pixmap::ImageFormat, ColorParams, Colorspace, Device, Document as MupdfDocument, Image, Matrix,
    Page as MupdfPage, Pixmap,
};
use serde::{Deserialize, Serialize};

/// Pages with fewer than this many alphanumeric characters from lopdf are
/// considered "no usable digital text" — if they also contain image
/// XObjects we route them to OCR. Tuned to catch headers-on-scan (a
/// digital header band over a scanned body) without false-positiving cover
/// pages with title-only text.
pub const DIGITAL_TEXT_THRESHOLD: usize = 32;

/// DPI to rasterize scanned pages at before sending to the OCR model.
/// Nanonets-OCR2 was trained on ~200 DPI scans; higher values mostly
/// inflate VRAM cost without quality wins.
pub const RASTER_DPI: f32 = 200.0;

/// What the extract phase decided to do with one page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageDecision {
    /// lopdf returned enough digital text — use it directly.
    Digital,
    /// lopdf returned little text but the page has image content — needs
    /// OCR.
    NeedsOcr,
    /// Neither digital text nor image content — skip OCR; emit empty text.
    Blank,
}

/// Per-page extraction result. The `digital_text` field is empty unless
/// the decision is [`PageDecision::Digital`] — it's the lopdf output, used
/// directly by the merge step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageExtraction {
    pub digital_text: String,
    pub decision: PageDecision,
    pub has_image_xobjects: bool,
    pub digital_char_count: usize,
}

/// Result of extracting a PDF.
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    pub pdf_bytes: Vec<u8>,
    pub page_count: usize,
    pub pages: Vec<PageExtraction>,
}

impl ExtractedDocument {
    /// Whether any page needs OCR. Drives the storage routing decision:
    /// `false` → write `files/{id}/text` directly; `true` → write
    /// `files/{id}/ocr_task` and let the OCR worker assemble the final
    /// text.
    pub fn needs_ocr(&self) -> bool {
        self.pages
            .iter()
            .any(|p| p.decision == PageDecision::NeedsOcr)
    }

    /// Concatenate just the digital pages' text. Useful when no page
    /// needed OCR, or as the digital baseline before merge.
    pub fn digital_text_concatenated(&self) -> (String, Vec<usize>) {
        let mut full = String::new();
        let mut boundaries = Vec::with_capacity(self.pages.len());
        for page in &self.pages {
            full.push_str(&page.digital_text);
            if !page.digital_text.ends_with('\n') && !page.digital_text.is_empty() {
                full.push('\n');
            }
            boundaries.push(full.len());
        }
        (full, boundaries)
    }
}

/// Payload of a `files/{id}/ocr_task` iroh entry.
///
/// Serialized as JSON. The OCR worker reads this back, rasterizes each
/// `NeedsOcr` page from the PDF source, runs inference, then merges the
/// per-page results into the final `files/{id}/text`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrTask {
    pub doc_id: String,
    pub page_count: usize,
    pub pages: Vec<PageExtraction>,
}

/// Extract text from a PDF on disk.
pub fn extract_text(path: &Path) -> Result<ExtractedDocument> {
    let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;
    extract_text_from_bytes(pdf_bytes)
}

/// Extract text from PDF bytes already in memory.
pub fn extract_text_from_bytes(pdf_bytes: Vec<u8>) -> Result<ExtractedDocument> {
    let lopdf_doc = lopdf::Document::load_mem(&pdf_bytes).context("Failed to parse PDF (lopdf)")?;
    let mupdf_doc = MupdfDocument::from_bytes(&pdf_bytes, "application/pdf")
        .context("Failed to parse PDF (mupdf)")?;

    let mut lopdf_pages: Vec<u32> = lopdf_doc.get_pages().keys().copied().collect();
    lopdf_pages.sort();
    let page_count = lopdf_pages.len();

    let mut pages = Vec::with_capacity(page_count);
    for (idx, page_num) in lopdf_pages.iter().enumerate() {
        let digital_text = lopdf_doc.extract_text(&[*page_num]).unwrap_or_default();
        let alnum = digital_text.chars().filter(|c| c.is_alphanumeric()).count();

        let has_image_xobjects = page_has_images(&mupdf_doc, idx as i32).unwrap_or(false);

        let decision = if alnum >= DIGITAL_TEXT_THRESHOLD {
            PageDecision::Digital
        } else if has_image_xobjects {
            PageDecision::NeedsOcr
        } else {
            PageDecision::Blank
        };

        // Append a trailing newline to digital page text so per-page
        // boundaries are obvious in the merged output. Matches the old
        // single-pass behavior.
        let digital_text = if decision == PageDecision::Digital {
            let mut s = digital_text;
            if !s.ends_with('\n') && !s.is_empty() {
                s.push('\n');
            }
            s
        } else {
            String::new()
        };

        pages.push(PageExtraction {
            digital_text,
            decision,
            has_image_xobjects,
            digital_char_count: alnum,
        });
    }

    tracing::debug!(
        page_count,
        digital = pages
            .iter()
            .filter(|p| p.decision == PageDecision::Digital)
            .count(),
        needs_ocr = pages
            .iter()
            .filter(|p| p.decision == PageDecision::NeedsOcr)
            .count(),
        blank = pages
            .iter()
            .filter(|p| p.decision == PageDecision::Blank)
            .count(),
        "PDF triage complete",
    );

    Ok(ExtractedDocument {
        pdf_bytes,
        page_count,
        pages,
    })
}

/// Render one PDF page to PNG bytes at the given DPI. Used by the OCR
/// worker right before sending to the multimodal model.
pub fn rasterize_page(pdf_bytes: &[u8], page_idx: usize, dpi: f32) -> Result<Vec<u8>> {
    let doc = MupdfDocument::from_bytes(pdf_bytes, "application/pdf")
        .context("mupdf failed to parse PDF for rasterization")?;
    let page = doc
        .load_page(page_idx as i32)
        .context("mupdf failed to load page for rasterization")?;
    let scale = dpi / 72.0;
    let pixmap: Pixmap = page
        .to_pixmap(
            &Matrix::new_scale(scale, scale),
            &Colorspace::device_rgb(),
            false,
            true,
        )
        .context("Failed to rasterize PDF page")?;
    let mut buf = Vec::new();
    pixmap
        .write_to(&mut buf, ImageFormat::PNG)
        .context("Failed to encode page pixmap as PNG")?;
    Ok(buf)
}

/// Given a character offset in the merged full-document text, find which
/// page it falls on (1-indexed). Pages whose end offset is `> offset` are
/// matched; if `offset` is past the last boundary, the last page is
/// returned.
pub fn char_offset_to_page(offset: usize, page_boundaries: &[usize]) -> usize {
    for (i, &boundary) in page_boundaries.iter().enumerate() {
        if offset < boundary {
            return i + 1;
        }
    }
    page_boundaries.len().max(1)
}

// ---- internal: image-XObject detection via a no-op mupdf device ----

#[derive(Default)]
struct ImageCounter {
    count: u32,
}

impl NativeDevice for ImageCounter {
    fn fill_image(&mut self, _img: &Image, _cmt: Matrix, _alpha: f32, _cp: ColorParams) {
        self.count += 1;
    }

    fn fill_image_mask(
        &mut self,
        _img: &Image,
        _cmt: Matrix,
        _color_space: &Colorspace,
        _color: &[f32],
        _alpha: f32,
        _cp: ColorParams,
    ) {
        self.count += 1;
    }
}

fn page_has_images(doc: &MupdfDocument, idx: i32) -> Result<bool> {
    let page: MupdfPage = doc.load_page(idx)?;
    let counter = Rc::new(RefCell::new(ImageCounter::default()));
    let dev = Device::from_native(counter.clone())?;
    page.run(&dev, &Matrix::IDENTITY)?;
    drop(dev);
    let count = counter.borrow().count;
    Ok(count > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream};

    fn create_test_pdf(text: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");

        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        let content = format!(
            "BT /F1 12 Tf 100 700 Td ({}) Tj ET",
            text.replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)")
        );
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });

        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        });

        if let Ok(page) = doc.get_object_mut(page_id) {
            if let Object::Dictionary(ref mut dict) = page {
                dict.set("Parent", pages_id);
            }
        }

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);

        let mut buffer = Vec::new();
        doc.save_to(&mut buffer).unwrap();
        buffer
    }

    /// Build a PDF whose page contains a 1x1 image XObject and no text.
    /// Used to exercise the `NeedsOcr` decision path.
    fn create_image_only_pdf() -> Vec<u8> {
        let mut doc = Document::with_version("1.4");

        // Minimal 1x1 grayscale image XObject.
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 1,
                "Height" => 1,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
                "Filter" => "FlateDecode",
            },
            // FlateDecode-encoded single 0x00 byte. Computed inline so the
            // test doesn't depend on a flate crate.
            //   echo -n -e '\x00' | python3 -c "import sys, zlib; sys.stdout.buffer.write(zlib.compress(sys.stdin.buffer.read()))"
            //   → b'x\x9ccx\x00\x00\x00\x01\x00\x01'
            vec![0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01],
        ));

        // Draw the image scaled to fill the page.
        let content = b"q\n612 0 0 792 0 0 cm\n/Im1 Do\nQ\n";
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.to_vec()));

        let resources_id = doc.add_object(dictionary! {
            "XObject" => dictionary! {
                "Im1" => image_id,
            },
        });

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });

        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        });

        if let Ok(page) = doc.get_object_mut(page_id) {
            if let Object::Dictionary(ref mut dict) = page {
                dict.set("Parent", pages_id);
            }
        }

        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);

        let mut buffer = Vec::new();
        doc.save_to(&mut buffer).unwrap();
        buffer
    }

    #[test]
    fn extract_decision_digital_text_pdf() {
        // Long enough to clear the threshold.
        let pdf =
            create_test_pdf("This is a test document with enough characters to count as digital");
        let result = extract_text_from_bytes(pdf).unwrap();
        assert_eq!(result.page_count, 1);
        assert_eq!(result.pages[0].decision, PageDecision::Digital);
        assert!(!result.needs_ocr());
        assert!(
            result.pages[0].digital_text.contains("test")
                || result.pages[0].digital_text.contains("document")
        );
    }

    #[test]
    fn extract_decision_image_only_pdf_needs_ocr() {
        let pdf = create_image_only_pdf();
        let result = extract_text_from_bytes(pdf).unwrap();
        assert_eq!(result.page_count, 1);
        assert_eq!(result.pages[0].decision, PageDecision::NeedsOcr);
        assert!(result.pages[0].has_image_xobjects);
        assert!(result.pages[0].digital_text.is_empty());
        assert!(result.needs_ocr());
    }

    #[test]
    fn extract_decision_blank_pdf() {
        // No text content stream and no image — should be Blank.
        let pdf = create_test_pdf(""); // Empty text → tiny content stream, no images.
        let result = extract_text_from_bytes(pdf).unwrap();
        assert_eq!(result.pages[0].decision, PageDecision::Blank);
        assert!(!result.needs_ocr());
    }

    #[test]
    fn rasterize_page_returns_png() {
        let pdf = create_test_pdf("Some text");
        let bytes = rasterize_page(&pdf, 0, 100.0).unwrap();
        // PNG magic bytes
        assert_eq!(&bytes[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn char_offset_to_page_basic() {
        let boundaries = vec![10, 20, 30];
        assert_eq!(char_offset_to_page(0, &boundaries), 1);
        assert_eq!(char_offset_to_page(9, &boundaries), 1);
        assert_eq!(char_offset_to_page(10, &boundaries), 2);
        assert_eq!(char_offset_to_page(29, &boundaries), 3);
        assert_eq!(char_offset_to_page(100, &boundaries), 3);
    }
}
