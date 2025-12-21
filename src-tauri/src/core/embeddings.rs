//! Text embedding using candle and HuggingFace BGE models

use std::path::Path;

use anyhow::{Context, Result};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use hf_hub::api::sync::Api;
use tokenizers::Tokenizer;

/// Text embedder using a BERT-style model (e.g., BGE)
pub struct Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    hidden_size: usize,
    normalize: bool,
}

impl Embedder {
    /// Load an embedding model from HuggingFace Hub
    ///
    /// The model will be downloaded if not already cached.
    pub fn from_hf(repo_id: &str) -> Result<Self> {
        let device = Device::Cpu;

        let api = Api::new().context("Failed to create HuggingFace API")?;
        let repo = api.model(repo_id.to_string());

        // Load config
        let config_path = repo.get("config.json").context("Failed to get config.json")?;
        let config: Config = serde_json::from_str(&std::fs::read_to_string(&config_path)?)
            .context("Failed to parse config.json")?;

        // Load tokenizer
        let tokenizer_path = repo
            .get("tokenizer.json")
            .context("Failed to get tokenizer.json")?;
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Load model weights
        let weights_path = repo
            .get("model.safetensors")
            .context("Failed to get model.safetensors")?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
                .context("Failed to load model weights")?
        };

        let hidden_size = config.hidden_size;
        let model = BertModel::load(vb, &config).context("Failed to load BERT model")?;

        Ok(Self {
            model,
            tokenizer,
            device,
            hidden_size,
            normalize: true,
        })
    }

    /// Load an embedding model from a local directory
    pub fn from_path(model_dir: &Path) -> Result<Self> {
        let device = Device::Cpu;

        // Load config
        let config_path = model_dir.join("config.json");
        let config: Config = serde_json::from_str(&std::fs::read_to_string(&config_path)?)
            .context("Failed to parse config.json")?;

        // Load tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer =
            Tokenizer::from_file(&tokenizer_path).map_err(|e| anyhow::anyhow!("{}", e))?;

        // Load model weights
        let weights_path = model_dir.join("model.safetensors");
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
                .context("Failed to load model weights")?
        };

        let hidden_size = config.hidden_size;
        let model = BertModel::load(vb, &config).context("Failed to load BERT model")?;

        Ok(Self {
            model,
            tokenizer,
            device,
            hidden_size,
            normalize: true,
        })
    }

    /// Generate embedding for a single text
    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let embeddings = self.embed_batch(&[text])?;
        Ok(embeddings.into_iter().next().unwrap())
    }

    /// Generate embeddings for multiple texts
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(vec![]);
        }

        // Tokenize
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| anyhow::anyhow!("Tokenization failed: {}", e))?;

        // Find max length for padding
        let max_len = encodings.iter().map(|e| e.get_ids().len()).max().unwrap_or(0);

        // Prepare input tensors
        let mut input_ids_vec = Vec::new();
        let mut attention_mask_vec = Vec::new();
        let mut token_type_ids_vec = Vec::new();

        for encoding in &encodings {
            let ids = encoding.get_ids();
            let mask = encoding.get_attention_mask();
            let type_ids = encoding.get_type_ids();

            // Pad to max_len
            let mut padded_ids = ids.to_vec();
            let mut padded_mask = mask.to_vec();
            let mut padded_type_ids = type_ids.to_vec();

            padded_ids.resize(max_len, 0);
            padded_mask.resize(max_len, 0);
            padded_type_ids.resize(max_len, 0);

            input_ids_vec.push(padded_ids);
            attention_mask_vec.push(padded_mask);
            token_type_ids_vec.push(padded_type_ids);
        }

        let batch_size = texts.len();

        let input_ids = Tensor::new(
            input_ids_vec
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &self.device,
        )?
        .reshape((batch_size, max_len))?;

        let attention_mask = Tensor::new(
            attention_mask_vec
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &self.device,
        )?
        .reshape((batch_size, max_len))?;

        let token_type_ids = Tensor::new(
            token_type_ids_vec
                .iter()
                .flatten()
                .copied()
                .collect::<Vec<_>>(),
            &self.device,
        )?
        .reshape((batch_size, max_len))?;

        // Run model
        let embeddings = self
            .model
            .forward(&input_ids, &token_type_ids, Some(&attention_mask))?;

        // Mean pooling over tokens (masked)
        let mask_expanded = attention_mask
            .unsqueeze(2)?
            .to_dtype(embeddings.dtype())?
            .broadcast_as(embeddings.shape())?;

        let sum_embeddings = (embeddings * &mask_expanded)?.sum(1)?;
        let sum_mask = mask_expanded.sum(1)?.clamp(1e-9, f64::MAX)?;
        let mean_embeddings = (sum_embeddings / sum_mask)?;

        // Normalize if requested
        let final_embeddings = if self.normalize {
            let norms = mean_embeddings
                .sqr()?
                .sum_keepdim(1)?
                .sqrt()?
                .clamp(1e-9, f64::MAX)?;
            let shape = mean_embeddings.shape().clone();
            (mean_embeddings / norms.broadcast_as(&shape)?)?
        } else {
            mean_embeddings
        };

        // Convert to Vec<Vec<f32>>
        let final_embeddings = final_embeddings.to_dtype(DType::F32)?;
        let flat: Vec<f32> = final_embeddings.flatten_all()?.to_vec1()?;
        let dim = flat.len() / batch_size;

        Ok(flat.chunks(dim).map(|c| c.to_vec()).collect())
    }

    /// Get the embedding dimension
    pub fn dimension(&self) -> usize {
        self.hidden_size
    }
}

#[cfg(test)]
mod tests {
    // Integration tests would require downloading models
    // Skip for unit tests
}
