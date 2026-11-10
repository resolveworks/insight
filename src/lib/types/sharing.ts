/** What this node may do with a collection's entries. */
export type CollectionAccess = 'read_only' | 'write';

/** Reachability of this node's home relays. */
export type SharingNetwork = 'online' | 'offline' | 'connecting';

/** Summary of the most recent peer sync for a collection. */
export interface LastSyncSummary {
	peer: string;
	finished_at: string;
	entries_received: number;
	entries_sent: number;
	error: string | null;
}

/** Sharing status for one collection. */
export interface CollectionSharingStatus {
	access: CollectionAccess;
	sync_running: boolean;
	network: SharingNetwork;
	connected_peers: number;
	known_peers: number;
	last_sync: LastSyncSummary | null;
}
