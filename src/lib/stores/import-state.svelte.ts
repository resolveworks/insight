/**
 * Global store for import progress state.
 * Tracks in-flight imports across page navigation.
 */
import { listen } from '@tauri-apps/api/event';

export interface ImportProgress {
	collectionId: string;
	total: number;
	completed: number;
	failed: { path: string; error: string }[];
}

const importState = $state<{
	importing: boolean;
	progress: ImportProgress | null;
}>({
	importing: false,
	progress: null,
});

// Listen for document-added events to track progress
if (typeof window !== 'undefined') {
	listen<{ collection_id: string; document: unknown }>(
		'document-added',
		(event) => {
			if (importState.progress?.collectionId === event.payload.collection_id) {
				importState.progress.completed++;
			}
		},
	);
}

export function startImport(collectionId: string, totalFiles: number) {
	importState.importing = true;
	importState.progress = {
		collectionId,
		total: totalFiles,
		completed: 0,
		failed: [],
	};
}

export function recordFailures(failures: { path: string; error: string }[]) {
	if (importState.progress) {
		importState.progress.failed = failures;
	}
}

export function completeImport() {
	importState.importing = false;
	importState.progress = null;
}

export function getImportState() {
	return importState;
}

export function getRemainingCount(): number {
	if (!importState.progress) return 0;
	return (
		importState.progress.total -
		importState.progress.completed -
		importState.progress.failed.length
	);
}
