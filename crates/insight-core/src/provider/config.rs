//! Provider configuration types persisted to settings.

use serde::{Deserialize, Serialize};

/// Provider configuration stored in settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    /// Local model via mistralrs
    Local { model_id: String },
    /// OpenAI API
    #[serde(rename = "openai")]
    OpenAI { api_key: String, model: String },
    /// Anthropic API
    Anthropic { api_key: String, model: String },
}

impl ProviderConfig {
    /// Short provider family name.
    pub fn provider_type(&self) -> &'static str {
        match self {
            ProviderConfig::Local { .. } => "local",
            ProviderConfig::OpenAI { .. } => "openai",
            ProviderConfig::Anthropic { .. } => "anthropic",
        }
    }

    /// Model identifier within the provider family.
    pub fn model_id(&self) -> &str {
        match self {
            ProviderConfig::Local { model_id } => model_id,
            ProviderConfig::OpenAI { model, .. } => model,
            ProviderConfig::Anthropic { model, .. } => model,
        }
    }
}

/// Information about a remote model returned from an API listing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteModelInfo {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
}

/// Provider family descriptor for the UI picker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFamily {
    pub id: String,
    pub name: String,
    pub description: String,
    pub requires_api_key: bool,
}

pub fn get_provider_families() -> Vec<ProviderFamily> {
    vec![
        ProviderFamily {
            id: "local".to_string(),
            name: "Local".to_string(),
            description: "Run models locally on your machine".to_string(),
            requires_api_key: false,
        },
        ProviderFamily {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            description: "GPT-4o, GPT-4, and other OpenAI models".to_string(),
            requires_api_key: true,
        },
        ProviderFamily {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            description: "Claude 3.5 Sonnet, Claude 3 Opus, and more".to_string(),
            requires_api_key: true,
        },
    ]
}
