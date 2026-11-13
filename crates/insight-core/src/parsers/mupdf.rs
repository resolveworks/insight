//! Text extraction via MuPDF.
//!
//! Extracts per-page digital text (sorted page numbers), appends a
//! trailing `\n` per non-empty page so per-page boundaries are obvious
//! in the merged output, and records the text span each page produced.
//! Non-paginated formats are reflowed by MuPDF onto pages.

use anyhow::{Context, Result};
use mupdf::{Document, TextExtractOptions, TextPageFlags};

use super::{Format, Parser};
use crate::text::{Extraction, ParserId, TextSpan, Unit, UnitKind};

/// Parser name recorded on extractions produced by this module.
const PARSER_NAME: &str = "mupdf";

/// Keep only text that can be seen, and flatten hyphenated line breaks.
///
/// `CLIP` drops glyphs entirely outside the page's clip path, which is how
/// off-page content ends up in extraction; `DEHYPHENATE` joins a line ending
/// in a hyphen with the next one instead of exposing `hyphen-` as a broken
/// token.
const EXTRACT_FLAGS: TextPageFlags = TextPageFlags::CLIP.union(TextPageFlags::DEHYPHENATE);

/// The formats MuPDF's bundled handlers accept.
const FORMATS: &[Format] = &[
    Format {
        mime: "application/pdf",
        extensions: &["pdf"],
    },
    Format {
        mime: "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        extensions: &["docx"],
    },
    Format {
        mime: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        extensions: &["xlsx"],
    },
    Format {
        mime: "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        extensions: &["pptx"],
    },
    Format {
        mime: "application/epub+zip",
        extensions: &["epub"],
    },
    Format {
        mime: "text/html",
        extensions: &["html", "htm", "xhtml"],
    },
    Format {
        mime: "text/plain",
        extensions: &["txt", "md", "rst"],
    },
];

/// MuPDF text extraction backend.
///
/// One parser identity covers every format MuPDF handles.
pub struct MupdfParser;

impl Parser for MupdfParser {
    fn id(&self) -> ParserId {
        ParserId {
            name: PARSER_NAME.to_string(),
        }
    }

    fn formats(&self) -> &'static [Format] {
        FORMATS
    }

    fn extract(&self, format: &Format, source: &[u8]) -> Result<Extraction> {
        // MuPDF dispatches on this magic string; the format's first extension
        // names its handler (all of a format's extensions share one handler).
        let magic = format.extensions[0];
        let doc = Document::from_bytes(source, magic).context("Failed to parse source (MuPDF)")?;

        let page_count = doc.page_count().context("Failed to read page count")?;
        let page_count = usize::try_from(page_count)
            .with_context(|| format!("Source reported an invalid page count ({page_count})"))?;

        let mut text = String::new();
        let mut units = Vec::with_capacity(page_count);
        for (index, page_number) in (0..page_count).enumerate() {
            let page_number = i32::try_from(page_number).context("Page index out of range")?;
            let page = doc
                .load_page(page_number)
                .with_context(|| format!("Failed to load page {}", index + 1))?;
            let page_text = page
                .text(TextExtractOptions {
                    flags: EXTRACT_FLAGS,
                })
                .with_context(|| format!("Failed to extract text from page {}", index + 1))?;

            // MuPDF ends a page's text with a blank line for block separation —
            // treat whitespace-only pages as blank (no text, no boundary advance).
            let page_text = page_text.trim_end_matches(['\r', '\n']);
            let span_start = text.len();
            if !page_text.trim().is_empty() {
                text.push_str(page_text);
                if !text.ends_with('\n') {
                    text.push('\n');
                }
            }
            let spans = if text.len() > span_start {
                vec![TextSpan {
                    start: span_start,
                    end: text.len(),
                }]
            } else {
                Vec::new()
            };
            units.push(Unit {
                number: index + 1,
                spans,
            });
        }

        tracing::debug!(
            page_count,
            text_len = text.len(),
            "Text extraction complete"
        );

        Ok(Extraction {
            parser: self.id(),
            text,
            unit_kind: Some(UnitKind::Page),
            units,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEXT_PDF: &[u8] = include_bytes!("../../tests/fixtures/text.pdf");
    const BLANK_PDF: &[u8] = include_bytes!("../../tests/fixtures/blank.pdf");
    /// Type 3 font with custom `/Differences` glyph names: only the font's
    /// `/ToUnicode` CMap yields real text.
    const TYPE3_PDF: &[u8] = include_bytes!("../../tests/fixtures/type3.pdf");
    /// Minimal OOXML packages: `_rels/.rels` plus one document part each.
    const OFFICE_DOCX: &[u8] = include_bytes!("../../tests/fixtures/office.docx");
    const OFFICE_XLSX: &[u8] = include_bytes!("../../tests/fixtures/office.xlsx");
    const OFFICE_PPTX: &[u8] = include_bytes!("../../tests/fixtures/office.pptx");

    /// Extract `source` as the format with MIME type `mime`.
    fn extract(mime: &str, source: &[u8]) -> Extraction {
        let parser = MupdfParser;
        let format = parser
            .formats()
            .iter()
            .find(|format| format.mime == mime)
            .expect("supported MIME type");
        parser.extract(format, source).unwrap()
    }

    #[test]
    fn extract_digital_text_pdf() {
        let result = extract("application/pdf", TEXT_PDF);
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert_eq!(result.page_count(), Some(1));
        assert_eq!(result.units.len(), 1);
        assert_eq!(result.units[0].number, 1);
        assert!(result
            .text
            .contains("This is a test document with enough characters to count as digital"));
        assert_eq!(result.units[0].spans.len(), 1);
        assert_eq!(result.units[0].spans[0].start, 0);
        assert_eq!(result.units[0].spans[0].end, result.text.len());
        assert_eq!(result.parser.name, "mupdf");
    }

    #[test]
    fn extract_blank_pdf() {
        let result = extract("application/pdf", BLANK_PDF);
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert!(result.text.is_empty());
        assert_eq!(
            result.units,
            vec![Unit {
                number: 1,
                spans: Vec::new(),
            }]
        );
    }

    #[test]
    fn extract_type3_custom_encoding_pdf() {
        let result = extract("application/pdf", TYPE3_PDF);
        assert_eq!(result.text.trim(), "Search for evidence");
    }

    #[test]
    fn extract_html_document() {
        let source = br#"<html>
            <body>
                <h1>Quarterly review</h1>
                <p>Revenue grew across every region.</p>
            </body>
        </html>"#;
        let result = extract("text/html", source);
        assert!(
            result.text.contains("Quarterly review"),
            "missing heading: {:?}",
            result.text
        );
        assert!(
            result.text.contains("Revenue grew across every region."),
            "missing paragraph: {:?}",
            result.text
        );
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert_eq!(result.page_count(), Some(result.units.len()));
        assert!(result.page_count().unwrap() >= 1);
    }

    #[test]
    fn extract_plain_text_document() {
        let source = b"first line of the note\nsecond line of the note\n";
        let result = extract("text/plain", source);
        assert!(
            result.text.contains("first line of the note"),
            "missing first line: {:?}",
            result.text
        );
        assert!(
            result.text.contains("second line of the note"),
            "missing second line: {:?}",
            result.text
        );
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert_eq!(result.page_count(), Some(result.units.len()));
        assert!(result.page_count().unwrap() >= 1);
    }

    #[test]
    fn extract_docx_document() {
        let result = extract(
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            OFFICE_DOCX,
        );
        assert!(
            result
                .text
                .contains("The quarterly report explains revenue growth."),
            "missing paragraph: {:?}",
            result.text
        );
        assert!(
            result.text.contains("Second paragraph of the memo."),
            "missing second paragraph: {:?}",
            result.text
        );
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert!(result.page_count().unwrap() >= 1);
    }

    #[test]
    fn extract_xlsx_document() {
        let result = extract(
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            OFFICE_XLSX,
        );
        for expected in ["Region", "Revenue", "North", "1250"] {
            assert!(
                result.text.contains(expected),
                "missing {expected:?}: {:?}",
                result.text
            );
        }
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert!(result.page_count().unwrap() >= 1);
    }

    #[test]
    fn extract_pptx_document() {
        let result = extract(
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            OFFICE_PPTX,
        );
        assert!(
            result.text.contains("Findings from the field"),
            "missing title: {:?}",
            result.text
        );
        assert!(
            result.text.contains("Revenue grew across every region."),
            "missing body: {:?}",
            result.text
        );
        assert_eq!(result.unit_kind, Some(UnitKind::Page));
        assert!(result.page_count().unwrap() >= 1);
    }
}
