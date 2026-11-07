import { listen } from '@tauri-apps/api/event';
import { invalidate } from '$app/navigation';
import type { PipelineProgress, StageProgress } from '#lib/types/documents.js';

interface CollectionInvalidated {
	collection_id: string;
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

async function invalidateCollection(collectionId: string): Promise<void> {
	await Promise.all([
		invalidate('app:collections'),
		invalidate(`app:collection:${collectionId}`),
	]);
}

/**
 * Peer sync is the one change the local process cannot know about, and a
 * collection's pipeline going idle is when extraction has finished changing
 * page counts. Both re-read the collection list and the affected collection.
 */
export async function startInvalidationListeners(): Promise<void> {
	await listen<CollectionInvalidated>('collection-invalidated', (event) => {
		void invalidateCollection(event.payload.collection_id);
	});
	await listen<PipelineProgress>('pipeline-progress', (event) => {
		if (!pipelineIsActive(event.payload)) {
			void invalidateCollection(event.payload.collection_id);
		}
	});
}
