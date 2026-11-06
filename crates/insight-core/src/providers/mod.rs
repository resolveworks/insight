//! Data-driven registry over the chat-completion providers offered by [`rig`].
//!
//! Insight's chat inference runs on remote LLM providers through the rig
//! framework. This module turns rig's provider registry into a uniform
//! application surface:
//!
//! - [`list_families`] describes the provider selections Insight offers for
//!   the UI (whether an API key is required, whether models can be listed
//!   remotely).
//! - [`build_model`] constructs a [`ModelHandle`] for a persisted
//!   [`ProviderSettings`], validating required key/URL inputs first.
//! - [`list_models`] queries a listing-capable selection's client.
//!
//! A family id is rig's canonical [`ProviderId`] selection string, such as
//! `"openai/openai"`, `"gcp.gemini/gemini"` or `"ollama/openai"`. The table
//! exposes one selection per registered vendor, preserving the wire each
//! vendor used before the registry migration.
//!
//! Credential requirements and the required-endpoint case come from rig
//! itself. Every rig provider configuration accepts a custom base URL, so
//! [`ProviderFamily::supports_base_url`] is true for every family. Local-server
//! selections (`llama.cpp`, Ollama) need no API key.

use rig_core::catalog::Catalog;
use rig_core::providers::registry::{ProviderConfig, ProviderId};
use rig_core::{operation::Completion, DynModel};
use serde::{Deserialize, Serialize};

type ModelHandle = DynModel<Completion>;

/// A provider configuration as persisted to `settings.json`.
///
/// `family` selects an entry from the [`list_families`] table by its
/// canonical rig selection string (e.g. `"openai/openai"`,
/// `"anthropic/anthropic"`, `"ollama/openai"`); `model` is the provider's
/// model identifier (e.g. `"gpt-4o-mini"`). `api_key` and `base_url` are
/// optional per family — see [`ProviderFamily::requires_api_key`] and
/// [`ProviderFamily::supports_base_url`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Canonical rig [`ProviderId`] selection string, one of
    /// [`ProviderFamily::id`].
    pub family: String,
    /// Provider-side model identifier.
    pub model: String,
    /// API key (or access token) for the family, if it requires one.
    pub api_key: Option<String>,
    /// Custom base URL / endpoint, overriding the provider default.
    pub base_url: Option<String>,
}

/// UI-facing descriptor for one provider family in the registry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderFamily {
    /// Canonical rig [`ProviderId`] selection string used in
    /// [`ProviderSettings::family`].
    pub id: String,
    /// Human-readable name for display.
    pub name: String,
    /// Whether the family requires an API key, from rig's credential
    /// requirements.
    pub requires_api_key: bool,
    /// Whether the family accepts a custom base URL. Every rig provider
    /// configuration does.
    pub supports_base_url: bool,
    /// Whether the family's models can be listed remotely via [`list_models`].
    pub supports_model_listing: bool,
}

/// A remotely listed model entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModel {
    /// Provider-side model identifier (usable as [`ProviderSettings::model`]).
    pub id: String,
    /// Human-readable display name, falling back to the id.
    pub display_name: String,
}

/// Errors from the provider registry.
#[derive(Debug, thiserror::Error)]
pub enum ProviderRegistryError {
    /// The settings reference a family id that is not in the registry.
    #[error("unknown provider family: {0}")]
    UnknownFamily(String),

    /// The family requires an API key but none was configured.
    #[error("provider family '{0}' requires an API key")]
    MissingApiKey(String),

    /// The family requires a base URL / endpoint but none was configured.
    #[error("provider family '{0}' requires a base URL")]
    MissingBaseUrl(String),

    /// Remote model listing failed.
    #[error("failed to list models for '{family}': {source}")]
    ListModels {
        /// Family whose listing endpoint failed.
        family: String,
        /// Underlying rig provider error.
        #[source]
        source: Box<rig::ProviderError>,
    },

    /// The family does not implement rig's model-listing capability.
    #[error("provider family '{0}' does not support model listing")]
    ListingNotSupported(String),
}

/// The registry's family table.
///
/// One canonical [`ProviderId`] selection per vendor registered by rig's
/// completion-capable providers. The selection keeps the wire each vendor
/// used before the registry migration (OpenAI-format unless Gemini or
/// Anthropic is named). Ordering is alphabetical by selection id.
struct FamilySpec {
    /// Canonical rig selection string used in [`ProviderSettings::family`].
    id: &'static str,
    /// Human-readable name for display.
    name: &'static str,
    /// Whether the family's models can be listed remotely via [`list_models`].
    supports_model_listing: bool,
}

const FAMILIES: &[FamilySpec] = &[
    spec("anthropic/anthropic", "Anthropic", true),
    spec("azure.openai/openai", "Azure OpenAI", false),
    spec("chatgpt/openai", "ChatGPT", false),
    spec("cohere/openai", "Cohere", false),
    spec("copilot/openai", "GitHub Copilot", true),
    spec("deepseek/openai", "DeepSeek", true),
    spec("doubleword/openai", "Doubleword", false),
    spec("gcp.gemini/gemini", "Google Gemini", true),
    spec("groq/openai", "Groq", true),
    spec("huggingface/openai", "Hugging Face", false),
    spec("hyperbolic/openai", "Hyperbolic", false),
    spec("llamacpp/openai", "llama.cpp", false),
    spec("minimax/openai", "MiniMax", true),
    spec("mira/openai", "Mira", true),
    spec("mistral/openai", "Mistral", true),
    spec("moonshot/openai", "Moonshot", true),
    spec("ollama/openai", "Ollama", true),
    spec("openai/openai", "OpenAI", true),
    spec("opencode/openai", "OpenCode Zen", true),
    spec("opencode-go/openai", "OpenCode Go", true),
    spec("openrouter/openai", "OpenRouter", true),
    spec("perplexity/openai", "Perplexity", false),
    spec("together/openai", "Together AI", false),
    spec("venice/openai", "Venice", true),
    spec("xai/openai", "xAI", false),
    spec("xiaomimimo/openai", "Xiaomi MiMo", true),
    spec("zai/openai", "Z.AI", false),
];

/// Small helper for building [`FAMILIES`] entries.
const fn spec(id: &'static str, name: &'static str, supports_model_listing: bool) -> FamilySpec {
    FamilySpec {
        id,
        name,
        supports_model_listing,
    }
}

/// The [`ProviderId`] a table entry names.
///
/// The table is static and every id is canonical, so a failure here is a
/// programming error rather than user input.
fn provider_id(spec: &FamilySpec) -> ProviderId {
    ProviderId::resolve(spec.id).expect("family table ids are canonical rig provider selections")
}

impl From<&FamilySpec> for ProviderFamily {
    fn from(spec: &FamilySpec) -> Self {
        let id = provider_id(spec);
        Self {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            requires_api_key: id.requires_credential(),
            // Every rig provider configuration accepts a custom base URL.
            supports_base_url: true,
            supports_model_listing: spec.supports_model_listing,
        }
    }
}

/// All provider families in the registry, ordered by id.
pub fn list_families() -> Vec<ProviderFamily> {
    FAMILIES.iter().map(ProviderFamily::from).collect()
}

/// Look up a family descriptor by canonical selection id.
fn lookup_family(id: &str) -> Result<&'static FamilySpec, ProviderRegistryError> {
    FAMILIES
        .iter()
        .find(|f| f.id == id)
        .ok_or_else(|| ProviderRegistryError::UnknownFamily(id.to_string()))
}

/// Resolve `family` and build its provider configuration.
///
/// Credential requirements come from rig's selection, as does the
/// required-endpoint case: a configuration whose default base URL is empty
/// (Azure) rejects an absent base URL. A non-empty custom base URL overrides
/// the provider default; an empty one is treated as absent.
fn build_config(
    family: &'static FamilySpec,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<(ProviderId, ProviderConfig), ProviderRegistryError> {
    let id = provider_id(family);
    let key = api_key.map(str::trim).unwrap_or_default();
    if id.requires_credential() && key.is_empty() {
        return Err(ProviderRegistryError::MissingApiKey(family.id.to_string()));
    }
    let config = id
        .config(key)
        .expect("registered provider selections have a configuration");
    let config = match base_url.map(str::trim).filter(|url| !url.is_empty()) {
        Some(url) => config.with_base_url(url),
        None => config,
    };
    if config.base_url().is_empty() {
        return Err(ProviderRegistryError::MissingBaseUrl(family.id.to_string()));
    }
    Ok((id, config))
}

/// Build a rig [`ModelHandle`] for the given settings.
///
/// Validates the key/base URL requirements of the family first, then
/// constructs the provider's completion model with rig's catalog facts so
/// model-dependent routing and catalog pricing apply. Construction performs
/// no network I/O; the first completion request pays any
/// authentication/verification cost.
pub fn build_model(settings: &ProviderSettings) -> Result<ModelHandle, ProviderRegistryError> {
    let family = lookup_family(&settings.family)?;
    let (id, config) = build_config(
        family,
        settings.api_key.as_deref(),
        settings.base_url.as_deref(),
    )?;
    let facts = Catalog::builtin().facts(id, &settings.model);
    Ok(config.completion_model(&settings.model, rig::rig_reqwest::shared(), facts))
}

/// List the models a family offers, via rig's model-listing capability.
///
/// `api_key`/`base_url` follow the same rules as in [`build_model`]. Returns
/// [`ProviderRegistryError::ListingNotSupported`] for families whose listing
/// is not offered. OpenAI-, Anthropic- and Gemini-format selections use their
/// wire's client; Ollama keeps its native `/api/tags` listing.
pub async fn list_models(
    family: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<ProviderModel>, ProviderRegistryError> {
    let family = lookup_family(family)?;
    if !family.supports_model_listing {
        return Err(ProviderRegistryError::ListingNotSupported(
            family.id.to_string(),
        ));
    }
    let (id, config) = build_config(family, api_key, base_url)?;
    let listed = if id.vendor() == "ollama" {
        let mut config = rig::providers::ollama::OllamaConfig::new()
            .with_api_key(api_key.map(str::trim).unwrap_or_default());
        if let Some(url) = base_url.map(str::trim).filter(|url| !url.is_empty()) {
            config = config.with_base_url(url);
        }
        config.client().list_models().await
    } else {
        match config {
            ProviderConfig::OpenAi(config) => config.client().list_models().await,
            ProviderConfig::Anthropic(config) => config.client().list_models().await,
            ProviderConfig::Gemini(config) => config.client().list_models().await,
        }
    };
    let models = listed.map_err(|source| ProviderRegistryError::ListModels {
        family: family.id.to_string(),
        source: Box::new(source),
    })?;
    Ok(models
        .iter()
        .map(|m| ProviderModel {
            id: m.id.clone(),
            display_name: m.display_name().to_string(),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn family_table_is_complete_and_unique() {
        let families = list_families();
        assert_eq!(families.len(), 27, "one selection per registered vendor");

        let mut ids: Vec<&str> = families.iter().map(|f| f.id.as_str()).collect();
        ids.sort_unstable();
        let unique: Vec<&str> = ids
            .iter()
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(ids, unique, "family ids must be unique");

        for family in &families {
            let id = ProviderId::resolve(&family.id).expect("family id resolves");
            assert_eq!(id.to_string(), family.id, "family id is canonical");
            assert_eq!(
                family.requires_api_key,
                id.requires_credential(),
                "credential requirement comes from rig for {}",
                family.id
            );
            assert!(family.supports_base_url);
            assert!(!family.name.is_empty(), "name must not be empty");
        }

        let registered_vendors: BTreeSet<&str> = ProviderId::all().map(|id| id.vendor()).collect();
        let table_vendors: BTreeSet<&str> = FAMILIES
            .iter()
            .map(|family| provider_id(family).vendor())
            .collect();
        assert_eq!(
            table_vendors, registered_vendors,
            "every registered vendor is exposed exactly once"
        );

        // Spot-check representatives of each shape.
        let openai = families.iter().find(|f| f.id == "openai/openai").unwrap();
        assert!(
            openai.requires_api_key && openai.supports_base_url && openai.supports_model_listing
        );
        let ollama = families.iter().find(|f| f.id == "ollama/openai").unwrap();
        assert!(
            !ollama.requires_api_key && ollama.supports_base_url && ollama.supports_model_listing
        );
        assert!(
            families.iter().any(|f| f.id == "opencode/openai"),
            "OpenCode Zen is exposed"
        );
        assert!(
            families.iter().any(|f| f.id == "opencode-go/openai"),
            "OpenCode Go is exposed"
        );
        assert!(
            families.iter().all(|f| f.id != "voyageai"),
            "embeddings-only families are excluded"
        );
    }

    #[test]
    fn settings_serde_round_trip() {
        let settings = ProviderSettings {
            family: "anthropic/anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            api_key: Some("sk-test".into()),
            base_url: None,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let back: ProviderSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(settings, back);

        // Tolerates absent optional fields.
        let minimal: ProviderSettings =
            serde_json::from_str(r#"{"family":"ollama/openai","model":"llama3"}"#)
                .expect("minimal");
        assert_eq!(minimal.api_key, None);
        assert_eq!(minimal.base_url, None);
    }

    #[test]
    fn build_model_rejects_missing_api_key() {
        let settings = ProviderSettings {
            family: "openai/openai".into(),
            model: "gpt-4o-mini".into(),
            api_key: None,
            base_url: None,
        };
        match build_model(&settings) {
            Err(ProviderRegistryError::MissingApiKey(family)) => {
                assert_eq!(family, "openai/openai")
            }
            other => panic!("expected MissingApiKey, got {:?}", other.map(|_| ())),
        }

        let settings = ProviderSettings {
            family: "anthropic/anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            api_key: Some("  ".into()), // whitespace-only is still missing
            base_url: None,
        };
        assert!(matches!(
            build_model(&settings),
            Err(ProviderRegistryError::MissingApiKey(_))
        ));
    }

    #[test]
    fn build_model_rejects_missing_base_url_for_azure() {
        let settings = ProviderSettings {
            family: "azure.openai/openai".into(),
            model: "gpt-4o".into(),
            api_key: Some("key".into()),
            base_url: None,
        };
        assert!(matches!(
            build_model(&settings),
            Err(ProviderRegistryError::MissingBaseUrl(_))
        ));
    }

    #[test]
    fn build_model_rejects_unknown_family() {
        for family in ["not-a-provider", "openai", "gemini", "llamafile"] {
            let settings = ProviderSettings {
                family: family.into(),
                model: "m".into(),
                api_key: Some("k".into()),
                base_url: None,
            };
            assert!(
                matches!(
                    build_model(&settings),
                    Err(ProviderRegistryError::UnknownFamily(_))
                ),
                "{family} is not a canonical table selection"
            );
        }
    }

    #[test]
    fn local_families_build_without_api_key() {
        // ollama accepts an empty key (rig maps it to "no auth") and its
        // client defaults to localhost — construction must succeed offline.
        let settings = ProviderSettings {
            family: "ollama/openai".into(),
            model: "llama3".into(),
            api_key: None,
            base_url: None,
        };
        let handle = build_model(&settings).expect("ollama client builds without key");
        let _ = handle; // opaque ModelHandle

        let settings = ProviderSettings {
            family: "llamacpp/openai".into(),
            model: "gemma-2".into(),
            api_key: None,
            base_url: Some("http://127.0.0.1:8080".into()),
        };
        assert!(build_model(&settings).is_ok());
    }

    #[test]
    fn remote_families_build_offline_with_dummy_key() {
        // Client construction is pure configuration; no network happens here.
        for family in [
            "openai/openai",
            "anthropic/anthropic",
            "groq/openai",
            "gcp.gemini/gemini",
            "azure.openai/openai",
        ] {
            let settings = ProviderSettings {
                family: family.into(),
                model: "test-model".into(),
                api_key: Some("dummy-key".into()),
                base_url: Some("https://example.invalid".into()),
            };
            assert!(
                build_model(&settings).is_ok(),
                "{family} client should construct offline"
            );
        }
    }

    #[tokio::test]
    async fn list_models_rejects_unsupported_families() {
        let err = list_models("cohere/openai", Some("k"), None)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ProviderRegistryError::ListingNotSupported(f) if f == "cohere/openai")
        );
        let err = list_models("voyageai", Some("k"), None).await.unwrap_err();
        assert!(matches!(err, ProviderRegistryError::UnknownFamily(_)));
    }
}
