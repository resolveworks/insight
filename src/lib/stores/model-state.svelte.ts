/**
 * Global store for model state.
 * Queries backend for initial state and listens for real-time updates.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

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

interface ModelStatusEvent {
	status: 'downloading' | 'loading' | 'ready' | 'failed';
	model_type: ModelType;
	model_id: string;
	model_name?: string;
	error?: string;
}

interface ModelDownloadProgressEvent {
	model_type: ModelType;
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

interface EmbeddingStatus {
	ready: boolean;
	error: string | null;
	model_id: string | null;
}

const embeddingState = $state<ModelState>({
	ready: false,
	error: null,
	progress: null,
});
const languageState = $state<ModelState>({
	ready: false,
	error: null,
	progress: null,
});

function updateState(modelType: ModelType, updates: Partial<ModelState>) {
	const state = modelType === 'embedding' ? embeddingState : languageState;
	Object.assign(state, updates);
}

// Initialize on module load
if (typeof window !== 'undefined') {
	// Query current embedding state
	invoke<EmbeddingStatus>('get_embedding_status')
		.then((status) => {
			updateState('embedding', {
				ready: status.ready,
				error: status.error,
				progress: null,
			});
		})
		.catch((e) => console.error('Failed to get embedding status:', e));

	// Listen for status changes
	listen<ModelStatusEvent>('model-status-changed', (e) => {
		const { status, model_type, error } = e.payload;
		const state = model_type === 'embedding' ? embeddingState : languageState;

		updateState(model_type, {
			ready: status === 'ready',
			error: status === 'failed' ? (error ?? 'Unknown error') : null,
			progress: status === 'downloading' ? state.progress : null,
		});
	});

	// Listen for download progress
	listen<ModelDownloadProgressEvent>('model-download-progress', (e) => {
		const { model_type, ...progress } = e.payload;
		updateState(model_type, { progress });
	});
}

export function getModelState(modelType: ModelType): ModelState {
	return modelType === 'embedding' ? embeddingState : languageState;
}

export function getEmbeddingState(): ModelState {
	return embeddingState;
}

export function getLanguageState(): ModelState {
	return languageState;
}
