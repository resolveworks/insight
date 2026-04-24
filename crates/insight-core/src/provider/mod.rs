//! Provider abstraction for inference capabilities.
//!
//! Every inference capability (chat, embedding, OCR) is behind a role-specific
//! trait that extends a base [`Provider`]. Remote providers use the defaults
//! (`coexist = true`, `MemoryKind::Remote`, no-op `ensure_loaded` / `unload`);
//! local providers override to report VRAM kind and the user's coexist choice.

pub mod chat;
pub mod config;
pub mod embedding;
pub mod local;
pub mod ocr;
pub mod remote;

use anyhow::Result;
use async_trait::async_trait;

pub use chat::{
    finalize_tool_calls, get_tool_definitions, ChatProvider, CompletedToolCall, CompletionResult,
    ProviderEvent, ToolDefinition,
};
pub use config::{get_provider_families, ProviderConfig, ProviderFamily, RemoteModelInfo};
pub use embedding::EmbeddingProvider;
pub use local::{LocalChatProvider, LocalEmbeddingProvider};
pub use ocr::OcrProvider;
pub use remote::{AnthropicChatProvider, OpenAIChatProvider};

/// Where a provider's weights live at runtime.
///
/// `Local` residents compete for VRAM/RAM and participate in eviction;
/// `Remote` providers never consume local resources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKind {
    Local,
    Remote,
}

/// Base trait implemented by every provider, regardless of role.
///
/// Defaults are remote-friendly so `AnthropicChatProvider` and friends can
/// `impl Provider for AnthropicChatProvider {}` and get correct behavior for
/// free. Local providers override the defaults.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Human-readable provider family (e.g. "local", "openai", "anthropic").
    fn provider_name(&self) -> &'static str;

    /// Model identifier within the provider family.
    fn model_id(&self) -> &str;

    /// Whether the weights live in local VRAM/RAM or on a remote service.
    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Remote
    }

    /// Whether this provider tolerates other local providers being resident
    /// at the same time. Remote providers are always coexist — they don't
    /// consume local resources.
    fn coexist(&self) -> bool {
        true
    }

    /// Update the coexist flag. Local implementations store this in their
    /// own state; remote providers are hard-coded `true` and ignore the call.
    fn set_coexist(&self, _coexist: bool) {}

    /// Whether the provider currently has weights resident in memory.
    /// Remote providers report `true` (they don't consume local memory);
    /// local providers reflect whether their weights are loaded.
    async fn is_loaded(&self) -> bool {
        true
    }

    /// Bring the model to a ready-to-infer state. Idempotent.
    ///
    /// Remote providers are already ready; locals load weights on demand.
    async fn ensure_loaded(&self) -> Result<()> {
        Ok(())
    }

    /// Release any in-memory resources. Idempotent.
    async fn unload(&self) -> Result<()> {
        Ok(())
    }
}
