import { invoke } from '@tauri-apps/api/core';
import { refreshCollections, refreshSharing } from '#lib/invalidation.js';
import type { Collection } from '#lib/types/documents.js';
import type { CollectionSharingStatus } from '#lib/types/sharing.js';

/** List every collection. */
export async function listCollections(): Promise<Collection[]> {
	return invoke<Collection[]>('get_collections');
}

/** Create a collection and re-read the collection list. */
export async function createCollection(name: string): Promise<Collection> {
	const collection = await invoke<Collection>('create_collection', { name });
	await refreshCollections();
	return collection;
}

/** Delete a collection and re-read the collection list. */
export async function deleteCollection(collectionId: string): Promise<void> {
	await invoke('delete_collection', { collectionId });
	await refreshCollections();
}

/** Generate a read-only share ticket for a collection. */
export async function shareCollection(collectionId: string): Promise<string> {
	const ticket = await invoke<string>('share_collection', {
		collectionId,
		writable: false,
	});
	await refreshCollections();
	await refreshSharing();
	return ticket;
}

/** Read a collection's current sharing status. */
export async function getCollectionSharingStatus(
	collectionId: string,
): Promise<CollectionSharingStatus> {
	return invoke<CollectionSharingStatus>('get_collection_sharing_status', {
		collectionId,
	});
}

/** Import a shared collection and re-read the collection list. */
export async function importCollection(ticket: string): Promise<Collection> {
	const collection = await invoke<Collection>('import_collection', {
		ticket: ticket.trim(),
	});
	await refreshCollections();
	return collection;
}

/** A collection's document count with its unit, e.g. "1 document". */
export function documentCountLabel(count: number): string {
	return `${count} ${count === 1 ? 'document' : 'documents'}`;
}
