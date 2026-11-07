//! The chat providers Insight offers, as rig registers them.
//!
//! Insight adds nothing vendor-specific here. Which selections exist, what
//! models each vendor serves, whether a credential is required and where the
//! default endpoint is all come from rig's registry and its built-in model
//! catalog; this module only persists a selection as settings and turns it
//! into a model handle.
//!
//! - [`selections`] lists every selectable provider, canonically spelled
//!   (`"openai/openai"`, `"gcp.gemini/gemini"`, `"ollama/openai"`).
//! - [`models`] lists the chat models rig's catalog files under a provider's
//!   vendor, with rig's display names.
//! - [`build_model`] builds the [`DynModel<Completion>`] for persisted
//!   settings, with the catalog facts rig's wires encode and price with.

use rig_core::catalog::Catalog;
use rig_core::providers::registry::{ProviderConfig, ProviderId, SelectionError};
use rig_core::{operation::Completion, DynModel};
use serde::{Deserialize, Serialize};

/// A provider configuration as persisted to `settings.json`.
///
/// `family` is a rig provider selection (`"openai/openai"`, or a bare vendor
/// when rig registers one family for it); `model` is the provider's model id.
/// `api_key` is required only by selections rig says need a credential, and
/// `base_url` overrides the selection's endpoint when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSettings {
    /// rig provider selection this configuration reaches.
    pub family: String,
    /// Provider-side model identifier.
    pub model: String,
    /// API key (or access token) for the selection, if it requires one.
    pub api_key: Option<String>,
    /// Custom base URL / endpoint, overriding the provider default.
    pub base_url: Option<String>,
}

/// One chat model of a provider, as the model field offers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelChoice {
    /// Provider-side model identifier, usable as [`ProviderSettings::model`].
    pub id: String,
    /// rig's human-readable name, falling back to the id when the catalog
    /// lists none.
    pub name: String,
}

/// Errors from building a provider.
#[derive(Debug, thiserror::Error)]
pub enum ProviderRegistryError {
    /// The family is not a provider selection rig registers.
    #[error(transparent)]
    Selection(#[from] SelectionError),

    /// The selection requires a credential but none was configured.
    #[error("provider '{0}' requires an API key")]
    MissingApiKey(String),

    /// The selection has no default endpoint and none was configured.
    #[error("provider '{0}' requires a base URL")]
    MissingBaseUrl(String),
}

/// Every provider selection rig registers, canonically spelled.
///
/// Sorted, so the UI's order is stable across builds.
pub fn selections() -> Vec<String> {
    let mut ids: Vec<String> = ProviderId::all().map(|id| id.to_string()).collect();
    ids.sort();
    ids
}

/// The chat models `family`'s vendor serves, in rig's display order.
///
/// Drawn from rig's built-in catalog: rows the catalog marks as another kind
/// (embeddings, image, realtime) or as deprecated are left out. No network
/// call is made.
pub fn models(family: &str) -> Result<Vec<ModelChoice>, ProviderRegistryError> {
    let id = ProviderId::resolve(family)?;
    let vendor = id.vendor();
    Ok(Catalog::builtin()
        .iter()
        .filter(|spec| spec.provider.vendor() == vendor)
        .filter(|spec| spec.details.kind.is_none() && !spec.deprecated)
        .map(|spec| ModelChoice {
            id: spec.id.clone(),
            name: spec.display_name.clone(),
        })
        .collect())
}

/// Resolve `family` and configure its provider.
///
/// The credential requirement comes from the selection; a configuration with
/// no default endpoint (Azure) rejects an absent base URL. A non-empty custom
/// base URL replaces the default verbatim; an empty one counts as absent.
fn configure(
    id: &ProviderId,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<ProviderConfig, ProviderRegistryError> {
    let family = id.to_string();
    let key = api_key.map(str::trim).unwrap_or_default();
    if id.requires_credential() && key.is_empty() {
        return Err(ProviderRegistryError::MissingApiKey(family));
    }
    let config = id
        .config(key)
        .expect("a registered provider selection configures");
    let config = match base_url.map(str::trim).filter(|url| !url.is_empty()) {
        Some(url) => config.with_base_url(url),
        None => config,
    };
    if config.base_url().is_empty() {
        return Err(ProviderRegistryError::MissingBaseUrl(family));
    }
    Ok(config)
}

/// Build a rig [`DynModel<Completion>`] for the given settings.
///
/// Validates the selection's key/base URL requirements, then constructs the
/// provider's completion model with the catalog facts for `model`, so
/// model-dependent routing, option refusals and pricing apply. Construction
/// performs no network I/O; the first completion request pays any
/// authentication cost.
pub fn build_model(
    settings: &ProviderSettings,
) -> Result<DynModel<Completion>, ProviderRegistryError> {
    let id = ProviderId::resolve(&settings.family)?;
    let config = configure(
        &id,
        settings.api_key.as_deref(),
        settings.base_url.as_deref(),
    )?;
    let facts = Catalog::builtin().facts(id, &settings.model);
    Ok(config.completion_model(&settings.model, rig::rig_reqwest::shared(), facts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(family: &str, api_key: Option<&str>, base_url: Option<&str>) -> ProviderSettings {
        ProviderSettings {
            family: family.into(),
            model: "test-model".into(),
            api_key: api_key.map(Into::into),
            base_url: base_url.map(Into::into),
        }
    }

    #[test]
    fn selections_are_sorted() {
        let selections = selections();
        let mut sorted = selections.clone();
        sorted.sort();
        assert_eq!(selections, sorted, "selections are sorted");
    }

    #[test]
    fn models_come_from_the_catalog_with_rigs_names() {
        let deepseek = models("deepseek").expect("bare vendor with one family resolves");
        assert_eq!(
            models("deepseek/openai").unwrap(),
            deepseek,
            "the vendor's models do not depend on how its selection is spelled"
        );
        let openai = models("openai/openai").unwrap();
        assert!(
            openai.iter().any(|choice| choice.id == "gpt-4o"),
            "chat models are listed"
        );
        assert!(
            openai
                .iter()
                .all(|choice| !choice.id.starts_with("text-embedding-")),
            "embedding models are left out"
        );
        assert!(
            openai.iter().all(|choice| choice.id != "gpt-4"),
            "deprecated models are left out"
        );
    }

    #[test]
    fn models_rejects_an_unknown_selection() {
        assert!(matches!(
            models("not-a-provider"),
            Err(ProviderRegistryError::Selection(_))
        ));
    }

    #[test]
    fn build_model_rejects_missing_api_key() {
        for key in [None, Some("  ")] {
            match build_model(&settings("openai/openai", key, None)) {
                Err(ProviderRegistryError::MissingApiKey(family)) => {
                    assert_eq!(family, "openai/openai")
                }
                other => panic!("expected MissingApiKey, got {:?}", other.map(|_| ())),
            }
        }
    }

    #[test]
    fn build_model_rejects_missing_base_url_for_azure() {
        assert!(matches!(
            build_model(&settings("azure.openai/openai", Some("key"), None)),
            Err(ProviderRegistryError::MissingBaseUrl(family)) if family == "azure.openai/openai"
        ));
    }

    #[test]
    fn build_model_rejects_an_unknown_selection() {
        for family in ["not-a-provider", "llamafile"] {
            assert!(
                matches!(
                    build_model(&settings(family, Some("k"), None)),
                    Err(ProviderRegistryError::Selection(_))
                ),
                "{family} is not a registered selection"
            );
        }
    }

    #[test]
    fn local_selections_build_without_api_key() {
        // A local daemon or server takes no credential, and construction
        // must succeed offline.
        assert!(build_model(&settings("ollama/openai", None, None)).is_ok());
        assert!(build_model(&settings(
            "llamacpp/openai",
            None,
            Some("http://127.0.0.1:8080")
        ))
        .is_ok());
    }

    #[test]
    fn build_model_treats_blank_base_url_as_missing() {
        assert!(matches!(
            build_model(&settings("azure.openai/openai", Some("key"), Some("   "))),
            Err(ProviderRegistryError::MissingBaseUrl(family)) if family == "azure.openai/openai"
        ));
    }
}
