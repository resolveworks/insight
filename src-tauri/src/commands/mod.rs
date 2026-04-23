use std::str::FromStr;

use iroh_docs::NamespaceId;
use serde::{Deserialize, Deserializer};

pub mod collections;
pub mod conversations;
pub mod documents;
pub mod models;
pub mod providers;

/// Tauri command parameter wrapping a [`NamespaceId`].
///
/// Deserializes from the hex string the frontend sends (via `NamespaceId`'s
/// `Display` impl) rather than the raw byte array `NamespaceId`'s own serde
/// impl expects. Parse failures surface as Tauri deserialization errors at
/// the command boundary.
#[derive(Debug, Clone, Copy)]
pub struct CollectionId(NamespaceId);

impl CollectionId {
    pub fn namespace(self) -> NamespaceId {
        self.0
    }
}

impl<'de> Deserialize<'de> for CollectionId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        NamespaceId::from_str(&s)
            .map(CollectionId)
            .map_err(serde::de::Error::custom)
    }
}
