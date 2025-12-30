/**
 * Global store for import progress state.
 * Tracks active imports via backend events.
 */
import { listen } from '@tauri-apps/api/event';

export interface ImportProgress {
	collection_id: string;
	total: number;
	completed: number;
	failed: number;
	pending: number;
	in_progress: number;
}

// Plain object for reactivity - reassign to trigger updates
let progressByCollection = $state<Record<string, ImportProgress>>({});

// Listen for progress updates from backend
if (typeof window !== 'undefined') {
	listen<ImportProgress>('import-progress', (event) => {
		const progress = event.payload;
		if (progress.pending === 0 && progress.in_progress === 0) {
			// Import complete, remove from tracking
			delete progressByCollection[progress.collection_id];
			progressByCollection = { ...progressByCollection };
		} else {
			progressByCollection = {
				...progressByCollection,
				[progress.collection_id]: progress,
			};
		}
	});
}

/** Get import progress for a specific collection */
export function getCollectionProgress(
	collectionId: string,
): ImportProgress | undefined {
	return progressByCollection[collectionId];
}

/** Check if any imports are active for a collection */
export function isImporting(collectionId: string): boolean {
	const progress = progressByCollection[collectionId];
	if (!progress) return false;
	return progress.pending > 0 || progress.in_progress > 0;
}

/** Get all active import progress */
export function getAllProgress(): ImportProgress[] {
	return Object.values(progressByCollection);
}

/** Check if any imports are active globally */
export function hasActiveImports(): boolean {
	return Object.keys(progressByCollection).length > 0;
}
