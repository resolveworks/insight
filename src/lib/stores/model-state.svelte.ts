/**
 * Global store for model download and loaded state.
 * Persists across page navigation by subscribing to Tauri events at module level.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import {
	languageModelConfig,
	embeddingModelConfig,
	type ModelSelectorConfig,
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

export interface ModelState {
	/** Download status */
	status: DownloadStatus;
	/** Download progress (if downloading) */
	progress: DownloadProgress | null;
	/** Model ID being downloaded */
	downloadingModelId: string | null;
	/** Currently loaded/active model ID */
	loadedModelId: string | null;
}

const initialState: ModelState = {
	status: 'idle',
	progress: null,
	downloadingModelId: null,
	loadedModelId: null,
};

// Module-level reactive state (persists across component mounts)
const languageModelState = $state<ModelState>({ ...initialState });
const embeddingModelState = $state<ModelState>({ ...initialState });

// Track if listeners have been set up
let listenersInitialized = false;
const unlisteners: UnlistenFn[] = [];

async function setupListeners() {
	if (listenersInitialized) return;
	listenersInitialized = true;

	// Query backend for currently loaded models
	try {
		const langModelId = await invoke<string | null>(
			languageModelConfig.currentCommand,
		);
		if (langModelId) {
			languageModelState.loadedModelId = langModelId;
		}
	} catch (e) {
		console.error('Failed to get current language model:', e);
	}

	try {
		const embModelId = await invoke<string | null>(
			embeddingModelConfig.currentCommand,
		);
		if (embModelId) {
			embeddingModelState.loadedModelId = embModelId;
		}
	} catch (e) {
		console.error('Failed to get current embedding model:', e);
	}

	// Language model events
	unlisteners.push(
		await listen<DownloadProgress>(languageModelConfig.progressEvent, (e) => {
			languageModelState.progress = e.payload;
			languageModelState.status = 'downloading';
		}),
	);

	unlisteners.push(
		await listen<string>(languageModelConfig.completeEvent, (e) => {
			languageModelState.status = 'idle';
			languageModelState.progress = null;
			languageModelState.downloadingModelId = null;
			languageModelState.loadedModelId = e.payload;
		}),
	);

	// Embedding model events
	unlisteners.push(
		await listen<DownloadProgress>(embeddingModelConfig.progressEvent, (e) => {
			embeddingModelState.progress = e.payload;
			embeddingModelState.status = 'downloading';
		}),
	);

	unlisteners.push(
		await listen<string>(embeddingModelConfig.completeEvent, (e) => {
			embeddingModelState.status = 'idle';
			embeddingModelState.progress = null;
			embeddingModelState.downloadingModelId = null;
			embeddingModelState.loadedModelId = e.payload;
		}),
	);
}

// Initialize listeners when module is imported (in browser context)
if (typeof window !== 'undefined') {
	setupListeners();
}

/** Start tracking a download for a model type */
export function startDownload(config: ModelSelectorConfig, modelId: string) {
	const state =
		config === languageModelConfig ? languageModelState : embeddingModelState;
	state.status = 'downloading';
	state.progress = null;
	state.downloadingModelId = modelId;
}

/** Clear download state (e.g., on error) */
export function clearDownload(config: ModelSelectorConfig) {
	const state =
		config === languageModelConfig ? languageModelState : embeddingModelState;
	state.status = 'idle';
	state.progress = null;
	state.downloadingModelId = null;
}

/** Set the loaded model (called after configure) */
export function setLoadedModel(config: ModelSelectorConfig, modelId: string) {
	const state =
		config === languageModelConfig ? languageModelState : embeddingModelState;
	state.loadedModelId = modelId;
}

/** Get model state for a model type */
export function getModelState(config: ModelSelectorConfig): ModelState {
	return config === languageModelConfig
		? languageModelState
		: embeddingModelState;
}

// Keep old name for backwards compatibility
export { getModelState as getDownloadState };
export type { ModelState as DownloadState };
