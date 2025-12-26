use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Application configuration (paths, computed at runtime)
#[derive(Debug, Clone)]
pub struct Config {
    /// Root data directory (~/.local/share/insight)
    pub data_dir: PathBuf,
    /// iroh data directory
    pub iroh_dir: PathBuf,
    /// Search index directory
    pub search_dir: PathBuf,
    /// Conversations storage directory
    pub conversations_dir: PathBuf,
    /// Settings file path
    pub settings_file: PathBuf,
}

impl Config {
    /// Load configuration or use defaults
    pub fn load_or_default() -> Self {
        let data_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("insight");

        Self {
            iroh_dir: data_dir.join("iroh"),
            search_dir: data_dir.join("search"),
            conversations_dir: data_dir.join("conversations"),
            settings_file: data_dir.join("settings.json"),
            data_dir,
        }
    }

    /// Ensure all required directories exist
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.data_dir)?;
        std::fs::create_dir_all(&self.iroh_dir)?;
        std::fs::create_dir_all(&self.search_dir)?;
        std::fs::create_dir_all(&self.conversations_dir)?;
        Ok(())
    }
}

/// User settings (persisted to disk)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Configured language model ID (None = not configured)
    #[serde(default)]
    pub language_model_id: Option<String>,
    /// Configured embedding model ID (None = disabled)
    #[serde(default)]
    pub embedding_model_id: Option<String>,
}

impl Settings {
    /// Load settings from file, or return defaults if not found
    pub fn load(path: &PathBuf) -> Self {
        match std::fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|e| {
                tracing::warn!("Failed to parse settings file, using defaults: {}", e);
                Self::default()
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(e) => {
                tracing::warn!("Failed to read settings file, using defaults: {}", e);
                Self::default()
            }
        }
    }

    /// Save settings to file
    pub fn save(&self, path: &PathBuf) -> std::io::Result<()> {
        let contents = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, contents)
    }
}
