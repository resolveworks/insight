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

/// User settings (persisted to disk)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Active chat provider configuration (OpenAI or Anthropic)
    #[serde(default)]
    pub provider: Option<ProviderConfig>,
    /// Stored OpenAI API key (persisted separately from active provider)
    #[serde(default)]
    pub openai_api_key: Option<String>,
    /// Stored Anthropic API key (persisted separately from active provider)
    #[serde(default)]
    pub anthropic_api_key: Option<String>,
}

impl Settings {
    /// Load settings from file, or return defaults if not found.
    ///
    /// Legacy settings files may contain a local-inference provider
    /// (`"type": "local"`) that no longer parses; rather than wiping ALL
    /// settings (including stored API keys), the stale provider entry is
    /// dropped and the remaining fields are kept.
    pub fn load(path: &PathBuf) -> Self {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Self::default(),
            Err(e) => {
                tracing::warn!("Failed to read settings file, using defaults: {}", e);
                return Self::default();
            }
        };

        match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(mut value) => {
                let filtered = filter_legacy_provider(value.take());
                serde_json::from_value(filtered).unwrap_or_else(|e| {
                    tracing::warn!("Failed to parse settings file, using defaults: {}", e);
                    Self::default()
                })
            }
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

/// Remove a provider entry that doesn't describe a supported (remote)
/// provider, e.g. a legacy `{"type": "local", ...}` object. Leaves
/// everything else untouched.
fn filter_legacy_provider(mut value: serde_json::Value) -> serde_json::Value {
    if let Some(provider) = value.get("provider").cloned() {
        let supported = provider
            .get("type")
            .and_then(|t| t.as_str())
            .is_some_and(|t| t == "openai" || t == "anthropic");
        if !supported {
            if let Some(map) = value.as_object_mut() {
                map.remove("provider");
            }
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_all_defaults_when_empty_object() {
        let parsed: Settings = serde_json::from_str("{}").unwrap();
        assert!(parsed.provider.is_none());
        assert!(parsed.openai_api_key.is_none());
        assert!(parsed.anthropic_api_key.is_none());
    }

    #[test]
    fn settings_defaults_when_fields_missing() {
        let parsed: Settings = serde_json::from_str(r#"{"openai_api_key":"sk-x"}"#).unwrap();
        assert_eq!(parsed.openai_api_key.as_deref(), Some("sk-x"));
        assert!(parsed.provider.is_none());
        assert!(parsed.anthropic_api_key.is_none());
    }

    #[test]
    fn legacy_local_provider_is_dropped_but_siblings_survive() {
        let json = r#"{
            "embedding_model_id": "qwen3-embedding",
            "ocr_model_id": "nanonets",
            "provider": {"type": "local", "model_id": "qwen3-8b"},
            "openai_api_key": "sk-keep",
            "anthropic_api_key": "ant-keep",
            "lifecycle": {"chat_coexist": true}
        }"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let filtered = filter_legacy_provider(value);
        let parsed: Settings = serde_json::from_value(filtered).unwrap();
        assert!(parsed.provider.is_none());
        assert_eq!(parsed.openai_api_key.as_deref(), Some("sk-keep"));
        assert_eq!(parsed.anthropic_api_key.as_deref(), Some("ant-keep"));
    }

    #[test]
    fn remote_providers_survive_migration() {
        let json = r#"{
            "provider": {"type": "openai", "api_key": "sk-1", "model": "gpt-4o"},
            "openai_api_key": "sk-2"
        }"#;
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let filtered = filter_legacy_provider(value);
        let parsed: Settings = serde_json::from_value(filtered).unwrap();
        assert!(matches!(
            parsed.provider,
            Some(ProviderConfig::OpenAI { ref model, .. }) if model == "gpt-4o"
        ));
    }

    #[test]
    fn settings_roundtrip() {
        let original = Settings {
            provider: Some(ProviderConfig::Anthropic {
                api_key: "k".into(),
                model: "claude".into(),
            }),
            openai_api_key: Some("sk".into()),
            anthropic_api_key: Some("ant".into()),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.provider, original.provider);
        assert_eq!(parsed.openai_api_key, original.openai_api_key);
        assert_eq!(parsed.anthropic_api_key, original.anthropic_api_key);
    }
}
