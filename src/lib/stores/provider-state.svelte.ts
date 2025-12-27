/**
 * Global store for active provider state.
 * Persists across page navigation by using module-level state.
 */
import { invoke } from '@tauri-apps/api/core';

export type ProviderConfig =
	| { type: 'local'; model_id: string }
	| { type: 'openai'; api_key: string; model: string }
	| { type: 'anthropic'; api_key: string; model: string };

export interface ProviderState {
	/** Current active provider config */
	provider: ProviderConfig | null;
	/** Whether we've loaded initial state from backend */
	initialized: boolean;
	/** Loading state for async operations */
	loading: boolean;
}

// Module-level reactive state (persists across component mounts)
const providerState = $state<ProviderState>({
	provider: null,
	initialized: false,
	loading: false,
});

let initPromise: Promise<void> | null = null;

/**
 * Initialize provider state from backend.
 * Safe to call multiple times - only fetches once.
 */
export async function initProviderState(): Promise<void> {
	if (providerState.initialized) return;

	// Deduplicate concurrent init calls
	if (initPromise) return initPromise;

	initPromise = (async () => {
		try {
			providerState.loading = true;
			const config = await invoke<ProviderConfig | null>(
				'get_current_provider',
			);
			providerState.provider = config;
		} catch (e) {
			console.error('Failed to get current provider:', e);
		} finally {
			providerState.loading = false;
			providerState.initialized = true;
		}
	})();

	return initPromise;
}

/**
 * Set the active provider.
 * Updates both local state and persists to backend.
 */
export function setProvider(config: ProviderConfig | null) {
	providerState.provider = config;
}

/**
 * Get the current provider state (reactive).
 */
export function getProviderState(): ProviderState {
	return providerState;
}

/**
 * Get provider display name.
 */
export function getProviderDisplayName(config: ProviderConfig): string {
	switch (config.type) {
		case 'local':
			return 'Local';
		case 'openai':
			return 'OpenAI';
		case 'anthropic':
			return 'Anthropic';
	}
}

/**
 * Get model display for a provider config.
 */
export function getProviderModelDisplay(config: ProviderConfig): string {
	switch (config.type) {
		case 'local':
			return config.model_id;
		case 'openai':
		case 'anthropic':
			return config.model;
	}
}

// Initialize when module is imported (in browser context)
if (typeof window !== 'undefined') {
	initProviderState();
}
