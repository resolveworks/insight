/**
 * Global store for import progress state.
 * Queries backend for initial state and tracks updates via events.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ImportProgress {
	collection_id: string;
	total: number;
	completed: number;
	failed: number;
	pending: number;
	in_progress: number;
}

// Module-level reactive state
let progressByCollection = $state<Record<string, ImportProgress>>({});

// Track unlisten function for cleanup
let unlisten: UnlistenFn | null = null;

function updateProgress(progress: ImportProgress) {
	if (progress.pending === 0 && progress.in_progress === 0) {
		// Import complete, remove from tracking
		const updated = { ...progressByCollection };
		delete updated[progress.collection_id];
		progressByCollection = updated;
	} else {
		progressByCollection = {
			...progressByCollection,
			[progress.collection_id]: progress,
		};
	}
}

async function queryInitialState() {
	try {
		const allProgress = await invoke<ImportProgress[]>('get_import_progress');
		for (const progress of allProgress) {
			updateProgress(progress);
		}
	} catch (e) {
		console.error('Failed to get import progress:', e);
	}
}

async function setupEventListener() {
	unlisten = await listen<ImportProgress>('import-progress', (event) => {
		updateProgress(event.payload);
	});
}

// Initialize on module load
if (typeof window !== 'undefined') {
	queryInitialState();
	setupEventListener();
}

/**
 * Cleanup event listener. Call this on app unmount if needed.
 */
export function cleanup() {
	unlisten?.();
	unlisten = null;
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
