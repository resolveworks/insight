import { describe, it, expect, beforeEach, vi } from 'vitest';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';

/**
 * The provider store keeps module-level $state and self-initializes on
 * import (window exists under jsdom), so each test re-imports a fresh module
 * copy with the IPC mock already installed.
 */
async function importStore() {
	vi.resetModules();
	return await import('./provider-state.svelte');
}

const FAMILIES = [
	{
		id: 'openai',
		name: 'OpenAI',
		requires_api_key: true,
		supports_base_url: false,
		supports_model_listing: true,
	},
	{
		id: 'ollama',
		name: 'Ollama',
		requires_api_key: false,
		supports_base_url: true,
		supports_model_listing: true,
	},
	{
		id: 'custom',
		name: 'Custom',
		requires_api_key: false,
		supports_base_url: true,
		supports_model_listing: false,
	},
];

describe('provider-state store', () => {
	beforeEach(() => {
		clearMocks();
	});

	it('loads the family registry, stored keys, and current provider at boot', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai',
					model: 'gpt-4o',
					api_key: 'sk-x',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'openai', model: 'gpt-4o', ready: true };
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-x' };
		});

		const store = await importStore();
		await store.loadFamilies();

		expect(store.getFamilies()).toEqual(FAMILIES);
		expect(store.getCurrentProvider()).toEqual({
			family: 'openai',
			model: 'gpt-4o',
			api_key: 'sk-x',
			base_url: null,
		});
		expect(store.getStatus()).toEqual({
			family: 'openai',
			model: 'gpt-4o',
			ready: true,
		});
		expect(store.isProviderReady()).toBe(true);
		expect(store.getStoredApiKey('openai')).toBe('sk-x');
	});

	it('treats a missing provider as not ready', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, ready: false };
			if (cmd === 'get_stored_api_keys') return {};
		});

		const store = await importStore();
		await store.ensureProviderStateLoaded();

		expect(store.getCurrentProvider()).toBeNull();
		expect(store.isProviderReady()).toBe(false);
		expect(store.isLoaded()).toBe(true);
	});

	it('configures a provider and refreshes state from the backend', async () => {
		let configuredFamily: string | null = null;
		let configuredModel: string | null = null;
		let configureArgs: Record<string, unknown> | null = null;

		mockIPC((cmd, args) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_stored_api_keys')
				return configuredFamily ? { [configuredFamily]: 'sk-new' } : {};
			if (cmd === 'get_current_provider')
				return configuredFamily
					? {
							family: configuredFamily,
							model: configuredModel,
							api_key: 'sk-new',
							base_url: null,
						}
					: null;
			if (cmd === 'get_provider_status')
				return configuredFamily
					? { family: configuredFamily, model: configuredModel, ready: true }
					: { family: null, model: null, ready: false };
			if (cmd === 'configure_provider') {
				const a = args as Record<string, unknown>;
				configureArgs = a;
				configuredFamily = a.family as string;
				configuredModel = a.model as string;
				return null;
			}
		});

		const store = await importStore();
		await store.loadFamilies();

		await store.configureProvider({
			family: 'openai',
			apiKey: 'sk-new',
			model: 'gpt-4o-mini',
		});

		expect(configureArgs).toMatchObject({
			family: 'openai',
			apiKey: 'sk-new',
			model: 'gpt-4o-mini',
		});
		expect(store.isProviderReady()).toBe(true);
		expect(store.getStoredApiKey('openai')).toBe('sk-new');
		expect(store.getCurrentProvider()?.model).toBe('gpt-4o-mini');
	});

	it('disableProvider clears the ready state', async () => {
		let disabled = false;
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'disable_provider') {
				disabled = true;
				return null;
			}
			if (cmd === 'get_current_provider')
				return disabled
					? null
					: {
							family: 'openai',
							model: 'gpt-4o',
							api_key: 'sk',
							base_url: null,
						};
			if (cmd === 'get_provider_status')
				return disabled
					? { family: null, model: null, ready: false }
					: { family: 'openai', model: 'gpt-4o', ready: true };
			if (cmd === 'get_stored_api_keys') return {};
		});

		const store = await importStore();
		expect(store.isProviderReady()).toBe(true);

		await store.disableProvider();
		expect(disabled).toBe(true);
		expect(store.isProviderReady()).toBe(false);
	});

	it('lists models for a family, forwarding key and base url', async () => {
		let listArgs: Record<string, unknown> | null = null;
		mockIPC((cmd, args) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'list_provider_models') {
				listArgs = args as Record<string, unknown>;
				return [{ id: 'qwen3', display_name: 'Qwen 3' }];
			}
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, ready: false };
			if (cmd === 'get_stored_api_keys') return {};
		});

		const store = await importStore();
		const models = await store.listProviderModels({
			family: 'ollama',
			apiKey: null,
			baseUrl: 'http://localhost:11434',
		});

		expect(models).toEqual([{ id: 'qwen3', display_name: 'Qwen 3' }]);
		expect(listArgs).toMatchObject({
			family: 'ollama',
			apiKey: null,
			baseUrl: 'http://localhost:11434',
		});
	});
});
