import { listen } from '@tauri-apps/api/event';
import { invalidate } from '$app/navigation';
import type { PipelineProgress, StageProgress } from '#lib/types/documents.js';

interface CollectionInvalidated {
	collection_id: string;
}

/** The data dependencies loads declare and mutations refresh. */
export const dataKeys = {
	collections: 'app:collections',
	provider: 'app:provider',
	collection: (collectionId: string): `app:collection:${string}` =>
		`app:collection:${collectionId}`,
	conversations: (collectionId: string): `app:conversations:${string}` =>
		`app:conversations:${collectionId}`,
} as const;

/** Re-read the collection list. */
export async function refreshCollections(): Promise<void> {
	await invalidate(dataKeys.collections);
}

/** Re-read a collection and the collection list. */
export async function refreshCollection(collectionId: string): Promise<void> {
	await Promise.all([
		invalidate(dataKeys.collections),
		invalidate(dataKeys.collection(collectionId)),
	]);
}

/** Re-read a collection's conversation list. */
export async function refreshConversations(
	collectionId: string,
): Promise<void> {
	await invalidate(dataKeys.conversations(collectionId));
}

/** Re-read the app-global provider configuration. */
export async function refreshProvider(): Promise<void> {
	await invalidate(dataKeys.provider);
}

function stageIsActive(stage: StageProgress): boolean {
	return stage.pending > 0 || stage.active > 0;
}

function pipelineIsActive(progress: PipelineProgress): boolean {
	return (
		stageIsActive(progress.store) ||
		stageIsActive(progress.extract) ||
		stageIsActive(progress.index)
	);
}

/**
 * Peer sync is the one change the local process cannot know about, and a
 * collection's pipeline going idle is when extraction has finished changing
 * page counts. Both re-read the collection list and the affected collection.
 */
export async function startInvalidationListeners(): Promise<void> {
	await listen<CollectionInvalidated>('collection-invalidated', (event) => {
		void refreshCollection(event.payload.collection_id);
	});
	await listen<PipelineProgress>('pipeline-progress', (event) => {
		if (!pipelineIsActive(event.payload)) {
			void refreshCollection(event.payload.collection_id);
		}
	});
}
