/**
 * Global store for active provider state.
 * Queries backend on module load, components react to state changes.
 */
import { invoke } from '@tauri-apps/api/core';

export type ProviderConfig =
	| { type: 'local'; model_id: string }
	| { type: 'openai'; api_key: string; model: string }
	| { type: 'anthropic'; api_key: string; model: string };

// Module-level reactive state
const providerState = $state<{ provider: ProviderConfig | null }>({
	provider: null,
});

// Query backend on module import
if (typeof window !== 'undefined') {
	invoke<ProviderConfig | null>('get_current_provider')
		.then((config) => {
			providerState.provider = config;
		})
		.catch((e) => console.error('Failed to get provider:', e));
}

/**
 * Get the current provider state (reactive).
 */
export function getProviderState() {
	return providerState;
}

/**
 * Set the active provider.
 */
export function setProvider(config: ProviderConfig | null) {
	providerState.provider = config;
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
