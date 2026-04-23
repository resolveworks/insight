/**
 * Provider state store.
 *
 * Mirrors the backend's model lifecycle as a tagged union so every consumer
 * can match on `status.kind` instead of reconstructing the machine from
 * independent booleans.
 *
 * - Persistent state (configured/ready vs unconfigured) comes from
 *   `get_provider_status` at boot.
 * - Transient state (downloading, loading, error) arrives via
 *   `model-status-changed` and `model-download-progress` events.
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

export type ProviderStatus =
	| { kind: 'unconfigured' }
	| { kind: 'downloading'; progress: DownloadProgress | null }
	| { kind: 'loading' }
	| { kind: 'ready' }
	| { kind: 'error'; message: string };

export interface ProviderState {
	providerType: string | null;
	modelId: string | null;
	status: ProviderStatus;
}

interface ProviderStatusResponse {
	provider_type: string | null;
	model_id: string | null;
	ready: boolean;
}

interface ModelStatusEvent {
	status: 'downloading' | 'loading' | 'ready' | 'failed';
	model_type: ProviderType;
	model_id: string;
	model_name?: string;
	error?: string;
}

interface ModelDownloadProgressEvent extends DownloadProgress {
	model_type: ProviderType;
}

function initialState(): ProviderState {
	return {
		providerType: null,
		modelId: null,
		status: { kind: 'unconfigured' },
	};
}

const embeddingState = $state<ProviderState>(initialState());
const languageState = $state<ProviderState>(initialState());

let unlistenStatus: UnlistenFn | null = null;
let unlistenProgress: UnlistenFn | null = null;

function stateFor(type: ProviderType): ProviderState {
	return type === 'embedding' ? embeddingState : languageState;
}

async function queryInitialStatus(type: ProviderType) {
	try {
		const res = await invoke<ProviderStatusResponse>('get_provider_status', {
			modelType: type,
		});
		const state = stateFor(type);
		state.providerType = res.provider_type;
		state.modelId = res.model_id;
		state.status = res.ready ? { kind: 'ready' } : { kind: 'unconfigured' };
	} catch (e) {
		console.error(`Failed to get ${type} provider status:`, e);
	}
}

async function setupEventListeners() {
	unlistenStatus = await listen<ModelStatusEvent>(
		'model-status-changed',
		(e) => {
			const { status, model_type, model_id, error } = e.payload;
			const state = stateFor(model_type);
			state.modelId = model_id;

			switch (status) {
				case 'downloading':
					// Preserve existing progress object across repeated events; the
					// progress channel fills it in as bytes arrive.
					state.status = {
						kind: 'downloading',
						progress:
							state.status.kind === 'downloading'
								? state.status.progress
								: null,
					};
					break;
				case 'loading':
					state.status = { kind: 'loading' };
					break;
				case 'ready':
					state.status = { kind: 'ready' };
					break;
				case 'failed':
					state.status = { kind: 'error', message: error ?? 'Unknown error' };
					break;
			}
		},
	);

	unlistenProgress = await listen<ModelDownloadProgressEvent>(
		'model-download-progress',
		(e) => {
			const { model_type, ...progress } = e.payload;
			const state = stateFor(model_type);
			// A progress tick implies we're downloading, even if the
			// status event hasn't arrived yet.
			state.status = { kind: 'downloading', progress };
		},
	);
}

if (typeof window !== 'undefined') {
	queryInitialStatus('embedding');
	queryInitialStatus('language');
	setupEventListeners();
}

export function cleanup() {
	unlistenStatus?.();
	unlistenProgress?.();
	unlistenStatus = null;
	unlistenProgress = null;
}

export function getProviderState(type: ProviderType): ProviderState {
	return stateFor(type);
}

/**
 * Re-sync from the backend's persistent state. Useful after actions like
 * `download_model` that don't emit a terminal status event (download only
 * puts files on disk — whether the model is "ready" depends on whether it's
 * the active one, which the backend already knows).
 */
export async function refreshProviderState(type: ProviderType): Promise<void> {
	await queryInitialStatus(type);
}

export function getEmbeddingState(): ProviderState {
	return embeddingState;
}

export function getLanguageState(): ProviderState {
	return languageState;
}

/**
 * Mark the language provider as configured (remote providers) or cleared.
 * Remote providers are instant: no backend events, just set the final state.
 */
export function setLanguageProvider(
	providerType: string | null,
	modelId: string | null,
) {
	languageState.providerType = providerType;
	languageState.modelId = modelId;
	languageState.status = providerType
		? { kind: 'ready' }
		: { kind: 'unconfigured' };
}

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
