//! Local provider implementations backed by mistralrs.
//!
//! All local providers compose [`LocalModelState`] — a shared helper that
//! owns the weight slot (lazy-loaded, unloadable) and the coexist flag.
//! Role-specific wrappers override [`super::Provider`]'s defaults to report
//! `MemoryKind::Local` and forward coexist/is_loaded/ensure_loaded/unload
//! to the shared state.

pub mod chat;
pub mod embedding;
mod state;

pub(crate) use state::LocalModelState;

pub use chat::LocalChatProvider;
pub use embedding::LocalEmbeddingProvider;
