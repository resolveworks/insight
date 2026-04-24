//! Central manager for in-memory inference state.
//!
//! One slot per role (chat, embedding, OCR). Ownership of the provider lives
//! here, not in `AppState`. Callers go through `acquire_*` to get a lease
//! they can use for one request; the lease keeps the provider alive for the
//! duration of the call.
//!
//! # Lifecycle
//!
//! - `set_*` installs a provider but does **not** load its weights.
//! - `acquire_*` returns a lease and triggers [`Provider::ensure_loaded`] on
//!   first use. Status events stream to subscribers so the UI can show the
//!   one-time load — emitted only when the load actually has to happen.
//! - Before loading a `coexist = false` local provider, the manager first
//!   unloads any other `coexist = false` local residents in different roles
//!   ("reactive eviction"). Remote providers never participate.
//! - An idle reaper (`spawn_idle_reaper`) unloads residents with no
//!   activity in the last `IDLE_TTL`, emitting `Unloaded` on each.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use tokio::sync::{broadcast, watch, RwLock};

use crate::config::LifecycleConfig;
use crate::provider::{
    ChatProvider, EmbeddingProvider, MemoryKind, OcrProvider, Provider, ProviderConfig,
};
use crate::{ModelStatus, ModelType};

const STATUS_CHANNEL_CAPACITY: usize = 64;

/// How long a loaded local model can be idle before the reaper unloads it.
const IDLE_TTL: Duration = Duration::from_secs(5 * 60);

/// How often the reaper checks for idle models.
const REAP_INTERVAL: Duration = Duration::from_secs(30);

/// Central manager for chat / embedding / OCR providers.
pub struct ModelManager {
    chat: RwLock<Option<Arc<dyn ChatProvider>>>,
    chat_config: RwLock<Option<ProviderConfig>>,
    chat_last_activity: AtomicU64,

    embedding: RwLock<Option<Arc<dyn EmbeddingProvider>>>,
    embedding_model_id: RwLock<Option<String>>,
    embedding_last_activity: AtomicU64,

    #[allow(dead_code)]
    ocr: RwLock<Option<Arc<dyn OcrProvider>>>,
    #[allow(dead_code)]
    ocr_last_activity: AtomicU64,

    lifecycle: RwLock<LifecycleConfig>,
    status_tx: broadcast::Sender<ModelStatus>,
    focus_tx: watch::Sender<bool>,
}

impl ModelManager {
    pub fn new() -> Self {
        let (status_tx, _) = broadcast::channel(STATUS_CHANNEL_CAPACITY);
        let (focus_tx, _) = watch::channel(false);
        Self {
            chat: RwLock::new(None),
            chat_config: RwLock::new(None),
            chat_last_activity: AtomicU64::new(0),
            embedding: RwLock::new(None),
            embedding_model_id: RwLock::new(None),
            embedding_last_activity: AtomicU64::new(0),
            ocr: RwLock::new(None),
            ocr_last_activity: AtomicU64::new(0),
            lifecycle: RwLock::new(LifecycleConfig::default()),
            status_tx,
            focus_tx,
        }
    }

    /// Spawn the idle reaper. Call once from app startup.
    ///
    /// Ticks every [`REAP_INTERVAL`]; any resident local model idle longer
    /// than [`IDLE_TTL`] is unloaded. Skips the chat slot while research
    /// focus is held so returning to the page doesn't pay a reload.
    pub fn spawn_idle_reaper(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(REAP_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                this.reap_idle().await;
            }
        });
    }

    pub fn subscribe_status(&self) -> broadcast::Receiver<ModelStatus> {
        self.status_tx.subscribe()
    }

    fn emit(&self, status: ModelStatus) {
        let _ = self.status_tx.send(status);
    }

    // ------------------------------------------------------------------
    // Foreground signal
    // ------------------------------------------------------------------

    /// Mark research focus as entered or left. No-op if the value is
    /// already set — subscribers aren't woken for repeat enters.
    pub fn set_research_focused(&self, focused: bool) {
        self.focus_tx.send_if_modified(|current| {
            if *current == focused {
                false
            } else {
                *current = focused;
                true
            }
        });
    }

    pub fn is_research_focused(&self) -> bool {
        *self.focus_tx.borrow()
    }

    pub fn focus_guard(&self) -> FocusGuard {
        FocusGuard {
            rx: self.focus_tx.subscribe(),
        }
    }

    // ------------------------------------------------------------------
    // Lifecycle config
    // ------------------------------------------------------------------

    /// Replace the lifecycle configuration and propagate coexist flags to
    /// currently installed providers.
    pub async fn set_lifecycle_config(&self, cfg: LifecycleConfig) {
        *self.lifecycle.write().await = cfg.clone();
        if let Some(p) = self.chat.read().await.as_ref() {
            p.set_coexist(cfg.chat_coexist);
        }
        if let Some(p) = self.embedding.read().await.as_ref() {
            p.set_coexist(cfg.embedding_coexist);
        }
    }

    pub async fn lifecycle_config(&self) -> LifecycleConfig {
        self.lifecycle.read().await.clone()
    }

    // ------------------------------------------------------------------
    // Chat
    // ------------------------------------------------------------------

    pub async fn set_chat(
        &self,
        provider: Arc<dyn ChatProvider>,
        config: ProviderConfig,
    ) -> Result<()> {
        // Apply current coexist setting so the provider's view matches
        // settings without a separate call.
        provider.set_coexist(self.lifecycle.read().await.chat_coexist);
        if let Some(old) = self.chat.write().await.replace(provider) {
            let _ = old.unload().await;
        }
        *self.chat_config.write().await = Some(config);
        Ok(())
    }

    pub async fn clear_chat(&self) {
        if let Some(old) = self.chat.write().await.take() {
            let _ = old.unload().await;
        }
        *self.chat_config.write().await = None;
    }

    pub async fn acquire_chat(&self) -> Result<Option<ChatLease>> {
        let provider = match self.chat.read().await.as_ref().cloned() {
            Some(p) => p,
            None => return Ok(None),
        };
        self.touch(ModelType::Language);
        self.evict_conflicting(ModelType::Language, provider.as_ref() as &dyn Provider)
            .await;
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
    // Embedding
    // ------------------------------------------------------------------

    pub async fn set_embedding(
        &self,
        provider: Arc<dyn EmbeddingProvider>,
        model_id: String,
    ) -> Result<()> {
        provider.set_coexist(self.lifecycle.read().await.embedding_coexist);
        if let Some(old) = self.embedding.write().await.replace(provider) {
            let _ = old.unload().await;
        }
        *self.embedding_model_id.write().await = Some(model_id);
        Ok(())
    }

    pub async fn clear_embedding(&self) {
        if let Some(old) = self.embedding.write().await.take() {
            let _ = old.unload().await;
        }
        *self.embedding_model_id.write().await = None;
    }

    pub async fn acquire_embedding(&self) -> Result<Option<EmbeddingLease>> {
        let provider = match self.embedding.read().await.as_ref().cloned() {
            Some(p) => p,
            None => return Ok(None),
        };
        self.touch(ModelType::Embedding);
        self.evict_conflicting(ModelType::Embedding, provider.as_ref() as &dyn Provider)
            .await;
        self.ensure_loaded(provider.as_ref() as &dyn Provider, ModelType::Embedding)
            .await?;
        Ok(Some(Lease { provider }))
    }

    /// Refresh the embedding activity timestamp. Call from long-running
    /// embed jobs so the reaper doesn't unload mid-batch.
    pub fn touch_embedding(&self) {
        self.touch(ModelType::Embedding);
    }

    pub async fn embedding_model_id(&self) -> Option<String> {
        self.embedding_model_id.read().await.clone()
    }

    pub async fn embedding_ready(&self) -> bool {
        self.embedding.read().await.is_some()
    }

    // ------------------------------------------------------------------
    // Internal: loading, eviction, reaping
    // ------------------------------------------------------------------

    /// Emit Loading/Ready only when the provider actually has to load.
    /// For already-resident or remote providers this is a no-op event-wise,
    /// which means the UI doesn't flash a loading state on every request.
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

    /// If `target` is a local `coexist = false` provider, unload any other
    /// local `coexist = false` residents in the other roles before we load.
    async fn evict_conflicting(&self, target_role: ModelType, target: &dyn Provider) {
        if target.memory_kind() != MemoryKind::Local || target.coexist() {
            return;
        }

        if target_role != ModelType::Language {
            if let Some(p) = self.chat.read().await.as_ref().cloned() {
                self.maybe_evict(p.as_ref() as &dyn Provider, ModelType::Language)
                    .await;
            }
        }
        if target_role != ModelType::Embedding {
            if let Some(p) = self.embedding.read().await.as_ref().cloned() {
                self.maybe_evict(p.as_ref() as &dyn Provider, ModelType::Embedding)
                    .await;
            }
        }
        if target_role != ModelType::Ocr {
            if let Some(p) = self.ocr.read().await.as_ref().cloned() {
                self.maybe_evict(p.as_ref() as &dyn Provider, ModelType::Ocr)
                    .await;
            }
        }
    }

    async fn maybe_evict(&self, provider: &dyn Provider, model_type: ModelType) {
        if provider.memory_kind() != MemoryKind::Local || provider.coexist() {
            return;
        }
        if !provider.is_loaded().await {
            return;
        }
        tracing::info!(
            target = provider.model_id(),
            "Reactive eviction: unloading coexist=false local resident"
        );
        let _ = provider.unload().await;
        self.emit(ModelStatus::Unloaded {
            model_type,
            model_id: provider.model_id().to_string(),
        });
    }

    async fn reap_idle(&self) {
        let cutoff = unix_now().saturating_sub(IDLE_TTL.as_secs());
        let focus_held = self.is_research_focused();

        // Skip chat while the user is looking at research.
        if !focus_held {
            if let Some(provider) = self.chat.read().await.as_ref().cloned() {
                let last = self.chat_last_activity.load(Ordering::Relaxed);
                if last != 0 && last <= cutoff {
                    self.try_reap(provider.as_ref() as &dyn Provider, ModelType::Language)
                        .await;
                }
            }
        }

        if let Some(provider) = self.embedding.read().await.as_ref().cloned() {
            let last = self.embedding_last_activity.load(Ordering::Relaxed);
            if last != 0 && last <= cutoff {
                self.try_reap(provider.as_ref() as &dyn Provider, ModelType::Embedding)
                    .await;
            }
        }

        if let Some(provider) = self.ocr.read().await.as_ref().cloned() {
            let last = self.ocr_last_activity.load(Ordering::Relaxed);
            if last != 0 && last <= cutoff {
                self.try_reap(provider.as_ref() as &dyn Provider, ModelType::Ocr)
                    .await;
            }
        }
    }

    async fn try_reap(&self, provider: &dyn Provider, model_type: ModelType) {
        if provider.memory_kind() != MemoryKind::Local {
            return;
        }
        // `acquire_*` refreshes last_activity every call, so the time-based
        // check in `reap_idle` already captures recent work. We let `unload`
        // be the single authority on "was there anything to drop."
        tracing::info!(model = %provider.model_id(), "Idle reaper: unloading model");
        let _ = provider.unload().await;
        self.emit(ModelStatus::Unloaded {
            model_type,
            model_id: provider.model_id().to_string(),
        });
    }

    fn touch(&self, model_type: ModelType) {
        let now = unix_now();
        match model_type {
            ModelType::Language => self.chat_last_activity.store(now, Ordering::Relaxed),
            ModelType::Embedding => self.embedding_last_activity.store(now, Ordering::Relaxed),
            ModelType::Ocr => self.ocr_last_activity.store(now, Ordering::Relaxed),
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

/// Subscription to the foreground-focus signal. Background workers call
/// [`FocusGuard::wait_until_released`] between jobs so chat has priority.
pub struct FocusGuard {
    rx: watch::Receiver<bool>,
}

impl FocusGuard {
    /// Return immediately if focus isn't held; otherwise wait until it is
    /// released. In-progress work is never preempted — workers call this at
    /// natural boundaries (between documents).
    pub async fn wait_until_released(&mut self) {
        if !*self.rx.borrow() {
            return;
        }
        tracing::debug!("Worker yielding — research focus is held");
        while *self.rx.borrow() {
            if self.rx.changed().await.is_err() {
                return;
            }
        }
        tracing::debug!("Research focus released — worker resuming");
    }
}

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
pub type EmbeddingLease = Lease<dyn EmbeddingProvider>;

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::Message;
    use crate::provider::{ChatProvider, CompletionResult, ProviderEvent, ToolDefinition};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    /// A test chat provider that tracks load/unload calls and reports a
    /// configurable memory kind + coexist flag.
    struct TestChatProvider {
        id: String,
        kind: MemoryKind,
        coexist: AtomicBool,
        loaded: AtomicBool,
        ensure_calls: AtomicUsize,
        unload_calls: AtomicUsize,
    }

    impl TestChatProvider {
        fn new(id: &str, kind: MemoryKind, coexist: bool) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                kind,
                coexist: AtomicBool::new(coexist),
                loaded: AtomicBool::new(kind == MemoryKind::Remote),
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
        fn memory_kind(&self) -> MemoryKind {
            self.kind
        }
        fn coexist(&self) -> bool {
            self.coexist.load(Ordering::Relaxed)
        }
        fn set_coexist(&self, v: bool) {
            self.coexist.store(v, Ordering::Relaxed);
        }
        async fn is_loaded(&self) -> bool {
            self.loaded.load(Ordering::Relaxed)
        }
        async fn ensure_loaded(&self) -> Result<()> {
            self.ensure_calls.fetch_add(1, Ordering::Relaxed);
            self.loaded.store(true, Ordering::Relaxed);
            Ok(())
        }
        async fn unload(&self) -> Result<()> {
            self.unload_calls.fetch_add(1, Ordering::Relaxed);
            self.loaded.store(false, Ordering::Relaxed);
            Ok(())
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
        let first = TestChatProvider::new("first", MemoryKind::Local, false);
        let second = TestChatProvider::new("second", MemoryKind::Local, false);
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
        let p = TestChatProvider::new("chat", MemoryKind::Local, false);
        manager.set_chat(p.clone(), remote_config()).await.unwrap();

        assert_eq!(p.ensure_calls(), 0);
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        assert_eq!(p.ensure_calls(), 1);
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        // Second acquire hits the fast path (is_loaded == true) and skips
        // ensure_loaded — that's the whole point: no UI flash per request.
        assert_eq!(p.ensure_calls(), 1);
        assert!(p.is_loaded().await);
    }

    #[tokio::test]
    async fn reactive_eviction_unloads_coexist_false_peer() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat", MemoryKind::Local, false);
        manager
            .set_chat(chat.clone(), remote_config())
            .await
            .unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();
        assert!(chat.is_loaded().await);

        // A second local coexist=false provider acquires — chat should evict.
        let other = TestChatProvider::new("other", MemoryKind::Local, false);
        manager
            .evict_conflicting(ModelType::Embedding, &*other)
            .await;
        assert_eq!(chat.unload_calls(), 1);
        assert!(!chat.is_loaded().await);
    }

    #[tokio::test]
    async fn reactive_eviction_leaves_coexist_peer_alone() {
        let manager = ModelManager::new();
        manager
            .set_lifecycle_config(LifecycleConfig {
                chat_coexist: true,
                embedding_coexist: false,
            })
            .await;
        let chat = TestChatProvider::new("chat", MemoryKind::Local, true);
        manager
            .set_chat(chat.clone(), remote_config())
            .await
            .unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();

        let other = TestChatProvider::new("other", MemoryKind::Local, false);
        manager
            .evict_conflicting(ModelType::Embedding, &*other)
            .await;
        assert_eq!(chat.unload_calls(), 0);
    }

    #[tokio::test]
    async fn remote_provider_is_ignored_by_eviction() {
        let manager = ModelManager::new();
        let remote = TestChatProvider::new("remote", MemoryKind::Remote, true);
        manager
            .set_chat(remote.clone(), remote_config())
            .await
            .unwrap();

        let target = TestChatProvider::new("target", MemoryKind::Local, false);
        manager
            .evict_conflicting(ModelType::Embedding, &*target)
            .await;
        assert_eq!(remote.unload_calls(), 0);
    }

    #[tokio::test]
    async fn set_lifecycle_config_propagates_to_installed_provider() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat", MemoryKind::Local, false);
        manager
            .set_chat(chat.clone(), remote_config())
            .await
            .unwrap();
        assert!(!chat.coexist());

        manager
            .set_lifecycle_config(LifecycleConfig {
                chat_coexist: true,
                embedding_coexist: false,
            })
            .await;
        assert!(chat.coexist());
    }

    #[tokio::test]
    async fn clear_chat_unloads_and_clears_config() {
        let manager = ModelManager::new();
        let chat = TestChatProvider::new("chat", MemoryKind::Local, false);
        manager
            .set_chat(chat.clone(), remote_config())
            .await
            .unwrap();
        manager.clear_chat().await;
        assert_eq!(chat.unload_calls(), 1);
        assert!(manager.chat_config().await.is_none());
        assert!(!manager.chat_ready().await);
    }

    #[tokio::test]
    async fn reaper_skips_chat_when_focus_held() {
        let manager = Arc::new(ModelManager::new());
        let chat = TestChatProvider::new("chat", MemoryKind::Local, false);
        manager
            .set_chat(chat.clone(), remote_config())
            .await
            .unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();

        // Pretend the model has been idle long enough to be reaped.
        manager.chat_last_activity.store(1, Ordering::Relaxed);
        manager.set_research_focused(true);

        manager.reap_idle().await;
        assert_eq!(chat.unload_calls(), 0);

        manager.set_research_focused(false);
        manager.reap_idle().await;
        assert_eq!(chat.unload_calls(), 1);
    }

    #[tokio::test]
    async fn reaper_skips_remote_providers() {
        let manager = Arc::new(ModelManager::new());
        let remote = TestChatProvider::new("remote", MemoryKind::Remote, true);
        manager
            .set_chat(remote.clone(), remote_config())
            .await
            .unwrap();
        let _ = manager.acquire_chat().await.unwrap().unwrap();

        manager.chat_last_activity.store(1, Ordering::Relaxed);
        manager.reap_idle().await;
        assert_eq!(remote.unload_calls(), 0);
    }

    #[tokio::test]
    async fn set_research_focused_is_idempotent() {
        let manager = ModelManager::new();
        let mut guard = manager.focus_guard();

        manager.set_research_focused(true);
        // Second call with same value should not fire a change.
        manager.set_research_focused(true);

        // Flip to false: guard.wait_until_released() returns.
        manager.set_research_focused(false);
        tokio::time::timeout(Duration::from_millis(100), guard.wait_until_released())
            .await
            .expect("wait_until_released did not complete in time");
    }

    #[tokio::test]
    async fn focus_guard_returns_immediately_when_not_focused() {
        let manager = ModelManager::new();
        let mut guard = manager.focus_guard();
        // Never set focus — should return immediately.
        tokio::time::timeout(Duration::from_millis(50), guard.wait_until_released())
            .await
            .expect("should not block when focus is false");
    }

    #[tokio::test]
    async fn focus_guard_blocks_then_releases() {
        let manager = Arc::new(ModelManager::new());
        manager.set_research_focused(true);
        let mut guard = manager.focus_guard();

        let m = manager.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            m.set_research_focused(false);
        });

        tokio::time::timeout(Duration::from_millis(500), guard.wait_until_released())
            .await
            .expect("guard should release after focus flips to false");
    }
}
