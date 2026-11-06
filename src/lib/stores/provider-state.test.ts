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

const SELECTIONS = ['ollama/openai', 'openai/openai'];

const MODELS: Record<string, Array<{ id: string; name: string }>> = {
	'openai/openai': [{ id: 'gpt-4o', name: 'GPT-4o' }],
	'ollama/openai': [{ id: 'llama3', name: 'Llama 3' }],
};

describe('provider-state store', () => {
	beforeEach(() => {
		clearMocks();
	});

	it('loads the selections, the current provider and its status at boot', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai/openai',
					model: 'gpt-4o',
					api_key: 'sk-x',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'openai/openai', model: 'gpt-4o', error: null };
		});

		const store = await importStore();
		await store.loadSelections();

		expect(store.getSelections()).toEqual(SELECTIONS);
		expect(store.getCurrentProvider()).toEqual({
			family: 'openai/openai',
			model: 'gpt-4o',
			api_key: 'sk-x',
			base_url: null,
		});
		expect(store.getStatus()).toEqual({
			family: 'openai/openai',
			model: 'gpt-4o',
			error: null,
		});
		expect(store.isProviderReady()).toBe(true);
		expect(store.isLoaded()).toBe(true);
	});

	it('treats a missing provider as not ready', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, error: null };
		});

		const store = await importStore();
		await store.__resetForTests();

		expect(store.getCurrentProvider()).toBeNull();
		expect(store.isProviderReady()).toBe(false);
	});

	it('treats a configured but unusable provider as not ready, keeping the reason', async () => {
		const error = 'no registered provider is named `deepseek`';
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider')
				return {
					family: 'deepseek',
					model: 'deepseek-flash',
					api_key: 'sk',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'deepseek', model: 'deepseek-flash', error };
		});

		const store = await importStore();
		await store.__resetForTests();

		expect(store.isProviderReady()).toBe(false);
		expect(store.getStatus().error).toBe(error);
	});

	it('configures a provider and refreshes state from the backend', async () => {
		let configuredFamily: string | null = null;
		let configureArgs: Record<string, unknown> | null = null;

		mockIPC((cmd, args) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider')
				return configuredFamily
					? {
							family: configuredFamily,
							model: 'gpt-4o-mini',
							api_key: 'sk-new',
							base_url: null,
						}
					: null;
			if (cmd === 'get_provider_status')
				return configuredFamily
					? { family: configuredFamily, model: 'gpt-4o-mini', error: null }
					: { family: null, model: null, error: null };
			if (cmd === 'configure_provider') {
				const a = args as Record<string, unknown>;
				configureArgs = a;
				configuredFamily = a.family as string;
				return null;
			}
		});

		const store = await importStore();
		await store.loadSelections();

		await store.configureProvider({
			family: 'openai/openai',
			apiKey: 'sk-new',
			model: 'gpt-4o-mini',
		});

		expect(configureArgs).toMatchObject({
			family: 'openai/openai',
			apiKey: 'sk-new',
			model: 'gpt-4o-mini',
		});
		expect(store.isProviderReady()).toBe(true);
		expect(store.getCurrentProvider()?.model).toBe('gpt-4o-mini');
	});

	it('disableProvider clears the ready state', async () => {
		let disabled = false;
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'disable_provider') {
				disabled = true;
				return null;
			}
			if (cmd === 'get_current_provider')
				return disabled
					? null
					: {
							family: 'openai/openai',
							model: 'gpt-4o',
							api_key: 'sk',
							base_url: null,
						};
			if (cmd === 'get_provider_status')
				return disabled
					? { family: null, model: null, error: null }
					: { family: 'openai/openai', model: 'gpt-4o', error: null };
		});

		const store = await importStore();
		expect(store.isProviderReady()).toBe(true);

		await store.disableProvider();
		expect(disabled).toBe(true);
		expect(store.isProviderReady()).toBe(false);
	});

	it('exposes a failed state query and clears it after a successful configure', async () => {
		let broken = true;
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider') {
				if (broken) throw new Error('malformed settings file');
				return {
					family: 'openai/openai',
					model: 'gpt-4o-mini',
					api_key: 'sk-new',
					base_url: null,
				};
			}
			if (cmd === 'get_provider_status')
				return { family: 'openai/openai', model: 'gpt-4o-mini', error: null };
			if (cmd === 'configure_provider') return null;
		});

		const store = await importStore();
		await store.__resetForTests();

		expect(store.getProviderStateError()).toContain('malformed settings file');
		expect(store.isLoaded()).toBe(true);

		broken = false;
		await store.configureProvider({
			family: 'openai/openai',
			apiKey: 'sk-new',
			model: 'gpt-4o-mini',
		});

		expect(store.getProviderStateError()).toBeNull();
		expect(store.isProviderReady()).toBe(true);
	});

	it('lists the catalog models of a selection', async () => {
		let listArgs: Record<string, unknown> | null = null;
		mockIPC((cmd, args) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'list_provider_models') {
				listArgs = args as Record<string, unknown>;
				return MODELS[(args as { family: string }).family];
			}
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, error: null };
		});

		const store = await importStore();
		const models = await store.loadModels('ollama/openai');

		expect(models).toEqual([{ id: 'llama3', name: 'Llama 3' }]);
		expect(listArgs).toMatchObject({ family: 'ollama/openai' });
	});

	it('reads the message of a structured command error', async () => {
		mockIPC(() => null);
		const store = await importStore();

		expect(
			store.describeError({
				code: 'external_error',
				message: "provider 'openai/openai' requires an API key",
			}),
		).toBe("provider 'openai/openai' requires an API key");
		expect(store.describeError(new Error('plain'))).toBe('plain');
	});
});
