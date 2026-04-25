//! Local OCR provider using mistralrs vision-language models.
//!
//! Construction is cheap: the HuggingFace repo id and per-model prompt are
//! recorded. Weights load on [`Provider::ensure_loaded`].

use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
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
                let model = MultimodalModelBuilder::new(&hf_repo_id)
                    .with_logging()
                    .build()
                    .await
                    .context("Failed to load multimodal model")?;
                tracing::info!("OCR model '{}' loaded", model_id);
                Ok(model)
            })
            .await
            .map(|_| ())
    }

    async fn unload(&self) -> Result<()> {
        if self.state.unload().await {
            tracing::info!("Unloaded OCR model '{}'", self.state.model_id());
        }
        Ok(())
    }
}

#[async_trait]
impl OcrProvider for LocalOcrProvider {
    async fn ocr_pages(&self, pages: Vec<DynamicImage>) -> Result<Vec<String>> {
        if pages.is_empty() {
            return Ok(Vec::new());
        }

        self.ensure_loaded().await?;
        let model = self.loaded().await?;

        let mut out = Vec::with_capacity(pages.len());
        for (idx, image) in pages.into_iter().enumerate() {
            match run_one_page(&model, &self.prompt, image).await {
                Ok(text) => out.push(text),
                Err(e) => {
                    tracing::warn!(page = idx, error = %e, "OCR page failed; substituting empty");
                    out.push(String::new());
                }
            }
        }
        Ok(out)
    }
}

async fn run_one_page(model: &Model, prompt: &str, image: DynamicImage) -> Result<String> {
    let messages =
        MultimodalMessages::new().add_image_message(TextMessageRole::User, prompt, vec![image]);

    let mut stream = model.stream_chat_request(messages).await?;
    let mut text = String::new();

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

    Ok(text)
}
