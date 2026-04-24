//! OCR provider role trait.
//!
//! Stub: no implementations or methods yet. OCR lands in #23 — this trait
//! exists so the [`crate::manager::ModelManager`] has a slot wired up
//! without a follow-up refactor.
use super::Provider;

/// OCR role trait. Currently no methods — populated by #23.
pub trait OcrProvider: Provider {}
