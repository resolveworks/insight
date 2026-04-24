//! Remote chat provider implementations.
//!
//! Remote providers use [`super::Provider`]'s defaults — `MemoryKind::Remote`,
//! `coexist = true`, no-op `ensure_loaded`/`unload` — so they never
//! contribute to local eviction decisions.

pub mod anthropic;
pub mod openai;

pub use anthropic::AnthropicChatProvider;
pub use openai::OpenAIChatProvider;
