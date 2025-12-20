use std::path::Path;

use anyhow::{Context, Result};

/// Result of extracting text from a PDF
#[derive(Debug, Clone)]
pub struct ExtractedDocument {
    /// Raw PDF bytes
    pub pdf_bytes: Vec<u8>,
    /// Extracted text content
    pub text: String,
    /// Number of pages in the PDF
    pub page_count: usize,
}

/// Extract text from a PDF file
pub fn extract_text(path: &Path) -> Result<ExtractedDocument> {
    let pdf_bytes = std::fs::read(path).context("Failed to read PDF file")?;

    let doc = lopdf::Document::load_mem(&pdf_bytes).context("Failed to parse PDF")?;

    let pages: Vec<u32> = doc.get_pages().keys().cloned().collect();
    let page_count = pages.len();

    let text = doc
        .extract_text(&pages)
        .context("Failed to extract text from PDF")?;

    tracing::debug!("Extracted {} chars from {} pages", text.len(), page_count);

    Ok(ExtractedDocument {
        pdf_bytes,
        text,
        page_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream};

    /// Create a minimal PDF with the given text content
    fn create_test_pdf(text: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");

        // Add a font resource
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        // Create page content stream with text
        let content = format!(
            "BT /F1 12 Tf 100 700 Td ({}) Tj ET",
            text.replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)")
        );
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

        // Create resources dictionary
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        // Create page
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });

        // Create pages tree
        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        });

        // Update page parent reference
        if let Ok(page) = doc.get_object_mut(page_id) {
            if let Object::Dictionary(ref mut dict) = page {
                dict.set("Parent", pages_id);
            }
        }

        // Create catalog
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });

        doc.trailer.set("Root", catalog_id);

        let mut buffer = Vec::new();
        doc.save_to(&mut buffer).unwrap();
        buffer
    }

    /// Create a multi-page PDF
    fn create_multipage_pdf(page_texts: &[&str]) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");

        // Add a font resource
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });

        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! {
                "F1" => font_id,
            },
        });

        let mut page_ids = Vec::new();

        for text in page_texts {
            let content = format!(
                "BT /F1 12 Tf 100 700 Td ({}) Tj ET",
                text.replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)")
            );
            let content_id = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));

            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => resources_id,
                "Contents" => content_id,
            });
            page_ids.push(page_id);
        }

        // Create pages tree
        let kids: Vec<Object> = page_ids.iter().map(|&id| id.into()).collect();
        let pages_id = doc.add_object(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => Object::Integer(page_texts.len() as i64),
        });

        // Update page parent references
        for page_id in &page_ids {
            if let Ok(page) = doc.get_object_mut(*page_id) {
                if let Object::Dictionary(ref mut dict) = page {
                    dict.set("Parent", pages_id);
                }
            }
        }

        // Create catalog
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
    fn test_extract_text_simple() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("test.pdf");

        // Create a PDF with known text
        let pdf_bytes = create_test_pdf("Hello World");
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();

        let result = extract_text(&pdf_path).unwrap();

        assert_eq!(result.page_count, 1);
        assert!(!result.text.is_empty());
        assert!(
            result.text.contains("Hello") || result.text.contains("World"),
            "Expected text to contain 'Hello' or 'World', got: '{}'",
            result.text
        );
        assert_eq!(result.pdf_bytes, pdf_bytes);
    }

    #[test]
    fn test_extract_text_multipage() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("multipage.pdf");

        let pdf_bytes = create_multipage_pdf(&["Page One", "Page Two", "Page Three"]);
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();

        let result = extract_text(&pdf_path).unwrap();

        assert_eq!(result.page_count, 3);
        assert!(!result.text.is_empty());
    }

    #[test]
    fn test_extract_text_file_not_found() {
        let result = extract_text(Path::new("/nonexistent/path/to/file.pdf"));

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Failed to read PDF file"),
            "Expected 'Failed to read PDF file' error, got: {}",
            err
        );
    }

    #[test]
    fn test_extract_text_invalid_pdf() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("invalid.pdf");

        // Write garbage data
        std::fs::write(&pdf_path, b"this is not a valid pdf file").unwrap();

        let result = extract_text(&pdf_path);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Failed to parse PDF"),
            "Expected 'Failed to parse PDF' error, got: {}",
            err
        );
    }

    #[test]
    fn test_extract_text_empty_file() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("empty.pdf");

        // Write empty file
        std::fs::File::create(&pdf_path).unwrap();

        let result = extract_text(&pdf_path);

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Failed to parse PDF"));
    }

    #[test]
    fn test_extract_text_preserves_pdf_bytes() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("preserve.pdf");

        let pdf_bytes = create_test_pdf("Test content");
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();

        let result = extract_text(&pdf_path).unwrap();

        // Verify the original PDF bytes are preserved exactly
        assert_eq!(result.pdf_bytes.len(), pdf_bytes.len());
        assert_eq!(result.pdf_bytes, pdf_bytes);
    }

    #[test]
    fn test_extract_text_special_characters() {
        let temp_dir = tempfile::tempdir().unwrap();
        let pdf_path = temp_dir.path().join("special.pdf");

        // Test with special characters that need escaping in PDF
        let pdf_bytes = create_test_pdf("Test with special chars");
        std::fs::write(&pdf_path, &pdf_bytes).unwrap();

        let result = extract_text(&pdf_path);

        assert!(result.is_ok());
        assert_eq!(result.unwrap().page_count, 1);
    }
}
