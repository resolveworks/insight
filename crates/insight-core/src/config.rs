use std::path::PathBuf;

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
}

/// Errors from loading persisted settings.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    /// The settings file exists but could not be read.
    #[error("failed to read settings file {}: {source}", path.display())]
    Read {
        /// Settings file that could not be read.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The settings file content is not valid settings JSON.
    #[error("failed to parse settings file {}: {source}", path.display())]
    Parse {
        /// Settings file that could not be parsed.
        path: PathBuf,
        /// Underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
}

impl Settings {
    /// Load settings from file.
    ///
    /// A missing file means no settings have been persisted yet and yields
    /// defaults. Any other failure — an unreadable file or malformed JSON — is
    /// returned as a [`SettingsError`]; invalid persisted state never silently
    /// degrades to defaults.
    pub fn load(path: &PathBuf) -> Result<Self, SettingsError> {
        let contents = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => {
                return Err(SettingsError::Read {
                    path: path.clone(),
                    source: e,
                })
            }
        };

        serde_json::from_str(&contents).map_err(|e| SettingsError::Parse {
            path: path.clone(),
            source: e,
        })
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
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{}").unwrap();
        let settings = Settings::load(&path).unwrap();
        assert!(settings.provider.is_none());
    }

    #[test]
    fn load_missing_file_returns_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let settings = Settings::load(&path).expect("missing file is not an error");
        assert!(settings.provider.is_none());
    }

    #[test]
    fn load_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"provider":{"family":"anthropic/anthropic","model":"claude","api_key":"k","base_url":null}}"#,
        )
        .unwrap();
        let settings = Settings::load(&path).unwrap();
        let provider = settings.provider.expect("provider configured");
        assert_eq!(provider.family, "anthropic/anthropic");
        assert_eq!(provider.model, "claude");
    }

    #[test]
    fn load_malformed_json_is_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        match Settings::load(&path) {
            Err(SettingsError::Parse { path: err_path, .. }) => assert_eq!(err_path, path),
            other => panic!("expected parse error, got {other:?}"),
        }
    }

    #[test]
    fn load_read_failure_is_read_error() {
        let dir = tempfile::tempdir().unwrap();
        // A directory at the settings path exists but cannot be read as a file.
        let path = dir.path().join("settings.json");
        std::fs::create_dir(&path).unwrap();
        match Settings::load(&path) {
            Err(SettingsError::Read { path: err_path, .. }) => assert_eq!(err_path, path),
            other => panic!("expected read error, got {other:?}"),
        }
    }
}
