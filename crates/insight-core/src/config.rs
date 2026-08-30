use std::{collections::HashMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::providers::ProviderSettings;

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
    /// Active chat provider configuration, as consumed by
    /// [`crate::providers::build_model`].
    #[serde(default)]
    pub provider: Option<ProviderSettings>,
    /// Stored API keys per provider family (`family -> key`), persisted
    /// separately from the active provider so switching families keeps the
    /// previously entered key.
    #[serde(default)]
    pub api_keys: HashMap<String, String>,
}

impl Settings {
    /// Load settings from file, or return defaults if not found.
    pub fn load(path: &PathBuf) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("Failed to read settings file, using defaults: {}", e);
                return Self::default();
            }
        };

        match serde_json::from_str::<Self>(&contents) {
            Ok(settings) => settings,
            Err(e) => {
                tracing::warn!("Failed to parse settings file, using defaults: {}", e);
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
    fn settings_all_defaults_when_empty_object() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.provider.is_none());
        assert!(parsed.api_keys.is_empty());
    }

    #[test]
    fn settings_defaults_when_fields_missing() {
        let parsed: Settings = serde_json::from_str(r#"{"api_keys":{"openai":"sk-x"}}"#).unwrap();
        assert_eq!(
            parsed.api_keys.get("openai").map(String::as_str),
            Some("sk-x")
        );
        assert!(parsed.provider.is_none());
    }

    #[test]
    fn settings_roundtrip() {
        let original = Settings {
            provider: Some(ProviderSettings {
                family: "anthropic".into(),
                model: "claude".into(),
                api_key: Some("k".into()),
                base_url: None,
            }),
            api_keys: [("anthropic".to_string(), "k".to_string())]
                .into_iter()
                .collect(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.provider, original.provider);
        assert_eq!(parsed.api_keys, original.api_keys);
    }
}
