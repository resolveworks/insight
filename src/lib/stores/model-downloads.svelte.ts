/**
 * Global store for model download state.
 * Persists download progress across page navigation by subscribing to Tauri events at module level.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
	languageModelConfig,
	embeddingModelConfig,
	type ModelSelectorConfig
} from '$lib/models/config';

export interface DownloadProgress {
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

export type DownloadStatus = 'idle' | 'downloading';

export interface DownloadState {
	status: DownloadStatus;
	progress: DownloadProgress | null;
	modelId: string | null;
}

const initialState: DownloadState = {
	status: 'idle',
	progress: null,
	modelId: null
};

// Module-level reactive state (persists across component mounts)
let languageModelDownload = $state<DownloadState>({ ...initialState });
let embeddingModelDownload = $state<DownloadState>({ ...initialState });

// Track if listeners have been set up
let listenersInitialized = false;
const unlisteners: UnlistenFn[] = [];

async function setupListeners() {
	if (listenersInitialized) return;
	listenersInitialized = true;

	// Language model events
	unlisteners.push(
		await listen<DownloadProgress>(languageModelConfig.progressEvent, (e) => {
			languageModelDownload.progress = e.payload;
			languageModelDownload.status = 'downloading';
		})
	);

	unlisteners.push(
		await listen(languageModelConfig.completeEvent, () => {
			languageModelDownload = { ...initialState };
		})
	);

	// Embedding model events
	unlisteners.push(
		await listen<DownloadProgress>(embeddingModelConfig.progressEvent, (e) => {
			embeddingModelDownload.progress = e.payload;
			embeddingModelDownload.status = 'downloading';
		})
	);

	unlisteners.push(
		await listen(embeddingModelConfig.completeEvent, () => {
			embeddingModelDownload = { ...initialState };
		})
	);
}

// Initialize listeners when module is imported (in browser context)
if (typeof window !== 'undefined') {
	setupListeners();
}

/** Start tracking a download for a model type */
export function startDownload(config: ModelSelectorConfig, modelId: string) {
	const state: DownloadState = {
		status: 'downloading',
		progress: null,
		modelId
	};

	if (config === languageModelConfig) {
		languageModelDownload = state;
	} else {
		embeddingModelDownload = state;
	}
}

/** Clear download state (e.g., on error) */
export function clearDownload(config: ModelSelectorConfig) {
	if (config === languageModelConfig) {
		languageModelDownload = { ...initialState };
	} else {
		embeddingModelDownload = { ...initialState };
	}
}

/** Get download state for a model type */
export function getDownloadState(config: ModelSelectorConfig): DownloadState {
	return config === languageModelConfig ? languageModelDownload : embeddingModelDownload;
}
