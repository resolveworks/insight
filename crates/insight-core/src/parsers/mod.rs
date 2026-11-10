//! Parser backends and the registry that routes stored documents to them.
//!
//! This module is the seam an external plugin backend will plug into: a
//! parser declares the formats it accepts, and the registry owns the mapping
//! from a document's MIME type or file extension to exactly one parser.

mod mupdf;

use std::collections::HashSet;
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::text::{Extraction, ParserId};

pub use mupdf::MupdfParser;

/// One input format a parser accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Format {
    /// The format's MIME type, as stored on a document's metadata.
    pub mime: &'static str,
    /// File extensions (without the dot) that name this format.
    pub extensions: &'static [&'static str],
}

/// A backend that extracts text from one or more input formats.
pub trait Parser: Send + Sync {
    /// The identity recorded on the parser's extractions.
    fn id(&self) -> ParserId;

    /// The formats this parser accepts.
    fn formats(&self) -> &'static [Format];

    /// Extract text from a source in `format`.
    ///
    /// `format` is always one of [`Self::formats`].
    fn extract(&self, format: &Format, source: &[u8]) -> Result<Extraction>;
}

/// The set of parsers available to extraction.
///
/// Construction rejects overlapping claims so a MIME type or extension names
/// at most one parser.
pub struct Registry {
    parsers: Vec<Box<dyn Parser>>,
}

impl Registry {
    /// Build a registry from `parsers`.
    ///
    /// Fails when two parsers claim the same MIME type or the same extension.
    pub fn new(parsers: Vec<Box<dyn Parser>>) -> Result<Self> {
        let mut mimes: HashSet<&'static str> = HashSet::new();
        let mut extensions: HashSet<String> = HashSet::new();
        for parser in &parsers {
            for format in parser.formats() {
                if !mimes.insert(format.mime) {
                    bail!(
                        "MIME type '{}' is claimed by more than one parser",
                        format.mime
                    );
                }
                for extension in format.extensions {
                    if !extensions.insert(extension.to_ascii_lowercase()) {
                        bail!("extension '{extension}' is claimed by more than one parser");
                    }
                }
            }
        }
        Ok(Self { parsers })
    }

    /// The parser and format that handle `mime`.
    ///
    /// Fails when no parser accepts the MIME type.
    pub fn parser_for(&self, mime: &str) -> Result<(&dyn Parser, &'static Format)> {
        for parser in &self.parsers {
            for format in parser.formats() {
                if format.mime == mime {
                    return Ok((parser.as_ref(), format));
                }
            }
        }
        bail!("no parser handles MIME type '{mime}'")
    }

    /// The format named by `path`'s extension, matched case-insensitively.
    ///
    /// Fails when the path has no extension or no parser accepts it.
    pub fn format_for_path(&self, path: &Path) -> Result<&'static Format> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .with_context(|| format!("{} has no file extension", path.display()))?;
        for parser in &self.parsers {
            for format in parser.formats() {
                if format
                    .extensions
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(extension))
                {
                    return Ok(format);
                }
            }
        }
        bail!("no parser handles the extension of {}", path.display())
    }
}

/// The registry of parsers this build ships with.
pub fn default_registry() -> Registry {
    Registry::new(vec![Box::new(MupdfParser)])
        .expect("the built-in parser registry has unique format claims")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::UnitKind;

    struct StubParser;

    impl Parser for StubParser {
        fn id(&self) -> ParserId {
            ParserId {
                name: "stub".to_string(),
                version: "1".to_string(),
            }
        }

        fn formats(&self) -> &'static [Format] {
            &[Format {
                mime: "application/x-stub",
                extensions: &["stub"],
            }]
        }

        fn extract(&self, _format: &Format, _source: &[u8]) -> Result<Extraction> {
            Ok(Extraction {
                parser: self.id(),
                text: String::new(),
                unit_kind: Some(UnitKind::Page),
                units: Vec::new(),
            })
        }
    }

    struct DuplicateMimeParser;

    impl Parser for DuplicateMimeParser {
        fn id(&self) -> ParserId {
            ParserId {
                name: "duplicate-mime".to_string(),
                version: "1".to_string(),
            }
        }

        fn formats(&self) -> &'static [Format] {
            &[Format {
                mime: "application/x-stub",
                extensions: &["other"],
            }]
        }

        fn extract(&self, _format: &Format, _source: &[u8]) -> Result<Extraction> {
            unreachable!("duplicate claims cannot be registered")
        }
    }

    struct DuplicateExtensionParser;

    impl Parser for DuplicateExtensionParser {
        fn id(&self) -> ParserId {
            ParserId {
                name: "duplicate-extension".to_string(),
                version: "1".to_string(),
            }
        }

        fn formats(&self) -> &'static [Format] {
            &[Format {
                mime: "application/x-duplicate-extension",
                extensions: &["stub"],
            }]
        }

        fn extract(&self, _format: &Format, _source: &[u8]) -> Result<Extraction> {
            unreachable!("duplicate claims cannot be registered")
        }
    }

    #[test]
    fn rejects_duplicate_mime_claims() {
        let error = Registry::new(vec![Box::new(StubParser), Box::new(DuplicateMimeParser)])
            .err()
            .expect("duplicate MIME claims must be rejected");
        assert!(
            error.to_string().contains("application/x-stub"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn rejects_duplicate_extension_claims() {
        let error = Registry::new(vec![
            Box::new(StubParser),
            Box::new(DuplicateExtensionParser),
        ])
        .err()
        .expect("duplicate extension claims must be rejected");
        assert!(
            error.to_string().contains("stub"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn every_supported_extension_maps_to_its_mime_type() {
        let registry = default_registry();
        let expected = [
            ("pdf", "application/pdf"),
            (
                "docx",
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            ),
            (
                "xlsx",
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            ),
            (
                "pptx",
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            ),
            ("epub", "application/epub+zip"),
            ("html", "text/html"),
            ("htm", "text/html"),
            ("xhtml", "text/html"),
            ("txt", "text/plain"),
            ("md", "text/plain"),
            ("rst", "text/plain"),
        ];
        for (extension, mime) in expected {
            let path = std::path::Path::new("document").with_extension(extension);
            assert_eq!(
                registry.format_for_path(&path).unwrap().mime,
                mime,
                "extension {extension}"
            );
        }
        assert_eq!(
            registry
                .format_for_path(std::path::Path::new("REPORT.PDF"))
                .unwrap()
                .mime,
            "application/pdf",
            "extensions are matched case-insensitively"
        );
    }

    #[test]
    fn unknown_extension_is_an_error_naming_the_path() {
        let error = default_registry()
            .format_for_path(std::path::Path::new("/tmp/report.xyz"))
            .unwrap_err();
        assert!(
            error.to_string().contains("/tmp/report.xyz"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unknown_mime_type_is_an_error() {
        let error = default_registry()
            .parser_for("application/x-unknown")
            .err()
            .expect("unknown MIME types must be rejected");
        assert!(
            error.to_string().contains("application/x-unknown"),
            "unexpected error: {error}"
        );
    }
}
