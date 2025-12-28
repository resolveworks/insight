/**
 * Global store for import progress state.
 * Persists across page navigation by tracking at module level.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface ImportProgress {
	/** Collection being imported to */
	collectionId: string;
	/** Total files being imported */
	total: number;
	/** Files successfully imported */
	completed: number;
	/** Files that failed to import */
	failed: { path: string; error: string }[];
}

export interface ImportState {
	/** Whether an import is in progress */
	importing: boolean;
	/** Current import progress (if importing) */
	progress: ImportProgress | null;
}

const initialState: ImportState = {
	importing: false,
	progress: null,
};

// Module-level reactive state (persists across component mounts)
const importState = $state<ImportState>({ ...initialState });

// Track if listeners have been set up
let listenersInitialized = false;
const unlisteners: UnlistenFn[] = [];

async function setupListeners() {
	if (listenersInitialized) return;
	listenersInitialized = true;

	// Listen for document-added events to track progress
	unlisteners.push(
		await listen<{ collection_id: string; document: unknown }>(
			'document-added',
			(event) => {
				if (
					importState.progress &&
					importState.progress.collectionId === event.payload.collection_id
				) {
					importState.progress.completed++;
				}
			},
		),
	);
}

// Initialize listeners when module is imported (in browser context)
if (typeof window !== 'undefined') {
	setupListeners();
}

/** Start tracking an import */
export function startImport(collectionId: string, totalFiles: number) {
	importState.importing = true;
	importState.progress = {
		collectionId,
		total: totalFiles,
		completed: 0,
		failed: [],
	};
}

/** Record failed imports */
export function recordFailures(failures: { path: string; error: string }[]) {
	if (importState.progress) {
		importState.progress.failed = failures;
	}
}

/** Complete the import */
export function completeImport() {
	importState.importing = false;
	importState.progress = null;
}

/** Get the current import state */
export function getImportState(): ImportState {
	return importState;
}

/** Get remaining count (total - completed - failed) */
export function getRemainingCount(): number {
	if (!importState.progress) return 0;
	return (
		importState.progress.total -
		importState.progress.completed -
		importState.progress.failed.length
	);
}
