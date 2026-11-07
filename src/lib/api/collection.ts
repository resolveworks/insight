import { invoke } from '@tauri-apps/api/core';
import { invalidate } from '$app/navigation';
import type { Collection } from '#lib/types/documents.js';

/** List every collection. */
export async function listCollections(): Promise<Collection[]> {
	return invoke<Collection[]>('get_collections');
}

/** Create a collection and re-read the collection list. */
export async function createCollection(name: string): Promise<Collection> {
	const collection = await invoke<Collection>('create_collection', { name });
	await invalidate('app:collections');
	return collection;
}

/** Delete a collection and re-read the collection list. */
export async function deleteCollection(collectionId: string): Promise<void> {
	await invoke('delete_collection', { collectionId });
	await invalidate('app:collections');
}

/** Generate a read-only share ticket for a collection. */
export async function shareCollection(collectionId: string): Promise<string> {
	const ticket = await invoke<string>('share_collection', {
		collectionId,
		writable: false,
	});
	await invalidate('app:collections');
	return ticket;
}

/** Import a shared collection and re-read the collection list. */
export async function importCollection(ticket: string): Promise<Collection> {
	const collection = await invoke<Collection>('import_collection', {
		ticket: ticket.trim(),
	});
	await invalidate('app:collections');
	return collection;
}
