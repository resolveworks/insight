/**
 * Unified provider state store.
 * Tracks both embedding and language providers with their status.
 * Queries backend for initial state and listens for real-time updates.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type ProviderType = 'embedding' | 'language';

export interface DownloadProgress {
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

export interface ProviderState {
	/** Provider type: "local", "openai", "anthropic", or null if not configured */
	providerType: string | null;
	/** Model ID within the provider */
	modelId: string | null;
	/** Whether the provider is ready to use */
	ready: boolean;
	/** Error message if failed */
	error: string | null;
	/** Download progress (only for local providers during download) */
	progress: DownloadProgress | null;
}

interface ProviderStatusResponse {
	provider_type: string | null;
	model_id: string | null;
	ready: boolean;
	error: string | null;
}

interface ModelStatusEvent {
	status: 'downloading' | 'loading' | 'ready' | 'failed';
	model_type: ProviderType;
	model_id: string;
	model_name?: string;
	error?: string;
}

interface ModelDownloadProgressEvent {
	model_type: ProviderType;
	file: string;
	downloaded: number;
	total: number;
	overall_progress: number;
	file_index: number;
	total_files: number;
}

function createInitialState(): ProviderState {
	return {
		providerType: null,
		modelId: null,
		ready: false,
		error: null,
		progress: null,
	};
}

// Module-level reactive state for both providers
const embeddingState = $state<ProviderState>(createInitialState());
const languageState = $state<ProviderState>(createInitialState());

// Track unsubscribe functions for cleanup
let unlistenStatus: UnlistenFn | null = null;
let unlistenProgress: UnlistenFn | null = null;

function getStateForType(type: ProviderType): ProviderState {
	return type === 'embedding' ? embeddingState : languageState;
}

function updateState(type: ProviderType, updates: Partial<ProviderState>) {
	const state = getStateForType(type);
	Object.assign(state, updates);
}

async function queryInitialStatus(type: ProviderType) {
	try {
		const status = await invoke<ProviderStatusResponse>('get_provider_status', {
			modelType: type,
		});
		updateState(type, {
			providerType: status.provider_type,
			modelId: status.model_id,
			ready: status.ready,
			error: status.error,
			progress: null,
		});
	} catch (e) {
		console.error(`Failed to get ${type} provider status:`, e);
	}
}

async function setupEventListeners() {
	// Listen for status changes
	unlistenStatus = await listen<ModelStatusEvent>(
		'model-status-changed',
		(e) => {
			const { status, model_type, model_id, error } = e.payload;
			const state = getStateForType(model_type);

			updateState(model_type, {
				modelId: model_id,
				ready: status === 'ready',
				error: status === 'failed' ? (error ?? 'Unknown error') : null,
				progress: status === 'downloading' ? state.progress : null,
			});
		},
	);

	// Listen for download progress
	unlistenProgress = await listen<ModelDownloadProgressEvent>(
		'model-download-progress',
		(e) => {
			const { model_type, ...progress } = e.payload;
			updateState(model_type, { progress });
		},
	);
}

// Initialize on module load
if (typeof window !== 'undefined') {
	// Query initial status for both providers
	queryInitialStatus('embedding');
	queryInitialStatus('language');

	// Setup event listeners
	setupEventListeners();
}

/**
 * Cleanup event listeners. Call this on app unmount if needed.
 */
export function cleanup() {
	unlistenStatus?.();
	unlistenProgress?.();
	unlistenStatus = null;
	unlistenProgress = null;
}

/**
 * Get the provider state for embedding or language.
 */
export function getProviderState(type: ProviderType): ProviderState {
	return getStateForType(type);
}

/**
 * Get the embedding provider state.
 */
export function getEmbeddingState(): ProviderState {
	return embeddingState;
}

/**
 * Get the language provider state.
 */
export function getLanguageState(): ProviderState {
	return languageState;
}

/**
 * Manually update the language provider state.
 * Used when configuring remote providers (OpenAI, Anthropic).
 */
export function setLanguageProvider(
	providerType: string | null,
	modelId: string | null,
) {
	updateState('language', {
		providerType,
		modelId,
		ready: providerType !== null, // Remote providers are ready once configured
		error: null,
		progress: null,
	});
}

/**
 * Get provider display name.
 */
export function getProviderDisplayName(providerType: string | null): string {
	switch (providerType) {
		case 'local':
			return 'Local';
		case 'openai':
			return 'OpenAI';
		case 'anthropic':
			return 'Anthropic';
		default:
			return 'Not configured';
	}
}
