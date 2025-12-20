use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hf_hub::api::tokio::{Api, ApiBuilder};
use hf_hub::Cache;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Information about a model available for use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Unique identifier for this model
    pub id: String,
    /// Display name
    pub name: String,
    /// HuggingFace repo ID (e.g., "microsoft/Phi-3.5-mini-instruct")
    pub repo_id: String,
    /// Description for the user
    pub description: String,
    /// Approximate size in GB (for display)
    pub size_gb: f32,
}

/// Files required for a text model
const REQUIRED_FILES: &[&str] = &["config.json", "tokenizer.json", "tokenizer_config.json"];

/// Optional files to download if present
const OPTIONAL_FILES: &[&str] = &["generation_config.json"];

/// Get the default model
pub fn default_model() -> ModelInfo {
    ModelInfo {
        id: "phi-4-mini".to_string(),
        name: "Phi 4 Mini".to_string(),
        repo_id: "microsoft/Phi-4-mini-instruct".to_string(),
        description: "Fast, capable 3.8B parameter model with 128K context".to_string(),
        size_gb: 7.6,
    }
}

/// Available models registry
pub fn available_models() -> Vec<ModelInfo> {
    vec![default_model()]
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
        let cache_repo = self.cache.model(model.repo_id.clone());

        // Check required files
        for file in REQUIRED_FILES {
            if cache_repo.get(file).is_none() {
                return false;
            }
        }

        // Check for at least one safetensors file
        // We can't enumerate cache, so we check common patterns
        cache_repo.get("model.safetensors").is_some()
            || cache_repo.get("model-00001-of-00002.safetensors").is_some()
            || cache_repo.get("pytorch_model.bin").is_some()
    }

    /// Get the cache path for a model (if downloaded)
    pub fn get_model_path(&self, model: &ModelInfo) -> Option<PathBuf> {
        if self.is_downloaded(model) {
            // Return the directory containing the model files
            self.cache
                .model(model.repo_id.clone())
                .get("config.json")
                .map(|p| p.parent().unwrap().to_path_buf())
        } else {
            None
        }
    }

    /// Get paths to all model files (for LocalModelPaths)
    pub async fn get_model_files(&self, model: &ModelInfo) -> Result<ModelFiles> {
        let repo = self.api.model(model.repo_id.clone());

        // Get repo info to find all files
        let info = repo.info().await.context("Failed to get repo info")?;

        let mut weight_files = Vec::new();
        for sibling in &info.siblings {
            if sibling.rfilename.ends_with(".safetensors") {
                weight_files.push(sibling.rfilename.clone());
            }
        }

        // Fall back to pytorch if no safetensors
        if weight_files.is_empty() {
            for sibling in &info.siblings {
                if sibling.rfilename.ends_with(".bin")
                    && sibling.rfilename.contains("pytorch_model")
                {
                    weight_files.push(sibling.rfilename.clone());
                }
            }
        }

        Ok(ModelFiles {
            config: "config.json".to_string(),
            tokenizer: "tokenizer.json".to_string(),
            tokenizer_config: "tokenizer_config.json".to_string(),
            generation_config: Some("generation_config.json".to_string()),
            weights: weight_files,
        })
    }

    /// Download a model with progress tracking
    pub async fn download_model(
        &self,
        model: &ModelInfo,
        progress_tx: mpsc::Sender<DownloadProgress>,
    ) -> Result<PathBuf> {
        let repo = self.api.model(model.repo_id.clone());

        // Get list of files to download
        let files = self.get_model_files(model).await?;
        let mut all_files: Vec<String> = REQUIRED_FILES.iter().map(|s| s.to_string()).collect();
        all_files.extend(files.weights.clone());

        // Add optional files if they exist in the repo
        let info = repo.info().await?;
        for optional in OPTIONAL_FILES {
            if info.siblings.iter().any(|s| s.rfilename == *optional) {
                all_files.push(optional.to_string());
            }
        }

        let total_files = all_files.len();
        let mut downloaded_paths = Vec::new();

        for (idx, filename) in all_files.iter().enumerate() {
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

            downloaded_paths.push(path);
        }

        // Return the directory containing the files
        let model_dir = downloaded_paths
            .first()
            .context("No files downloaded")?
            .parent()
            .context("Invalid path")?
            .to_path_buf();

        Ok(model_dir)
    }

    /// Download model without progress (uses hf-hub's built-in caching)
    pub async fn ensure_downloaded(&self, model: &ModelInfo) -> Result<PathBuf> {
        let repo = self.api.model(model.repo_id.clone());

        // Get list of files
        let files = self.get_model_files(model).await?;

        // Download required files (hf-hub caches automatically)
        for file in REQUIRED_FILES {
            repo.get(file)
                .await
                .with_context(|| format!("Failed to download {}", file))?;
        }

        // Download weight files
        for weight_file in &files.weights {
            repo.get(weight_file)
                .await
                .with_context(|| format!("Failed to download {}", weight_file))?;
        }

        // Download optional files if they exist
        let info = repo.info().await?;
        for optional in OPTIONAL_FILES {
            if info.siblings.iter().any(|s| s.rfilename == *optional) {
                let _ = repo.get(optional).await; // Ignore errors for optional files
            }
        }

        // Return path to config.json's directory
        let config_path = repo.get("config.json").await?;
        Ok(config_path.parent().unwrap().to_path_buf())
    }
}

/// Collection of paths to model files
#[derive(Debug, Clone)]
pub struct ModelFiles {
    pub config: String,
    pub tokenizer: String,
    pub tokenizer_config: String,
    pub generation_config: Option<String>,
    pub weights: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_available_models() {
        let models = available_models();
        assert!(!models.is_empty());
        assert_eq!(models[0].id, "phi-4-mini");
    }
}
