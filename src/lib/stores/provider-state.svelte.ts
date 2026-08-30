/**
 * Chat provider state store.
 *
 * Mirrors the backend's chat provider lifecycle as a tagged union so every
 * consumer can match on `status.kind` instead of reconstructing the machine
 * from independent booleans.
 *
 * - Persistent state (configured/ready vs unconfigured) comes from
 *   `get_provider_status` at boot.
 * - Transient state (loading, error) arrives via `model-status-changed`
 *   events.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type ProviderStatus =
	| { kind: 'unconfigured' }
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
	status: 'loading' | 'ready' | 'failed';
	model_type: 'language';
	model_id: string;
	error?: string;
}

const languageState = $state<ProviderState>({
	providerType: null,
	modelId: null,
	status: { kind: 'unconfigured' },
});

let unlistenStatus: UnlistenFn | null = null;

async function queryInitialStatus() {
	try {
		const res = await invoke<ProviderStatusResponse>('get_provider_status');
		languageState.providerType = res.provider_type;
		languageState.modelId = res.model_id;
		languageState.status = res.ready
			? { kind: 'ready' }
			: { kind: 'unconfigured' };
	} catch (e) {
		console.error('Failed to get chat provider status:', e);
	}
}

async function setupEventListeners() {
	unlistenStatus = await listen<ModelStatusEvent>(
		'model-status-changed',
		(e) => {
			const { status, model_id, error } = e.payload;
			languageState.modelId = model_id;

			switch (status) {
				case 'loading':
					languageState.status = { kind: 'loading' };
					break;
				case 'ready':
					languageState.status = { kind: 'ready' };
					break;
				case 'failed':
					languageState.status = {
						kind: 'error',
						message: error ?? 'Unknown error',
					};
					break;
			}
		},
	);
}

if (typeof window !== 'undefined') {
	queryInitialStatus();
	setupEventListeners();
}

export function cleanup() {
	unlistenStatus?.();
	unlistenStatus = null;
}

/**
 * Re-sync from the backend's persistent state. Useful after actions that
 * don't emit a terminal status event.
 */
export async function refreshProviderState(): Promise<void> {
	await queryInitialStatus();
}

export function getLanguageState(): ProviderState {
	return languageState;
}

export function getProviderState(): ProviderState {
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
		case 'openai':
			return 'OpenAI';
		case 'anthropic':
			return 'Anthropic';
		default:
			return 'Not configured';
	}
}
