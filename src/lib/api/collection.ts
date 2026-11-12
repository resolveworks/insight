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

/** Create a collaboration ticket that grants write access to a collection. */
export async function shareCollection(collectionId: string): Promise<string> {
	const ticket = await invoke<string>('share_collection', { collectionId });
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
export async function importCollection(ticket: string): Promise<string> {
	const collectionId = await invoke<string>('import_collection', {
		ticket: ticket.trim(),
	});
	await refreshCollections();
	return collectionId;
}

/** A collection's display name, pending metadata included. */
export function collectionDisplayName(
	collection: Pick<Collection, 'id' | 'name'>,
): string {
	return collection.name ?? `Shared collection ${collection.id.slice(0, 8)}`;
}

/** A collection's document count with its unit, e.g. "1 document". */
export function documentCountLabel(count: number): string {
	return `${count} ${count === 1 ? 'document' : 'documents'}`;
}
