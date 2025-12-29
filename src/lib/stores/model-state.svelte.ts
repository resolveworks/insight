/**
 * Global store for model state.
 * Listens to model-status-changed and model-download-progress events from backend.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type ModelType = 'embedding' | 'language';

export interface DownloadProgress {
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

export interface ModelState {
	ready: boolean;
	error: string | null;
	progress: DownloadProgress | null;
}

/** Status event from backend */
interface ModelStatusEvent {
	status: 'downloading' | 'loading' | 'ready' | 'failed';
	model_type: ModelType;
	model_id: string;
	model_name?: string;
	error?: string;
}

/** Progress event from backend */
interface ModelDownloadProgressEvent {
	model_type: ModelType;
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

const initialState: ModelState = {
	ready: false,
	error: null,
	progress: null,
};

// Module-level reactive state (persists across component mounts)
const embeddingState = $state<ModelState>({ ...initialState });
const languageState = $state<ModelState>({ ...initialState });

// Track if listeners have been set up
let listenersInitialized = false;
const unlisteners: UnlistenFn[] = [];

function getStateForType(modelType: ModelType): ModelState {
	return modelType === 'embedding' ? embeddingState : languageState;
}

function updateState(modelType: ModelType, updates: Partial<ModelState>) {
	const state = modelType === 'embedding' ? embeddingState : languageState;
	Object.assign(state, updates);
}

async function setupListeners() {
	if (listenersInitialized) return;
	listenersInitialized = true;

	// Listen for status changes
	unlisteners.push(
		await listen<ModelStatusEvent>('model-status-changed', (e) => {
			const { status, model_type, error } = e.payload;

			updateState(model_type, {
				ready: status === 'ready',
				error: status === 'failed' ? (error ?? 'Unknown error') : null,
				// Clear progress when not downloading
				progress:
					status === 'downloading'
						? getStateForType(model_type).progress
						: null,
			});
		}),
	);

	// Listen for download progress
	unlisteners.push(
		await listen<ModelDownloadProgressEvent>('model-download-progress', (e) => {
			const { model_type, ...progress } = e.payload;

			updateState(model_type, { progress });
		}),
	);
}

// Initialize listeners when module is imported (in browser context)
if (typeof window !== 'undefined') {
	setupListeners();
}

/** Get model state for a model type */
export function getModelState(modelType: ModelType): ModelState {
	return modelType === 'embedding' ? embeddingState : languageState;
}

/** Get embedding model state */
export function getEmbeddingState(): ModelState {
	return embeddingState;
}

/** Get language model state */
export function getLanguageState(): ModelState {
	return languageState;
}
