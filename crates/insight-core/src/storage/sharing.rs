//! Collection sharing status.
//!
//! [`CollectionSharingStatus`] is the app-owned read model the UI consumes. It
//! combines stored capability, live sync state reported by iroh-docs, network
//! reachability, and live peer/sync events tracked by [`SharingTracker`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use iroh::PublicKey;
use iroh_docs::NamespaceId;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

/// Reachability of this node's home relays.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharingNetwork {
    /// At least one home relay is connected.
    Online,
    /// No home relay is connected and at least one has failed.
    Offline,
    /// No home relay is connected yet, and none has failed.
    Connecting,
}

/// Summary of the most recent peer sync for a collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastSyncSummary {
    /// Short display form of the peer's id.
    pub peer: String,
    /// When the sync finished.
    pub finished_at: DateTime<Utc>,
    /// Number of entries received from the peer.
    pub entries_received: usize,
    /// Number of entries sent to the peer.
    pub entries_sent: usize,
    /// The sync error, if the exchange failed.
    pub error: Option<String>,
}

/// Sharing status for one collection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectionSharingStatus {
    /// Whether the local replica accepts sync requests.
    pub sync_running: bool,
    /// Reachability of this node's home relays.
    pub network: SharingNetwork,
    /// Peers currently connected to this collection's live sync.
    pub connected_peers: usize,
    /// Previously successful sync peers, whether connected or not.
    pub known_peers: usize,
    /// The most recent peer sync, if one has finished since startup.
    pub last_sync: Option<LastSyncSummary>,
}

/// Error reading a collection's sharing status.
#[derive(Debug, thiserror::Error)]
pub enum SharingStatusError {
    /// The namespace is not stored locally.
    #[error("collection {0} not found")]
    CollectionNotFound(NamespaceId),
}

/// Live sharing state per collection.
///
/// The collection watcher records neighbor and sync events here; the storage
/// read model joins it with iroh-docs' stored and live state.
#[derive(Clone, Default)]
pub struct SharingTracker {
    live: Arc<RwLock<HashMap<NamespaceId, LiveState>>>,
}

#[derive(Debug, Default)]
struct LiveState {
    connected_peers: HashSet<PublicKey>,
    last_sync: Option<LastSyncSummary>,
}

impl SharingTracker {
    /// Create an empty tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record that a peer joined this collection's live sync.
    pub async fn peer_connected(&self, namespace: NamespaceId, peer: PublicKey) {
        self.live
            .write()
            .await
            .entry(namespace)
            .or_default()
            .connected_peers
            .insert(peer);
    }

    /// Record that a peer left this collection's live sync.
    pub async fn peer_disconnected(&self, namespace: NamespaceId, peer: PublicKey) {
        if let Some(state) = self.live.write().await.get_mut(&namespace) {
            state.connected_peers.remove(&peer);
        }
    }

    /// Record the outcome of a finished peer sync.
    pub async fn sync_finished(&self, namespace: NamespaceId, summary: LastSyncSummary) {
        self.live
            .write()
            .await
            .entry(namespace)
            .or_default()
            .last_sync = Some(summary);
    }

    /// Forget all live state for a collection.
    pub(crate) async fn clear(&self, namespace: NamespaceId) {
        self.live.write().await.remove(&namespace);
    }

    /// Read the live state for a collection.
    pub(crate) async fn snapshot(
        &self,
        namespace: NamespaceId,
    ) -> (usize, Option<LastSyncSummary>) {
        let live = self.live.read().await;
        match live.get(&namespace) {
            Some(state) => (state.connected_peers.len(), state.last_sync.clone()),
            None => (0, None),
        }
    }
}
