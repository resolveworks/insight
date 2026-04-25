use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hf_hub::api::tokio::{Api, ApiBuilder};
use hf_hub::Cache;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::sync::mpsc;

// ============================================================================
// ModelSpec Trait
// ============================================================================

/// Trait for model specifications (both language and embedding models)
///
/// This provides a unified interface for model metadata and download requirements
/// while allowing different model types to have their own specific fields.
pub trait ModelSpec: Clone + Serialize + DeserializeOwned + Send + Sync {
    /// Unique identifier for this model
    fn id(&self) -> &str;

    /// Display name for the UI
    fn name(&self) -> &str;

    /// User-facing description
    fn description(&self) -> &str;

    /// Approximate download size in GB (for display)
    fn size_gb(&self) -> f32;

    /// Files required for this model: (repo_id, filename)
    fn required_files(&self) -> Vec<(String, String)>;

    /// Primary repository ID (used for path resolution)
    fn primary_repo(&self) -> &str;

    /// Primary file within the repo (parent dir becomes model path)
    fn primary_file(&self) -> &str;
}

// ============================================================================
// Language Models (LLM)
// ============================================================================

/// Information about a GGUF language model
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguageModelInfo {
    /// Unique identifier for this model
    pub id: String,
    /// Display name
    pub name: String,
    /// Description for the user
    pub description: String,
    /// Approximate size in GB (for display)
    pub size_gb: f32,
    /// HuggingFace repo ID containing GGUF files (e.g., "Qwen/Qwen3-8B-GGUF")
    pub gguf_repo_id: String,
    /// GGUF filename within the repo
    pub gguf_file: String,
    /// Repo ID for tokenizer (e.g., "Qwen/Qwen3-8B")
    pub tokenizer_repo_id: String,
}

impl ModelSpec for LanguageModelInfo {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn size_gb(&self) -> f32 {
        self.size_gb
    }

    fn required_files(&self) -> Vec<(String, String)> {
        vec![
            (self.gguf_repo_id.clone(), self.gguf_file.clone()),
            (self.tokenizer_repo_id.clone(), "tokenizer.json".to_string()),
            (
                self.tokenizer_repo_id.clone(),
                "tokenizer_config.json".to_string(),
            ),
        ]
    }

    fn primary_repo(&self) -> &str {
        &self.gguf_repo_id
    }

    fn primary_file(&self) -> &str {
        &self.gguf_file
    }
}

/// Get the default language model
pub fn default_language_model() -> LanguageModelInfo {
    available_language_models().into_iter().next().unwrap()
}

/// Get a language model by ID
pub fn get_language_model(id: &str) -> Option<LanguageModelInfo> {
    available_language_models().into_iter().find(|m| m.id == id)
}

/// Available language models registry
pub fn available_language_models() -> Vec<LanguageModelInfo> {
    vec![
        // Default: Qwen3-8B Q4_K_M - Best balance of size and quality
        LanguageModelInfo {
            id: "qwen3-8b-q4km".to_string(),
            name: "Qwen3 8B (Q4_K_M)".to_string(),
            description: "Recommended. Fast 8B model with tool calling. ~5GB download.".to_string(),
            size_gb: 5.0,
            gguf_repo_id: "Qwen/Qwen3-8B-GGUF".to_string(),
            gguf_file: "Qwen3-8B-Q4_K_M.gguf".to_string(),
            tokenizer_repo_id: "Qwen/Qwen3-8B".to_string(),
        },
        // Smaller option for constrained systems
        LanguageModelInfo {
            id: "qwen3-4b-q4km".to_string(),
            name: "Qwen3 4B (Q4_K_M)".to_string(),
            description: "Lighter model for systems with less memory. ~2.5GB download.".to_string(),
            size_gb: 2.5,
            gguf_repo_id: "Qwen/Qwen3-4B-GGUF".to_string(),
            gguf_file: "Qwen3-4B-Q4_K_M.gguf".to_string(),
            tokenizer_repo_id: "Qwen/Qwen3-4B".to_string(),
        },
        // Higher quality option
        LanguageModelInfo {
            id: "qwen3-8b-q8".to_string(),
            name: "Qwen3 8B (Q8_0)".to_string(),
            description: "Higher quality 8-bit quantization. Better accuracy. ~8.5GB download."
                .to_string(),
            size_gb: 8.5,
            gguf_repo_id: "Qwen/Qwen3-8B-GGUF".to_string(),
            gguf_file: "Qwen3-8B-Q8_0.gguf".to_string(),
            tokenizer_repo_id: "Qwen/Qwen3-8B".to_string(),
        },
    ]
}

// ============================================================================
// Embedding Models
// ============================================================================

/// Information about an embedding model for semantic search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingModelInfo {
    /// Unique identifier for this model
    pub id: String,
    /// Display name
    pub name: String,
    /// Description for the user
    pub description: String,
    /// Approximate size in GB (for display)
    pub size_gb: f32,
    /// HuggingFace repo ID (e.g., "Qwen/Qwen3-Embedding-0.6B")
    pub hf_repo_id: String,
    /// Vector dimensions produced by this model
    pub dimensions: usize,
}

impl ModelSpec for EmbeddingModelInfo {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn size_gb(&self) -> f32 {
        self.size_gb
    }

    fn required_files(&self) -> Vec<(String, String)> {
        vec![
            (self.hf_repo_id.clone(), "config.json".to_string()),
            (self.hf_repo_id.clone(), "model.safetensors".to_string()),
            (self.hf_repo_id.clone(), "tokenizer.json".to_string()),
            (self.hf_repo_id.clone(), "tokenizer_config.json".to_string()),
        ]
    }

    fn primary_repo(&self) -> &str {
        &self.hf_repo_id
    }

    fn primary_file(&self) -> &str {
        "config.json"
    }
}

/// Get the default embedding model
pub fn default_embedding_model() -> EmbeddingModelInfo {
    available_embedding_models().into_iter().next().unwrap()
}

/// Get an embedding model by ID
pub fn get_embedding_model(id: &str) -> Option<EmbeddingModelInfo> {
    available_embedding_models()
        .into_iter()
        .find(|m| m.id == id)
}

/// Available embedding models registry
/// Note: Only models supported by mistralrs are listed
pub fn available_embedding_models() -> Vec<EmbeddingModelInfo> {
    vec![
        // Default: Qwen3 Embedding - Apache 2.0, not gated
        EmbeddingModelInfo {
            id: "qwen3-embedding".to_string(),
            name: "Qwen3 Embedding 0.6B".to_string(),
            description: "Recommended. High quality Qwen3-based embedding model.".to_string(),
            size_gb: 1.2,
            hf_repo_id: "Qwen/Qwen3-Embedding-0.6B".to_string(),
            dimensions: 1024,
        },
    ]
}

// ============================================================================
// OCR Models
// ============================================================================

/// Information about a vision-language OCR model.
///
/// All current OCR models are sharded multimodal HF repos (Qwen2.5-VL or
/// Qwen3-VL fine-tunes) — `weight_shards` captures the shard count so we
/// can list each `model-{i:05}-of-{N:05}.safetensors` shard explicitly.
/// Without listing them, `is_downloaded()` would return true as soon as
/// the small config files land, while mistralrs would still need to fetch
/// multi-GB of weights on first load.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OcrModelInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub size_gb: f32,
    pub hf_repo_id: String,
    /// Per-model OCR prompt. Each fine-tune has a recommended prompt; we
    /// don't auto-augment with anchor-text harnesses (deferred — see #23).
    pub prompt: String,
    /// Number of safetensors shards in the HF repo (e.g. `2` for the
    /// `model-00001-of-00002.safetensors` / `model-00002-of-00002.safetensors`
    /// pair). Used to build the `required_files` list.
    pub weight_shards: u32,
}

impl ModelSpec for OcrModelInfo {
    fn id(&self) -> &str {
        &self.id
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn size_gb(&self) -> f32 {
        self.size_gb
    }

    fn required_files(&self) -> Vec<(String, String)> {
        let repo = self.hf_repo_id.clone();
        let mut files = vec![
            (repo.clone(), "config.json".to_string()),
            (repo.clone(), "tokenizer.json".to_string()),
            (repo.clone(), "tokenizer_config.json".to_string()),
            (repo.clone(), "preprocessor_config.json".to_string()),
            (repo.clone(), "generation_config.json".to_string()),
            (repo.clone(), "special_tokens_map.json".to_string()),
            (repo.clone(), "added_tokens.json".to_string()),
            (repo.clone(), "vocab.json".to_string()),
            (repo.clone(), "merges.txt".to_string()),
            (repo.clone(), "chat_template.jinja".to_string()),
            (repo.clone(), "model.safetensors.index.json".to_string()),
        ];
        let n = self.weight_shards;
        for i in 1..=n {
            files.push((
                repo.clone(),
                format!("model-{:05}-of-{:05}.safetensors", i, n),
            ));
        }
        files
    }

    fn primary_repo(&self) -> &str {
        &self.hf_repo_id
    }

    fn primary_file(&self) -> &str {
        "config.json"
    }
}

/// Default OCR model. None is configured on startup — OCR is opt-in
/// via Settings; this is just the "Recommended" entry shown to the user.
pub fn default_ocr_model() -> OcrModelInfo {
    available_ocr_models().into_iter().next().unwrap()
}

pub fn get_ocr_model(id: &str) -> Option<OcrModelInfo> {
    available_ocr_models().into_iter().find(|m| m.id == id)
}

/// Available OCR models registry.
///
/// All run on mistralrs's existing Qwen2.5-VL / Qwen3-VL pipelines.
pub fn available_ocr_models() -> Vec<OcrModelInfo> {
    vec![
        OcrModelInfo {
            id: "nanonets-ocr2-3b".to_string(),
            name: "Nanonets-OCR2 3B".to_string(),
            description: "Recommended. Qwen2.5-VL fine-tune for document OCR. ~6GB download."
                .to_string(),
            size_gb: 6.0,
            hf_repo_id: "nanonets/Nanonets-OCR2-3B".to_string(),
            prompt: "Extract the text from the above document as if you were reading \
                     it naturally. Return tables in markdown. Return equations in \
                     LaTeX. Watermarks should be wrapped in <watermark></watermark>. \
                     Page numbers should be wrapped in <page_number></page_number>. \
                     Prefer using ☐ and ☑ for check boxes."
                .to_string(),
            weight_shards: 2,
        },
        OcrModelInfo {
            id: "olmocr2-7b".to_string(),
            name: "olmOCR-2 7B (1025)".to_string(),
            description: "Higher accuracy. Qwen2.5-VL 7B fine-tune. ~15GB download.".to_string(),
            size_gb: 15.0,
            hf_repo_id: "allenai/olmOCR-2-7B-1025".to_string(),
            prompt: "Attempt to read the document and convert it to markdown. \
                     Return tables in markdown. Skip headers, footers, and page numbers."
                .to_string(),
            weight_shards: 4,
        },
        OcrModelInfo {
            id: "chandra".to_string(),
            name: "Chandra-OCR".to_string(),
            description: "SOTA accuracy. Qwen3-VL 9B. ~18GB download. \
                          High VRAM required."
                .to_string(),
            size_gb: 18.0,
            hf_repo_id: "datalab-to/chandra".to_string(),
            prompt: "Convert this document to markdown.".to_string(),
            weight_shards: 4,
        },
    ]
}

// ============================================================================
// Download Progress
// ============================================================================

/// Download progress event
#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    /// Current file being downloaded
    pub file: String,
    /// Bytes downloaded for current file
    pub downloaded: u64,
    /// Total bytes for current file
    pub total: u64,
    /// Overall progress (0.0 to 1.0)
    pub overall_progress: f32,
    /// Current file index (1-based)
    pub file_index: usize,
    /// Total number of files
    pub total_files: usize,
}

/// Minimum interval between progress updates (100ms)
const PROGRESS_THROTTLE: Duration = Duration::from_millis(100);

/// Shared state for progress tracking across parallel chunk downloads
struct SharedProgress {
    file: Mutex<String>,
    downloaded: AtomicU64,
    total: AtomicU64,
    last_emit: Mutex<Instant>,
}

/// Progress tracker that sends events via channel (throttled)
/// Uses Arc for shared state since hf-hub clones the tracker for parallel chunk downloads
#[derive(Clone)]
struct ProgressTracker {
    file_index: usize,
    total_files: usize,
    shared: Arc<SharedProgress>,
    tx: mpsc::Sender<DownloadProgress>,
}

impl hf_hub::api::tokio::Progress for ProgressTracker {
    async fn init(&mut self, size: usize, filename: &str) {
        self.shared.total.store(size as u64, Ordering::SeqCst);
        self.shared.downloaded.store(0, Ordering::SeqCst);
        *self.shared.file.lock().unwrap() = filename.to_string();
        // Always emit on file start
        self.emit_progress().await;
    }

    async fn update(&mut self, size: usize) {
        // Atomically add to the shared downloaded counter
        self.shared
            .downloaded
            .fetch_add(size as u64, Ordering::SeqCst);

        // Throttle updates to avoid flooding the UI
        let should_emit = {
            let last = self.shared.last_emit.lock().unwrap();
            last.elapsed() >= PROGRESS_THROTTLE
        };
        if should_emit {
            self.emit_progress().await;
        }
    }

    async fn finish(&mut self) {
        // Always emit on file complete
        self.emit_progress().await;
    }
}

impl ProgressTracker {
    async fn emit_progress(&self) {
        let downloaded = self.shared.downloaded.load(Ordering::SeqCst);
        let total = self.shared.total.load(Ordering::SeqCst);
        let file = self.shared.file.lock().unwrap().clone();

        let progress = DownloadProgress {
            file,
            downloaded,
            total,
            overall_progress: self.calculate_overall(downloaded, total),
            file_index: self.file_index,
            total_files: self.total_files,
        };
        let _ = self.tx.send(progress).await;
        *self.shared.last_emit.lock().unwrap() = Instant::now();
    }

    fn calculate_overall(&self, downloaded: u64, total: u64) -> f32 {
        let files_done = (self.file_index - 1) as f32;
        let current_file_progress = if total > 0 {
            downloaded as f32 / total as f32
        } else {
            0.0
        };
        (files_done + current_file_progress) / self.total_files as f32
    }
}

// ============================================================================
// Model Downloader
// ============================================================================

/// Downloads and caches HuggingFace models on disk.
///
/// Orthogonal to `crate::manager::ModelManager`: this cares about bytes on
/// disk, while `ModelManager` cares about in-memory inference state.
pub struct ModelDownloader {
    cache: Cache,
    api: Api,
}

impl ModelDownloader {
    /// Create a new downloader using the HuggingFace cache
    ///
    /// Respects the `HF_HOME` environment variable if set, otherwise uses
    /// the default cache location (`~/.cache/huggingface/hub`).
    pub async fn new() -> Result<Self> {
        let cache = Cache::from_env();
        let api = ApiBuilder::new()
            .build()
            .context("Failed to create HuggingFace API client")?;

        Ok(Self { cache, api })
    }

    /// Check if a model is fully downloaded
    pub fn is_downloaded<M: ModelSpec>(&self, model: &M) -> bool {
        for (repo_id, filename) in model.required_files() {
            let repo_cache = self.cache.model(repo_id);
            if repo_cache.get(&filename).is_none() {
                return false;
            }
        }
        true
    }

    /// Get the cache path for a model (if downloaded)
    ///
    /// Returns the parent directory of the primary file.
    pub fn get_path<M: ModelSpec>(&self, model: &M) -> Option<PathBuf> {
        if !self.is_downloaded(model) {
            return None;
        }

        self.cache
            .model(model.primary_repo().to_string())
            .get(model.primary_file())
            .map(|p| p.parent().unwrap().to_path_buf())
    }

    /// Download a model with status and progress tracking
    ///
    /// Downloads all required files and returns the model directory path.
    /// Emits status events (Downloading, Ready, Failed) and progress events.
    pub async fn download<M: ModelSpec>(
        &self,
        model: &M,
        model_type: crate::ModelType,
        status_tx: mpsc::Sender<crate::ModelStatus>,
        progress_tx: mpsc::Sender<crate::ModelDownloadProgress>,
    ) -> Result<PathBuf> {
        // Emit downloading status
        let _ = status_tx
            .send(crate::ModelStatus::Downloading {
                model_type,
                model_id: model.id().to_string(),
                model_name: model.name().to_string(),
            })
            .await;

        let all_files = model.required_files();
        let total_files = all_files.len();

        for (idx, (repo_id, filename)) in all_files.iter().enumerate() {
            let repo = self.api.model(repo_id.clone());

            // Create a channel to receive raw progress, then wrap with model_type
            let (raw_tx, mut raw_rx) = mpsc::channel::<DownloadProgress>(100);
            let wrapped_tx = progress_tx.clone();

            // Forward progress with model_type wrapper
            tokio::spawn(async move {
                while let Some(progress) = raw_rx.recv().await {
                    let _ = wrapped_tx
                        .send(crate::ModelDownloadProgress {
                            model_type,
                            progress,
                        })
                        .await;
                }
            });

            let shared = Arc::new(SharedProgress {
                file: Mutex::new(filename.clone()),
                downloaded: AtomicU64::new(0),
                total: AtomicU64::new(0),
                last_emit: Mutex::new(Instant::now()),
            });

            let progress = ProgressTracker {
                file_index: idx + 1,
                total_files,
                shared,
                tx: raw_tx,
            };

            if let Err(e) = repo.download_with_progress(filename, progress).await {
                let error = format!("Failed to download {}: {}", filename, e);
                let _ = status_tx
                    .send(crate::ModelStatus::Failed {
                        model_type,
                        model_id: model.id().to_string(),
                        error: error.clone(),
                    })
                    .await;
                return Err(anyhow::anyhow!(error));
            }
        }

        // Return the directory containing the primary file
        let model_dir = self
            .cache
            .model(model.primary_repo().to_string())
            .get(model.primary_file())
            .context("Primary file not found after download")?
            .parent()
            .context("Invalid path")?
            .to_path_buf();

        Ok(model_dir)
    }

    /// Get the HuggingFace cache directory path
    pub fn cache_path(&self) -> PathBuf {
        self.cache.path().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_language_models() {
        let models = available_language_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "qwen3-8b-q4km");
    }

    #[test]
    fn test_get_language_model() {
        let model = get_language_model("qwen3-4b-q4km");
        assert!(model.is_some());
        assert_eq!(model.unwrap().name, "Qwen3 4B (Q4_K_M)");

        let missing = get_language_model("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_default_language_model() {
        let model = default_language_model();
        assert_eq!(model.id, "qwen3-8b-q4km");
        assert!(!model.gguf_file.is_empty());
    }

    #[test]
    fn test_language_model_spec() {
        let model = default_language_model();
        assert_eq!(model.id(), "qwen3-8b-q4km");
        assert_eq!(model.name(), "Qwen3 8B (Q4_K_M)");
        assert_eq!(model.required_files().len(), 3);
        assert_eq!(model.primary_repo(), "Qwen/Qwen3-8B-GGUF");
        assert_eq!(model.primary_file(), "Qwen3-8B-Q4_K_M.gguf");
    }

    #[test]
    fn test_available_embedding_models() {
        let models = available_embedding_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "qwen3-embedding");
        assert_eq!(models[0].dimensions, 1024);
    }

    #[test]
    fn test_get_embedding_model() {
        let model = get_embedding_model("qwen3-embedding");
        assert!(model.is_some());
        assert_eq!(model.unwrap().dimensions, 1024);

        let missing = get_embedding_model("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_default_embedding_model() {
        let model = default_embedding_model();
        assert_eq!(model.id, "qwen3-embedding");
        assert_eq!(model.hf_repo_id, "Qwen/Qwen3-Embedding-0.6B");
    }

    #[test]
    fn test_embedding_model_spec() {
        let model = default_embedding_model();
        assert_eq!(model.id(), "qwen3-embedding");
        assert_eq!(model.name(), "Qwen3 Embedding 0.6B");
        assert_eq!(model.required_files().len(), 4);
        assert_eq!(model.primary_repo(), "Qwen/Qwen3-Embedding-0.6B");
        assert_eq!(model.primary_file(), "config.json");
    }

    #[test]
    fn test_available_ocr_models() {
        let models = available_ocr_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "nanonets-ocr2-3b");
        assert_eq!(models.len(), 3);
    }

    #[test]
    fn test_get_ocr_model() {
        let model = get_ocr_model("nanonets-ocr2-3b");
        assert!(model.is_some());
        assert_eq!(model.unwrap().hf_repo_id, "nanonets/Nanonets-OCR2-3B");

        assert!(get_ocr_model("nonexistent").is_none());
    }

    #[test]
    fn test_default_ocr_model() {
        let model = default_ocr_model();
        assert_eq!(model.id, "nanonets-ocr2-3b");
        assert!(!model.prompt.is_empty());
    }

    #[test]
    fn test_ocr_model_required_files() {
        let m = get_ocr_model("nanonets-ocr2-3b").unwrap();
        let files = m.required_files();
        let names: Vec<&String> = files.iter().map(|(_, f)| f).collect();
        // Two-shard model: index + both shards listed.
        assert!(names
            .iter()
            .any(|n| n.as_str() == "model.safetensors.index.json"));
        assert!(names
            .iter()
            .any(|n| n.as_str() == "model-00001-of-00002.safetensors"));
        assert!(names
            .iter()
            .any(|n| n.as_str() == "model-00002-of-00002.safetensors"));

        let m4 = get_ocr_model("olmocr2-7b").unwrap();
        let names4: Vec<String> = m4.required_files().into_iter().map(|(_, f)| f).collect();
        assert!(names4
            .iter()
            .any(|n| n.as_str() == "model-00004-of-00004.safetensors"));
    }
}
