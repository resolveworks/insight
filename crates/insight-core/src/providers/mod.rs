//! Data-driven registry over the chat-completion providers offered by [`rig`].
//!
//! Insight's chat inference runs on remote LLM providers through the rig
//! framework. This module turns rig's per-provider client types into a
//! uniform, table-driven surface:
//!
//! - [`list_families`] describes every supported provider family for the UI
//!   (whether an API key or base URL is required, whether models can be
//!   listed remotely).
//! - [`build_model`] constructs a [`ModelHandle`] for a persisted
//!   [`ProviderSettings`], validating required key/URL inputs first.
//! - [`list_models`] queries a listing-capable family via rig's
//!   [`ModelListingClient`](rig::client::ModelListingClient).
//!
//! The family table mirrors the provider modules in rig-core that declare a
//! completion capability. Providers that are embeddings-only (e.g. voyageai)
//! are intentionally absent. Construction is uniform: every family uses
//! rig's builder pattern (`Client::builder().api_key(..).base_url(..)`)
//! except Azure, whose OpenAI resource endpoint is set through
//! [`azure_endpoint`](rig::providers::azure::ClientBuilder::azure_endpoint).
//!
//! Local-server families (`ollama`, `llamacpp`) need no API key — rig treats
//! an empty key as "no auth" for them, and their clients default to the
//! usual localhost endpoints.

use rig::client::{CompletionClient, DefaultTransportBuilder, ModelListingClient};
use rig::completion::ModelHandle;
use serde::{Deserialize, Serialize};

/// A provider configuration as persisted to `settings.json`.
///
/// `family` selects an entry from the [`list_families`] table (e.g. `"openai"`,
/// `"anthropic"`); `model` is the provider's model identifier (e.g.
/// `"gpt-4o-mini"`). `api_key` and `base_url` are optional per family — see
/// [`ProviderFamily::requires_api_key`] and [`ProviderFamily::supports_base_url`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// Registry family id, one of [`ProviderFamily::id`].
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
    /// Stable family identifier used in [`ProviderSettings::family`].
    pub id: String,
    /// Human-readable name for display.
    pub name: String,
    /// Whether the family requires an API key.
    pub requires_api_key: bool,
    /// Whether the family accepts a custom base URL.
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

    /// The provider client could not be constructed.
    #[error("failed to construct '{family}' client: {source}")]
    ClientConstruction {
        /// Family that failed to build.
        family: String,
        /// Underlying rig transport error.
        #[source]
        source: rig::http_client::Error,
    },

    /// Remote model listing failed.
    #[error("failed to list models for '{family}': {source}")]
    ListModels {
        /// Family whose listing endpoint failed.
        family: String,
        /// Underlying rig listing error.
        #[source]
        source: rig::model::ModelListingError,
    },

    /// The family does not implement rig's model-listing capability.
    #[error("provider family '{0}' does not support model listing")]
    ListingNotSupported(String),
}

/// The registry's family table.
///
/// Sourced from the completion-capable provider modules of rig-core
/// (`crates/rig-core/src/providers/`). Ordering is alphabetical by id.
/// Excluded: `voyageai` (embeddings/rerank only, no completion client).
struct FamilySpec {
    /// Stable family identifier used in [`ProviderSettings::family`].
    id: &'static str,
    /// Human-readable name for display.
    name: &'static str,
    /// Whether the family requires an API key.
    requires_api_key: bool,
    /// Whether the family accepts a custom base URL.
    supports_base_url: bool,
    /// Whether the family's models can be listed remotely via [`list_models`].
    supports_model_listing: bool,
}

const FAMILIES: &[FamilySpec] = &[
    spec("anthropic", "Anthropic", true, true, true),
    spec("azure", "Azure OpenAI", true, true, false),
    spec("chatgpt", "ChatGPT", true, false, false),
    spec("cohere", "Cohere", true, false, false),
    spec("copilot", "GitHub Copilot", true, false, true),
    spec("deepseek", "DeepSeek", true, false, true),
    spec("doubleword", "Doubleword", true, true, false),
    spec("gemini", "Google Gemini", true, false, true),
    spec("groq", "Groq", true, false, true),
    spec("huggingface", "Hugging Face", true, false, false),
    spec("hyperbolic", "Hyperbolic", true, false, false),
    spec("llamacpp", "llama.cpp", false, true, true),
    spec("minimax", "MiniMax", true, false, true),
    spec("mira", "Mira", true, false, true),
    spec("mistral", "Mistral", true, false, true),
    spec("moonshot", "Moonshot", true, false, true),
    spec("ollama", "Ollama", false, true, true),
    spec("openai", "OpenAI", true, true, true),
    spec("openrouter", "OpenRouter", true, false, true),
    spec("perplexity", "Perplexity", true, false, false),
    spec("together", "Together AI", true, false, false),
    spec("venice", "Venice", true, false, true),
    spec("xai", "xAI", true, false, false),
    spec("xiaomimimo", "Xiaomi MiMo", true, false, true),
    spec("zai", "Z.ai", true, false, false),
];

/// Small helper for building [`FAMILIES`] entries.
const fn spec(
    id: &'static str,
    name: &'static str,
    requires_api_key: bool,
    supports_base_url: bool,
    supports_model_listing: bool,
) -> FamilySpec {
    FamilySpec {
        id,
        name,
        requires_api_key,
        supports_base_url,
        supports_model_listing,
    }
}

impl From<&FamilySpec> for ProviderFamily {
    fn from(spec: &FamilySpec) -> Self {
        Self {
            id: spec.id.to_string(),
            name: spec.name.to_string(),
            requires_api_key: spec.requires_api_key,
            supports_base_url: spec.supports_base_url,
            supports_model_listing: spec.supports_model_listing,
        }
    }
}

/// All provider families in the registry, ordered by id.
pub fn list_families() -> Vec<ProviderFamily> {
    FAMILIES.iter().map(ProviderFamily::from).collect()
}

/// Look up a family descriptor by id.
fn lookup_family(id: &str) -> Result<&'static FamilySpec, ProviderRegistryError> {
    FAMILIES
        .iter()
        .find(|f| f.id == id)
        .ok_or_else(|| ProviderRegistryError::UnknownFamily(id.to_string()))
}

/// Construct a rig client for `family` and run `body` with it.
///
/// Every family builds through rig's uniform builder
/// (`Client::builder().api_key(..)[.base_url(..)].build()`); Azure routes its
/// endpoint through the provider-specific `azure_endpoint` builder method.
/// The macro form keeps each arm's concrete client type local to that arm,
/// so one expansion serves both model construction and model listing.
macro_rules! with_client {
    ($family:expr, $key:expr, $url:expr, |$client:ident| $body:expr) => {
        match $family {
            "anthropic" => with_client!(@arm anthropic, $key, $url, $family, $client, $body),
            "azure" => {{
                let $client = rig::providers::azure::Client::builder()
                    .api_key(rig::providers::azure::AzureOpenAIAuth::ApiKey(
                        $key.to_string(),
                    ))
                    .azure_endpoint($url.unwrap_or_default().to_string())
                    .build()
                    .map_err(|source| ProviderRegistryError::ClientConstruction {
                        family: $family.to_string(),
                        source,
                    })?;
                $body
            }},
            "chatgpt" => with_client!(@arm chatgpt, $key, $url, $family, $client, $body),
            "cohere" => with_client!(@arm cohere, $key, $url, $family, $client, $body),
            "copilot" => with_client!(@arm copilot, $key, $url, $family, $client, $body),
            "deepseek" => with_client!(@arm deepseek, $key, $url, $family, $client, $body),
            "doubleword" => with_client!(@arm doubleword, $key, $url, $family, $client, $body),
            "gemini" => with_client!(@arm gemini, $key, $url, $family, $client, $body),
            "groq" => with_client!(@arm groq, $key, $url, $family, $client, $body),
            "huggingface" => with_client!(@arm huggingface, $key, $url, $family, $client, $body),
            "hyperbolic" => with_client!(@arm hyperbolic, $key, $url, $family, $client, $body),
            "llamacpp" => with_client!(@arm llamacpp, $key, $url, $family, $client, $body),
            "minimax" => with_client!(@arm minimax, $key, $url, $family, $client, $body),
            "mira" => with_client!(@arm mira, $key, $url, $family, $client, $body),
            "mistral" => with_client!(@arm mistral, $key, $url, $family, $client, $body),
            "moonshot" => with_client!(@arm moonshot, $key, $url, $family, $client, $body),
            "ollama" => with_client!(@arm ollama, $key, $url, $family, $client, $body),
            "openai" => with_client!(@arm openai, $key, $url, $family, $client, $body),
            "openrouter" => with_client!(@arm openrouter, $key, $url, $family, $client, $body),
            "perplexity" => with_client!(@arm perplexity, $key, $url, $family, $client, $body),
            "together" => with_client!(@arm together, $key, $url, $family, $client, $body),
            "venice" => with_client!(@arm venice, $key, $url, $family, $client, $body),
            "xai" => with_client!(@arm xai, $key, $url, $family, $client, $body),
            "xiaomimimo" => with_client!(@arm xiaomimimo, $key, $url, $family, $client, $body),
            "zai" => with_client!(@arm zai, $key, $url, $family, $client, $body),
            other => return Err(ProviderRegistryError::UnknownFamily(other.to_string())),
        }
    };
    (@arm $module:ident, $key:expr, $url:expr, $family:expr, $client:ident, $body:expr) => {{
        let mut builder = rig::providers::$module::Client::builder().api_key($key);
        if let Some(url) = $url {
            builder = builder.base_url(url);
        }
        let $client = builder
            .build()
            .map_err(|source| ProviderRegistryError::ClientConstruction {
                family: $family.to_string(),
                source,
            })?;
        $body
    }};
}

/// Like [`with_client`], restricted to the families whose rig clients
/// implement [`ModelListingClient`]. A compile-time-filtered copy is needed
/// because the shared macro would otherwise require the listing trait bound
/// on clients that do not provide it.
macro_rules! with_listing_client {
    ($family:expr, $key:expr, $url:expr, |$client:ident| $body:expr) => {
        match $family {
            "anthropic" => with_client!(@arm anthropic, $key, $url, $family, $client, $body),
            "copilot" => with_client!(@arm copilot, $key, $url, $family, $client, $body),
            "deepseek" => with_client!(@arm deepseek, $key, $url, $family, $client, $body),
            "gemini" => with_client!(@arm gemini, $key, $url, $family, $client, $body),
            "groq" => with_client!(@arm groq, $key, $url, $family, $client, $body),
            "llamacpp" => with_client!(@arm llamacpp, $key, $url, $family, $client, $body),
            "minimax" => with_client!(@arm minimax, $key, $url, $family, $client, $body),
            "mira" => with_client!(@arm mira, $key, $url, $family, $client, $body),
            "mistral" => with_client!(@arm mistral, $key, $url, $family, $client, $body),
            "moonshot" => with_client!(@arm moonshot, $key, $url, $family, $client, $body),
            "ollama" => with_client!(@arm ollama, $key, $url, $family, $client, $body),
            "openai" => with_client!(@arm openai, $key, $url, $family, $client, $body),
            "openrouter" => with_client!(@arm openrouter, $key, $url, $family, $client, $body),
            "venice" => with_client!(@arm venice, $key, $url, $family, $client, $body),
            "xiaomimimo" => with_client!(@arm xiaomimimo, $key, $url, $family, $client, $body),
            other => return Err(ProviderRegistryError::UnknownFamily(other.to_string())),
        }
    };
}

/// Validate shared inputs and normalize them for client construction.
///
/// Returns the family descriptor plus the (possibly empty) key string rig
/// should receive. Local-server families (ollama, llamacpp) treat an empty
/// key as "no auth", so they never fail validation here.
fn validate<'a>(
    settings_family: &str,
    api_key: Option<&'a str>,
    base_url: Option<&'a str>,
) -> Result<(&'static FamilySpec, &'a str, Option<&'a str>), ProviderRegistryError> {
    let fam = lookup_family(settings_family)?;
    let key = api_key.map(str::trim).unwrap_or_default();
    if fam.requires_api_key && key.is_empty() {
        return Err(ProviderRegistryError::MissingApiKey(fam.id.to_string()));
    }
    let url = base_url.map(str::trim).filter(|u| !u.is_empty());
    if settings_family == "azure" && url.is_none() {
        // Azure has no default endpoint: the resource URL identifies the
        // deployment, so it must come from settings.
        return Err(ProviderRegistryError::MissingBaseUrl(fam.id.to_string()));
    }
    Ok((fam, key, url))
}

/// Build a rig [`ModelHandle`] for the given settings.
///
/// Validates the key/base URL requirements of the family first, then
/// constructs the provider's client and its completion model. Construction
/// performs no network I/O; the first completion request pays any
/// authentication/verification cost.
pub fn build_model(settings: &ProviderSettings) -> Result<ModelHandle, ProviderRegistryError> {
    let (_, key, url) = validate(
        &settings.family,
        settings.api_key.as_deref(),
        settings.base_url.as_deref(),
    )?;
    with_client!(settings.family.as_str(), key, url, |client| Ok(
        ModelHandle::new(client.completion_model(&settings.model))
    ))
}

/// List the models a family offers, via rig's model-listing capability.
///
/// `api_key`/`base_url` follow the same rules as in [`build_model`]. Returns
/// [`ProviderRegistryError::ListingNotSupported`] for families whose rig
/// clients do not implement model listing.
pub async fn list_models(
    family: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<Vec<ProviderModel>, ProviderRegistryError> {
    let fam = lookup_family(family)?;
    if !fam.supports_model_listing {
        return Err(ProviderRegistryError::ListingNotSupported(
            fam.id.to_string(),
        ));
    }
    let (_, key, url) = validate(family, api_key, base_url)?;
    let models = with_listing_client!(family, key, url, |client| client
        .list_models()
        .await
        .map_err(|source| ProviderRegistryError::ListModels {
            family: family.to_string(),
            source,
        })?);
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

    #[test]
    fn family_table_is_complete_and_unique() {
        let families = list_families();
        assert_eq!(families.len(), 25, "every completion family is registered");

        let mut ids: Vec<&str> = families.iter().map(|f| f.id.as_str()).collect();
        ids.sort_unstable();
        let unique: Vec<&str> = ids
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        assert_eq!(ids, unique, "family ids must be unique");

        // Key fields are non-empty for every descriptor.
        for f in &families {
            assert!(!f.id.is_empty(), "id must not be empty");
            assert!(!f.name.is_empty(), "name must not be empty for {}", f.id);
        }

        // Spot-check representatives of each shape.
        let openai = families.iter().find(|f| f.id == "openai").unwrap();
        assert!(
            openai.requires_api_key && openai.supports_base_url && openai.supports_model_listing
        );
        let ollama = families.iter().find(|f| f.id == "ollama").unwrap();
        assert!(
            !ollama.requires_api_key && ollama.supports_base_url && ollama.supports_model_listing
        );
        let voyageai = families.iter().find(|f| f.id == "voyageai");
        assert!(voyageai.is_none(), "embeddings-only families are excluded");
    }

    #[test]
    fn settings_serde_round_trip() {
        let settings = ProviderSettings {
            family: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            api_key: Some("sk-test".into()),
            base_url: None,
        };
        let json = serde_json::to_string(&settings).expect("serialize");
        let back: ProviderSettings = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(settings, back);

        // Tolerates absent optional fields.
        let minimal: ProviderSettings =
            serde_json::from_str(r#"{"family":"ollama","model":"llama3"}"#).expect("minimal");
        assert_eq!(minimal.api_key, None);
        assert_eq!(minimal.base_url, None);
    }

    #[test]
    fn build_model_rejects_missing_api_key() {
        let settings = ProviderSettings {
            family: "openai".into(),
            model: "gpt-4o-mini".into(),
            api_key: None,
            base_url: None,
        };
        match build_model(&settings) {
            Err(ProviderRegistryError::MissingApiKey(family)) => assert_eq!(family, "openai"),
            other => panic!("expected MissingApiKey, got {:?}", other.map(|_| ())),
        }

        let settings = ProviderSettings {
            family: "anthropic".into(),
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
            family: "azure".into(),
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
        let settings = ProviderSettings {
            family: "not-a-provider".into(),
            model: "m".into(),
            api_key: Some("k".into()),
            base_url: None,
        };
        assert!(matches!(
            build_model(&settings),
            Err(ProviderRegistryError::UnknownFamily(_))
        ));
    }

    #[test]
    fn local_families_build_without_api_key() {
        // ollama accepts an empty key (rig maps it to "no auth") and its
        // client defaults to localhost — construction must succeed offline.
        let settings = ProviderSettings {
            family: "ollama".into(),
            model: "llama3".into(),
            api_key: None,
            base_url: None,
        };
        let handle = build_model(&settings).expect("ollama client builds without key");
        let _ = handle; // opaque ModelHandle

        let settings = ProviderSettings {
            family: "llamacpp".into(),
            model: "gemma-2".into(),
            api_key: None,
            base_url: Some("http://127.0.0.1:8080".into()),
        };
        assert!(build_model(&settings).is_ok());
    }

    #[test]
    fn remote_families_build_offline_with_dummy_key() {
        // Client construction is pure configuration; no network happens here.
        for family in ["openai", "anthropic", "groq", "gemini", "azure"] {
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
        let err = list_models("cohere", Some("k"), None).await.unwrap_err();
        assert!(matches!(err, ProviderRegistryError::ListingNotSupported(f) if f == "cohere"));
        let err = list_models("voyageai", Some("k"), None).await.unwrap_err();
        assert!(matches!(err, ProviderRegistryError::UnknownFamily(_)));
    }
}
