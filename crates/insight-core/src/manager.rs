//! Central manager for the active chat provider.
//!
//! There is a single chat slot; ownership of the provider lives here, not
//! in `AppState`. Callers go through [`ModelManager::acquire_chat`] to get
//! a lease they can use for one request. Providers are remote (OpenAI,
//! Anthropic), so there is no weight lifecycle to manage — but status
//! events (`Loading`/`Ready`/`Failed`/`Unloaded`) still stream to
//! subscribers around the (default no-op) load calls, keeping the UI
//! contract stable.

use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{broadcast, RwLock};

use crate::provider::{ChatProvider, Provider, ProviderConfig};
use crate::{ModelStatus, ModelType};

const STATUS_CHANNEL_CAPACITY: usize = 64;

/// Central manager for the chat provider slot.
pub struct ModelManager {
    chat: RwLock<Option<Arc<dyn ChatProvider>>>,
    chat_config: RwLock<Option<ProviderConfig>>,
    status_tx: broadcast::Sender<ModelStatus>,
}

impl ModelManager {
    pub fn new() -> Self {
        let (status_tx, _) = broadcast::channel(STATUS_CHANNEL_CAPACITY);
        Self {
            chat: RwLock::new(None),
            chat_config: RwLock::new(None),
            status_tx,
        }
    }

    pub fn subscribe_status(&self) -> broadcast::Receiver<ModelStatus> {
        self.status_tx.subscribe()
    }

    fn emit(&self, status: ModelStatus) {
        let _ = self.status_tx.send(status);
    }

    // ------------------------------------------------------------------
    // Chat
    // ------------------------------------------------------------------

    pub async fn set_chat(
        &self,
        provider: Arc<dyn ChatProvider>,
        config: ProviderConfig,
    ) -> Result<()> {
        if let Some(old) = self.chat.write().await.replace(provider) {
            self.announce_unload(old.as_ref() as &dyn Provider, ModelType::Language)
                .await;
        }
        *self.chat_config.write().await = Some(config);
        Ok(())
    }

    pub async fn clear_chat(&self) {
        if let Some(old) = self.chat.write().await.take() {
            self.announce_unload(old.as_ref() as &dyn Provider, ModelType::Language)
                .await;
        }
        *self.chat_config.write().await = None;
    }

    pub async fn acquire_chat(&self) -> Result<Option<ChatLease>> {
        let provider = match self.chat.read().await.as_ref().cloned() {
            Some(p) => p,
            None => return Ok(None),
        };
        self.ensure_loaded(provider.as_ref() as &dyn Provider, ModelType::Language)
            .await?;
        Ok(Some(Lease { provider }))
    }

    pub async fn chat_config(&self) -> Option<ProviderConfig> {
        self.chat_config.read().await.clone()
    }

    pub async fn chat_ready(&self) -> bool {
        self.chat.read().await.is_some()
    }

    // ------------------------------------------------------------------
    // Internal: loading
    // ------------------------------------------------------------------

    /// Emit Loading/Ready only when the provider actually has to load.
    /// For already-resident (remote) providers this is a no-op
    /// event-wise, which means the UI doesn't flash a loading state on
    /// every request.
    async fn ensure_loaded(&self, provider: &dyn Provider, model_type: ModelType) -> Result<()> {
        if provider.is_loaded().await {
            return Ok(());
        }

        let id = provider.model_id().to_string();
        self.emit(ModelStatus::Loading {
            model_type,
            model_id: id.clone(),
            model_name: id.clone(),
        });

        match provider.ensure_loaded().await {
            Ok(()) => {
                self.emit(ModelStatus::Ready {
                    model_type,
                    model_id: id,
                });
                Ok(())
            }
            Err(e) => {
                self.emit(ModelStatus::Failed {
                    model_type,
                    model_id: id,
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }

    /// Call `unload` on `provider` and emit `Unloaded` iff it actually
    /// transitioned (loaded → unloaded). The provider's own `unload` impl is
    /// the single authority on whether a transition happened.
    async fn announce_unload(&self, provider: &dyn Provider, model_type: ModelType) -> bool {
        match provider.unload().await {
            Ok(true) => {
                self.emit(ModelStatus::Unloaded {
                    model_type,
                    model_id: provider.model_id().to_string(),
                });
                true
            }
            Ok(false) => false,
            Err(e) => {
                tracing::warn!(
                    model = %provider.model_id(),
                    error = %e,
                    "unload failed",
                );
                false
            }
        }
    }
}

impl Default for ModelManager {
    fn default() -> Self {
        Self::new()
    }
}

// ------------------------------------------------------------------
// Leases
// ------------------------------------------------------------------

/// Short-lived handle that keeps a provider alive for the duration of one
/// request. Deref-s to `T` so callers can forward methods transparently.
pub struct Lease<T: ?Sized> {
    provider: Arc<T>,
}

impl<T: ?Sized> Lease<T> {
    pub fn provider(&self) -> &T {
        &self.provider
    }
}

impl<T: ?Sized> Clone for Lease<T> {
    fn clone(&self) -> Self {
        Self {
            provider: self.provider.clone(),
        }
    }
}

impl<T: ?Sized> std::ops::Deref for Lease<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

pub type ChatLease = Lease<dyn ChatProvider>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Message;
    use crate::provider::{ChatProvider, CompletionResult, ProviderEvent, ToolDefinition};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::sync::atomic::Ordering;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// A test chat provider that tracks load/unload calls with remote-style
    /// semantics: starts unloaded-in-name, loads on `ensure_loaded`, and
    /// reports real loaded→unloaded transitions.
    struct TestChatProvider {
        id: String,
        loaded: AtomicBool,
        ensure_calls: AtomicUsize,
        unload_calls: AtomicUsize,
    }

    impl TestChatProvider {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                loaded: AtomicBool::new(false),
                ensure_calls: AtomicUsize::new(0),
                unload_calls: AtomicUsize::new(0),
            })
        }

        fn ensure_calls(&self) -> usize {
            self.ensure_calls.load(Ordering::Relaxed)
        }
        fn unload_calls(&self) -> usize {
            self.unload_calls.load(Ordering::Relaxed)
        }
    }

    #[async_trait]
    impl Provider for TestChatProvider {
        fn provider_name(&self) -> &'static str {
            "test"
        }
        fn model_id(&self) -> &str {
            &self.id
        }
        async fn is_loaded(&self) -> bool {
            self.loaded.load(Ordering::Relaxed)
        }
        async fn ensure_loaded(&self) -> Result<()> {
            self.ensure_calls.fetch_add(1, Ordering::Relaxed);
            self.loaded.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn unload(&self) -> Result<bool> {
            self.unload_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.loaded.swap(false, Ordering::Relaxed))
        }
    }

    #[async_trait]
    impl ChatProvider for TestChatProvider {
        async fn stream_completion(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
            event_tx: mpsc::Sender<ProviderEvent>,
            _cancel: CancellationToken,
        ) -> Result<CompletionResult> {
            let _ = event_tx.send(ProviderEvent::Done).await;
            Ok(CompletionResult::default())
        }
    }

    fn remote_config() -> ProviderConfig {
        ProviderConfig::OpenAI {
            api_key: "test".into(),
            model: "gpt".into(),
        }
    }

    #[tokio::test]
    async fn set_chat_replaces_and_unloads_old_provider() {
        let manager = ModelManager::new();
        let first = TestChatProvider::new("first");
        let second = TestChatProvider::new("second");
        manager
            .set_chat(first.clone(), remote_config())
            .await
            .unwrap();
        manager
            .set_chat(second.clone(), remote_config())
            .await
            .unwrap();
        assert_eq!(first.unload_calls(), 1);
        assert_eq!(second.unload_calls(), 0);
    }

    #[tokio::test]
    async fn acquire_chat_loads_exactly_once() {
        let manager = ModelManager::new();
        let p = TestChatProvider::new("chat");
        manager.set_chat(p.clone(), remote_config()).await.unwrap();

        assert_eq!(p.ensure_calls(), 0);
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        assert_eq!(p.ensure_calls(), 1);
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        // Second acquire hits the fast path (is_loaded == true) and skips
        // ensure_loaded — no UI flash per request.
        assert_eq!(p.ensure_calls(), 1);
        assert!(p.is_loaded().await);
    }

    #[tokio::test]
    async fn acquire_chat_returns_none_when_unconfigured() {
        let manager = ModelManager::new();
        assert!(manager.acquire_chat().await.unwrap().is_none());
        assert!(!manager.chat_ready().await);
        assert!(manager.chat_config().await.is_none());
    }

    #[tokio::test]
    async fn clear_chat_unloads_and_clears_config() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat");
        manager.set_chat(chat.clone(), remote_config()).await.unwrap();
        manager.clear_chat().await;
        assert_eq!(chat.unload_calls(), 1);
        assert!(manager.chat_config().await.is_none());
        assert!(!manager.chat_ready().await);
    }

    #[tokio::test]
    async fn set_chat_emits_unloaded_for_loaded_displaced_provider() {
        let manager = ModelManager::new();
        let first = TestChatProvider::new("first");
        manager.set_chat(first.clone(), remote_config()).await.unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        assert!(first.is_loaded().await);

        let mut events = manager.subscribe_status();
        let second = TestChatProvider::new("second");
        manager.set_chat(second.clone(), remote_config()).await.unwrap();

        // First was loaded → displacement must announce Unloaded for it.
        match events.try_recv() {
            Ok(ModelStatus::Unloaded {
                model_type: ModelType::Language,
                model_id,
            }) => assert_eq!(model_id, "first"),
            other => panic!("expected Unloaded for first, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn set_chat_silent_for_unloaded_displaced_provider() {
        let manager = ModelManager::new();
        let first = TestChatProvider::new("first");
        // Install but never acquire — first stays unloaded.
        manager.set_chat(first.clone(), remote_config()).await.unwrap();
        assert!(!first.is_loaded().await);

        let mut events = manager.subscribe_status();
        let second = TestChatProvider::new("second");
        manager.set_chat(second.clone(), remote_config()).await.unwrap();

        // No transition happened — no Unloaded should fire.
        assert!(events.try_recv().is_err());
        // unload was still called (idempotent), but reported false.
        assert_eq!(first.unload_calls(), 1);
    }

    #[tokio::test]
    async fn clear_chat_emits_unloaded_when_loaded() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat");
        manager.set_chat(chat.clone(), remote_config()).await.unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();

        let mut events = manager.subscribe_status();
        manager.clear_chat().await;

        match events.try_recv() {
            Ok(ModelStatus::Unloaded {
                model_type: ModelType::Language,
                model_id,
            }) => assert_eq!(model_id, "chat"),
            other => panic!("expected Unloaded for chat, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn acquire_chat_emits_ready_after_load() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat");
        manager.set_chat(chat.clone(), remote_config()).await.unwrap();

        let mut events = manager.subscribe_status();
        let _ = manager.acquire_chat().await.unwrap().unwrap();

        match events.try_recv() {
            Ok(ModelStatus::Loading { model_id, .. }) => assert_eq!(model_id, "chat"),
            other => panic!("expected Loading, got {other:?}"),
        }
        match events.try_recv() {
            Ok(ModelStatus::Ready { model_id, .. }) => assert_eq!(model_id, "chat"),
            other => panic!("expected Ready, got {other:?}"),
        }
    }
}
