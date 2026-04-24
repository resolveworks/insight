//! Shared plumbing for local providers: a lazily-loaded weight slot plus the
//! coexist flag.
//!
//! Both [`super::LocalChatProvider`] and [`super::LocalEmbeddingProvider`]
//! compose a `LocalModelState<T>` — the generic parameter is whatever the
//! provider treats as "loaded weights" (an `Arc<Model>` for chat; a
//! tokenizer+model bundle for embedding).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::RwLock;

/// Lazily-loaded weights for a local provider.
pub(crate) struct LocalModelState<T: Send + Sync> {
    model_id: String,
    loaded: RwLock<Option<Arc<T>>>,
    coexist: AtomicBool,
}

impl<T: Send + Sync> LocalModelState<T> {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
            loaded: RwLock::new(None),
            coexist: AtomicBool::new(false),
        }
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn coexist(&self) -> bool {
        self.coexist.load(Ordering::Relaxed)
    }

    pub fn set_coexist(&self, v: bool) {
        if self.coexist.load(Ordering::Relaxed) != v {
            self.coexist.store(v, Ordering::Relaxed);
        }
    }

    pub async fn is_loaded(&self) -> bool {
        self.loaded.read().await.is_some()
    }

    pub async fn current(&self) -> Option<Arc<T>> {
        self.loaded.read().await.clone()
    }

    /// Return the loaded value, or run `loader` under a write lock and
    /// install its result. Concurrent callers see the same `Arc` and the
    /// loader runs at most once per uninstalled/unloaded cycle.
    pub async fn get_or_load<F, Fut>(&self, loader: F) -> Result<Arc<T>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        if let Some(t) = self.loaded.read().await.clone() {
            return Ok(t);
        }
        let mut guard = self.loaded.write().await;
        if let Some(t) = guard.clone() {
            return Ok(t);
        }
        let arc = Arc::new(loader().await?);
        *guard = Some(arc.clone());
        Ok(arc)
    }

    /// Drop the loaded weights. Returns whether anything was unloaded — the
    /// caller uses it to decide whether to log / emit an event.
    pub async fn unload(&self) -> bool {
        self.loaded.write().await.take().is_some()
    }
}
