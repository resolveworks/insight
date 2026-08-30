//! Provider abstraction for inference capabilities.
//!
//! All providers are remote (OpenAI, Anthropic): chat runs against hosted
//! APIs, so there is no local weight management. The base [`Provider`]
//! trait's defaults encode remote semantics — always "loaded", no-op
//! `ensure_loaded`, `unload` reporting no transition — and concrete
//! providers need only implement `provider_name` and `model_id`.

pub mod chat;
pub mod config;
pub mod remote;

use anyhow::Result;
use async_trait::async_trait;

pub use chat::{
    finalize_tool_calls, get_tool_definitions, ChatProvider, CompletedToolCall, CompletionResult,
    ProviderEvent, ToolDefinition,
};
pub use config::{get_provider_families, ProviderConfig, ProviderFamily, RemoteModelInfo};
pub use remote::{AnthropicChatProvider, OpenAIChatProvider};

/// Base trait implemented by every provider, regardless of role.
///
/// Defaults are remote-friendly so providers like
/// `AnthropicChatProvider` can `impl Provider for AnthropicChatProvider {}`
/// and get correct behavior for free.
#[async_trait]
pub trait Provider: Send + Sync {
    /// Human-readable provider family (e.g. "openai", "anthropic").
    fn provider_name(&self) -> &'static str;

    /// Model identifier within the provider family.
    fn model_id(&self) -> &str;

    /// Whether the provider currently has weights resident in memory.
    /// Remote providers report `true` (they don't consume local memory).
    async fn is_loaded(&self) -> bool {
        true
    }

    /// Bring the provider to a ready-to-infer state. Idempotent.
    /// Remote providers are already ready.
    async fn ensure_loaded(&self) -> Result<()> {
        Ok(())
    }

    /// Release any in-memory resources. Idempotent. Returns `true` iff this
    /// call transitioned the provider from loaded to unloaded — callers use
    /// the bool to decide whether to emit a status event. Remote providers
    /// never hold local memory, so the default returns `false`.
    async fn unload(&self) -> Result<bool> {
        Ok(false)
    }
}
