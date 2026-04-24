use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::provider::ProviderConfig;

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

/// Per-role lifecycle settings.
///
/// Defaults are `false`: local models do **not** stay resident alongside
/// other local models. Remote providers ignore these flags (they're
/// hard-coded to `coexist = true`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LifecycleConfig {
    /// Keep the local chat model loaded while other local models load.
    #[serde(default)]
    pub chat_coexist: bool,
    /// Keep the local embedding model loaded while other local models load.
    #[serde(default)]
    pub embedding_coexist: bool,
}

/// User settings (persisted to disk)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Configured embedding model ID (None = disabled)
    #[serde(default)]
    pub embedding_model_id: Option<String>,
    /// Active chat provider configuration (local, OpenAI, or Anthropic)
    #[serde(default)]
    pub provider: Option<ProviderConfig>,
    /// Stored OpenAI API key (persisted separately from active provider)
    #[serde(default)]
    pub openai_api_key: Option<String>,
    /// Stored Anthropic API key (persisted separately from active provider)
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
    /// Per-role lifecycle controls (coexist flags, idle TTL, etc.).
    #[serde(default)]
    pub lifecycle: LifecycleConfig,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_config_defaults_when_missing() {
        // Existing installs whose settings.json pre-dates the `lifecycle`
        // field should deserialize cleanly with defaults.
        let json = r#"{"embedding_model_id":"qwen3-embedding"}"#;
        let parsed: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.lifecycle, LifecycleConfig::default());
        assert!(!parsed.lifecycle.chat_coexist);
        assert!(!parsed.lifecycle.embedding_coexist);
    }

    #[test]
    fn lifecycle_config_roundtrip() {
        let original = Settings {
            embedding_model_id: Some("m".into()),
            provider: None,
            openai_api_key: None,
            anthropic_api_key: None,
            lifecycle: LifecycleConfig {
                chat_coexist: true,
                embedding_coexist: false,
            },
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.lifecycle, original.lifecycle);
    }

    #[test]
    fn settings_all_defaults_when_empty_object() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.embedding_model_id.is_none());
        assert!(parsed.provider.is_none());
        assert_eq!(parsed.lifecycle, LifecycleConfig::default());
    }
}
