use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hf_hub::api::tokio::{Api, ApiBuilder};
use hf_hub::Cache;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Information about a GGUF model available for use (LLM)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
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

/// Get the default model
pub fn default_model() -> ModelInfo {
    available_models().into_iter().next().unwrap()
}

/// Get a model by ID
pub fn get_model(id: &str) -> Option<ModelInfo> {
    available_models().into_iter().find(|m| m.id == id)
}

/// Available models registry
pub fn available_models() -> Vec<ModelInfo> {
    vec![
        // Default: Qwen3-8B Q4_K_M - Best balance of size and quality
        ModelInfo {
            id: "qwen3-8b-q4km".to_string(),
            name: "Qwen3 8B (Q4_K_M)".to_string(),
            description: "Recommended. Fast 8B model with tool calling. ~5GB download.".to_string(),
            size_gb: 5.0,
            gguf_repo_id: "Qwen/Qwen3-8B-GGUF".to_string(),
            gguf_file: "Qwen3-8B-Q4_K_M.gguf".to_string(),
            tokenizer_repo_id: "Qwen/Qwen3-8B".to_string(),
        },
        // Smaller option for constrained systems
        ModelInfo {
            id: "qwen3-4b-q4km".to_string(),
            name: "Qwen3 4B (Q4_K_M)".to_string(),
            description: "Lighter model for systems with less memory. ~2.5GB download.".to_string(),
            size_gb: 2.5,
            gguf_repo_id: "Qwen/Qwen3-4B-GGUF".to_string(),
            gguf_file: "Qwen3-4B-Q4_K_M.gguf".to_string(),
            tokenizer_repo_id: "Qwen/Qwen3-4B".to_string(),
        },
        // Higher quality option
        ModelInfo {
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
    /// HuggingFace repo ID (e.g., "BAAI/bge-base-en-v1.5")
    pub hf_repo_id: String,
    /// Vector dimensions produced by this model
    pub dimensions: usize,
}

/// Get the default embedding model
pub fn default_embedding_model() -> EmbeddingModelInfo {
    available_embedding_models().into_iter().next().unwrap()
}

/// Get an embedding model by ID
pub fn get_embedding_model(id: &str) -> Option<EmbeddingModelInfo> {
    available_embedding_models().into_iter().find(|m| m.id == id)
}

/// Available embedding models registry
pub fn available_embedding_models() -> Vec<EmbeddingModelInfo> {
    vec![
        // Default: BGE Base - Good balance
        EmbeddingModelInfo {
            id: "bge-base-en".to_string(),
            name: "BGE Base EN v1.5".to_string(),
            description: "Recommended. Good balance of speed and quality.".to_string(),
            size_gb: 0.44,
            hf_repo_id: "BAAI/bge-base-en-v1.5".to_string(),
            dimensions: 768,
        },
        // Smaller option
        EmbeddingModelInfo {
            id: "bge-small-en".to_string(),
            name: "BGE Small EN v1.5".to_string(),
            description: "Faster, smaller footprint for constrained systems.".to_string(),
            size_gb: 0.13,
            hf_repo_id: "BAAI/bge-small-en-v1.5".to_string(),
            dimensions: 384,
        },
        // Higher quality option
        EmbeddingModelInfo {
            id: "bge-large-en".to_string(),
            name: "BGE Large EN v1.5".to_string(),
            description: "Higher quality, slower. Best for accuracy.".to_string(),
            size_gb: 1.3,
            hf_repo_id: "BAAI/bge-large-en-v1.5".to_string(),
            dimensions: 1024,
        },
    ]
}

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

/// Model download status
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status")]
pub enum ModelStatus {
    /// Model is not downloaded
    NotDownloaded,
    /// Model is being downloaded
    Downloading { progress: DownloadProgress },
    /// Model is fully downloaded and ready
    Ready { path: PathBuf },
    /// Download failed
    Failed { error: String },
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

/// Model manager handles downloading and caching models
pub struct ModelManager {
    cache: Cache,
    api: Api,
}

impl ModelManager {
    /// Create a new model manager with custom cache directory
    pub async fn new(cache_dir: PathBuf) -> Result<Self> {
        let cache = Cache::new(cache_dir);
        let api = ApiBuilder::new()
            .with_cache_dir(cache.path().clone())
            .build()
            .context("Failed to create HuggingFace API client")?;

        Ok(Self { cache, api })
    }

    /// Check if a model is fully downloaded
    pub fn is_downloaded(&self, model: &ModelInfo) -> bool {
        // Check GGUF file is present
        let gguf_cache = self.cache.model(model.gguf_repo_id.clone());
        if gguf_cache.get(&model.gguf_file).is_none() {
            return false;
        }

        // Check tokenizer files
        let tok_cache = self.cache.model(model.tokenizer_repo_id.clone());
        tok_cache.get("tokenizer.json").is_some()
            && tok_cache.get("tokenizer_config.json").is_some()
    }

    /// Get the cache path for a model (if downloaded)
    pub fn get_model_path(&self, model: &ModelInfo) -> Option<PathBuf> {
        if !self.is_downloaded(model) {
            return None;
        }

        // Return the directory containing the GGUF file
        self.cache
            .model(model.gguf_repo_id.clone())
            .get(&model.gguf_file)
            .map(|p| p.parent().unwrap().to_path_buf())
    }

    /// Download a model with progress tracking
    pub async fn download_model(
        &self,
        model: &ModelInfo,
        progress_tx: mpsc::Sender<DownloadProgress>,
    ) -> Result<PathBuf> {
        // Files to download: (repo_id, filename)
        let all_files = [
            (model.gguf_repo_id.clone(), model.gguf_file.clone()),
            (
                model.tokenizer_repo_id.clone(),
                "tokenizer.json".to_string(),
            ),
            (
                model.tokenizer_repo_id.clone(),
                "tokenizer_config.json".to_string(),
            ),
        ];

        let total_files = all_files.len();
        let mut first_path: Option<PathBuf> = None;

        for (idx, (repo_id, filename)) in all_files.iter().enumerate() {
            let repo = self.api.model(repo_id.clone());

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
                tx: progress_tx.clone(),
            };

            let path = repo
                .download_with_progress(filename, progress)
                .await
                .with_context(|| format!("Failed to download {}", filename))?;

            if first_path.is_none() {
                first_path = Some(path);
            }
        }

        // Return the directory containing the GGUF file
        let model_dir = first_path
            .context("No files downloaded")?
            .parent()
            .context("Invalid path")?
            .to_path_buf();

        Ok(model_dir)
    }

    // ========================================================================
    // Embedding Model Methods
    // ========================================================================

    /// Check if an embedding model is fully downloaded
    pub fn is_embedding_model_downloaded(&self, model: &EmbeddingModelInfo) -> bool {
        let cache = self.cache.model(model.hf_repo_id.clone());
        // HuggingFace transformer models need these core files
        cache.get("config.json").is_some()
            && (cache.get("model.safetensors").is_some()
                || cache.get("pytorch_model.bin").is_some())
            && cache.get("tokenizer.json").is_some()
    }

    /// Get the cache path for an embedding model (if downloaded)
    pub fn get_embedding_model_path(&self, model: &EmbeddingModelInfo) -> Option<PathBuf> {
        if !self.is_embedding_model_downloaded(model) {
            return None;
        }

        // Return the directory containing the model files
        self.cache
            .model(model.hf_repo_id.clone())
            .get("config.json")
            .map(|p| p.parent().unwrap().to_path_buf())
    }

    /// Download an embedding model with progress tracking
    pub async fn download_embedding_model(
        &self,
        model: &EmbeddingModelInfo,
        progress_tx: mpsc::Sender<DownloadProgress>,
    ) -> Result<PathBuf> {
        // Files needed for HuggingFace transformer embedding models
        let all_files = [
            (model.hf_repo_id.clone(), "config.json".to_string()),
            (model.hf_repo_id.clone(), "model.safetensors".to_string()),
            (model.hf_repo_id.clone(), "tokenizer.json".to_string()),
            (model.hf_repo_id.clone(), "tokenizer_config.json".to_string()),
        ];

        let total_files = all_files.len();
        let mut first_path: Option<PathBuf> = None;

        for (idx, (repo_id, filename)) in all_files.iter().enumerate() {
            let repo = self.api.model(repo_id.clone());

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
                tx: progress_tx.clone(),
            };

            let path = repo
                .download_with_progress(filename, progress)
                .await
                .with_context(|| format!("Failed to download {}", filename))?;

            if first_path.is_none() {
                first_path = Some(path);
            }
        }

        // Return the directory containing the model files
        let model_dir = first_path
            .context("No files downloaded")?
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
    fn test_available_models() {
        let models = available_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "qwen3-8b-q4km");
    }

    #[test]
    fn test_get_model() {
        let model = get_model("qwen3-4b-q4km");
        assert!(model.is_some());
        assert_eq!(model.unwrap().name, "Qwen3 4B (Q4_K_M)");

        let missing = get_model("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_default_model() {
        let model = default_model();
        assert_eq!(model.id, "qwen3-8b-q4km");
        assert!(!model.gguf_file.is_empty());
    }

    #[test]
    fn test_available_embedding_models() {
        let models = available_embedding_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "bge-base-en");
        assert_eq!(models[0].dimensions, 768);
    }

    #[test]
    fn test_get_embedding_model() {
        let model = get_embedding_model("bge-small-en");
        assert!(model.is_some());
        assert_eq!(model.unwrap().dimensions, 384);

        let missing = get_embedding_model("nonexistent");
        assert!(missing.is_none());
    }

    #[test]
    fn test_default_embedding_model() {
        let model = default_embedding_model();
        assert_eq!(model.id, "bge-base-en");
        assert_eq!(model.hf_repo_id, "BAAI/bge-base-en-v1.5");
    }
}
