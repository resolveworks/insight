//! Local OCR provider using mistralrs vision-language models.
//!
//! Construction is cheap: the HuggingFace repo id and per-model prompt are
//! recorded. Weights load on [`Provider::ensure_loaded`].

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use hf_hub::Cache;
use image::DynamicImage;
use mistralrs::{
    ChatCompletionChunkResponse, Delta, Model, MultimodalMessages, MultimodalModelBuilder,
    Response, TextMessageRole,
};

use crate::provider::{MemoryKind, OcrProvider, Provider};

use super::LocalModelState;

/// Local OCR provider backed by a mistralrs multimodal model.
///
/// Loads on first inference and unloads on demand via the shared
/// [`LocalModelState`] plumbing. Inference iterates per page — mistralrs
/// drives one chat request at a time, and per-page errors are demoted to
/// empty strings so a single bad scan doesn't kill the whole document.
pub struct LocalOcrProvider {
    hf_repo_id: String,
    prompt: String,
    state: LocalModelState<Model>,
}

impl LocalOcrProvider {
    pub fn new(model_id: &str, hf_repo_id: &str, prompt: &str) -> Self {
        Self {
            hf_repo_id: hf_repo_id.to_string(),
            prompt: prompt.to_string(),
            state: LocalModelState::new(model_id),
        }
    }

    async fn loaded(&self) -> Result<Arc<Model>> {
        self.state
            .current()
            .await
            .ok_or_else(|| anyhow::anyhow!("Local OCR model not loaded"))
    }
}

#[async_trait]
impl Provider for LocalOcrProvider {
    fn provider_name(&self) -> &'static str {
        "local"
    }

    fn model_id(&self) -> &str {
        self.state.model_id()
    }

    fn memory_kind(&self) -> MemoryKind {
        MemoryKind::Local
    }

    fn coexist(&self) -> bool {
        self.state.coexist()
    }

    fn set_coexist(&self, coexist: bool) {
        self.state.set_coexist(coexist);
    }

    async fn is_loaded(&self) -> bool {
        self.state.is_loaded().await
    }

    async fn ensure_loaded(&self) -> Result<()> {
        let hf_repo_id = self.hf_repo_id.clone();
        let model_id = self.state.model_id().to_string();

        self.state
            .get_or_load(|| async move {
                tracing::info!("Loading OCR model '{}' ({})", model_id, hf_repo_id);

                // mistralrs v0.8's Qwen2_5VLConfig requires `tie_word_embeddings`
                // at the top level. Newer transformers nest it under
                // `text_config`. Patch the cached config.json before the
                // builder reads it. Idempotent: no-op if the field is already
                // present at the root.
                if let Err(e) = patch_multimodal_config(&hf_repo_id) {
                    tracing::warn!(
                        repo = %hf_repo_id,
                        error = ?e,
                        "Failed to pre-patch config.json; load may fail",
                    );
                }

                let model = MultimodalModelBuilder::new(&hf_repo_id)
                    .with_logging()
                    .build()
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to load multimodal model {} ({})",
                            model_id, hf_repo_id
                        )
                    })?;
                tracing::info!("OCR model '{}' loaded", model_id);
                Ok(model)
            })
            .await
            .map(|_| ())
    }

    async fn unload(&self) -> Result<bool> {
        let did = self.state.unload().await;
        if did {
            tracing::info!("Unloaded OCR model '{}'", self.state.model_id());
        }
        Ok(did)
    }
}

#[async_trait]
impl OcrProvider for LocalOcrProvider {
    async fn ocr_page(&self, image: DynamicImage) -> Result<String> {
        self.ensure_loaded().await?;
        let model = self.loaded().await?;
        run_one_page(&model, &self.prompt, image).await
    }
}

/// Bridge the schema gap between modern HF Qwen2.5-VL configs (which nest
/// `tie_word_embeddings` under `text_config`) and mistralrs v0.8's
/// `Qwen2_5VLConfig` (which expects it at the root and has no serde
/// default). Walks the HF cache to find the snapshot's `config.json`,
/// adds the missing top-level field if needed, and writes back in place.
///
/// Safe to call repeatedly: the second invocation finds the field
/// already present and returns without touching the file.
///
/// Once mistralrs is bumped to a version that follows the new schema
/// this entire function can go away.
fn patch_multimodal_config(hf_repo_id: &str) -> Result<()> {
    let cache = Cache::from_env();
    let path: PathBuf = cache
        .model(hf_repo_id.to_string())
        .get("config.json")
        .ok_or_else(|| anyhow::anyhow!("config.json not found in HF cache"))?;

    let raw = std::fs::read_to_string(&path).context("read config.json")?;
    let mut cfg: serde_json::Value = serde_json::from_str(&raw).context("parse config.json")?;

    let obj = cfg
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("config.json root is not an object"))?;

    if obj.contains_key("tie_word_embeddings") {
        return Ok(());
    }

    let nested = obj
        .get("text_config")
        .and_then(|tc| tc.get("tie_word_embeddings"))
        .cloned()
        .unwrap_or(serde_json::Value::Bool(false));

    obj.insert("tie_word_embeddings".to_string(), nested);

    let patched = serde_json::to_string_pretty(&cfg).context("serialize patched config")?;
    std::fs::write(&path, patched).context("write patched config.json")?;

    tracing::info!(
        path = %path.display(),
        "Patched config.json: hoisted tie_word_embeddings to top level"
    );
    Ok(())
}

/// Per-page OCR timeout. A single page shouldn't need more than 5 minutes;
/// if it exceeds this the model is likely in a hallucination loop and we
/// cut it short so the rest of the document can proceed.
pub(crate) const OCR_PAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

async fn run_one_page(model: &Model, prompt: &str, image: DynamicImage) -> Result<String> {
    let (w, h) = (image.width(), image.height());
    tracing::info!(
        image_dims = %format!("{}x{}", w, h),
        "OCR: starting page inference"
    );

    let messages =
        MultimodalMessages::new().add_image_message(TextMessageRole::User, prompt, vec![image]);

    let t0 = std::time::Instant::now();
    let mut stream = model.stream_chat_request(messages).await?;
    let mut text = String::new();
    let mut token_count: usize = 0;

    while let Some(chunk) = stream.next().await {
        match chunk {
            Response::Chunk(ChatCompletionChunkResponse { choices, .. }) => {
                if let Some(choice) = choices.first() {
                    let Delta {
                        content: delta_content,
                        ..
                    } = &choice.delta;
                    if let Some(t) = delta_content {
                        text.push_str(t);
                        token_count += 1;
                        if token_count.is_multiple_of(500) {
                            tracing::debug!(
                                tokens = token_count,
                                elapsed_secs = t0.elapsed().as_secs_f32(),
                                "OCR: still generating…"
                            );
                        }
                    }
                }
            }
            Response::Done(_) => break,
            Response::ModelError(msg, _) => {
                return Err(anyhow::anyhow!("Model error: {}", msg));
            }
            _ => {}
        }
    }

    tracing::info!(
        tokens = token_count,
        elapsed_secs = t0.elapsed().as_secs_f32(),
        output_len = text.len(),
        "OCR: page completed"
    );
    Ok(text)
}
