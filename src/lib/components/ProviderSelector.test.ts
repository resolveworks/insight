import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import ProviderSelector from './ProviderSelector.svelte';
import * as providerStore from '#lib/stores/provider-state.svelte.js';

const SELECTIONS = ['ollama/openai', 'openai/openai'];

const MODELS: Record<string, Array<{ id: string; name: string }>> = {
	'openai/openai': [
		{ id: 'gpt-4o', name: 'GPT-4o' },
		{ id: 'gpt-4o-mini', name: 'GPT-4o mini' },
	],
	'ollama/openai': [{ id: 'llama3', name: 'Llama 3' }],
};

const idleStatus = { family: null, model: null, error: null };

describe('ProviderSelector', () => {
	beforeEach(async () => {
		clearMocks();
		// Reset the shared store so each test boots against its own mock.
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status') return idleStatus;
			return null;
		});
		await providerStore.__resetForTests();
	});

	it('offers rig selections and preselects the configured provider and model', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai/openai',
					model: 'gpt-4o',
					api_key: 'sk-stored',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'openai/openai', model: 'gpt-4o', error: null };
			if (cmd === 'list_provider_models') return MODELS['openai/openai'];
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const provider = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		await waitFor(() => expect(provider.value).toBe('openai/openai'));
		expect(Array.from(provider.options).map((o) => o.value)).toEqual(
			SELECTIONS,
		);

		const model = screen.getByLabelText('Model') as HTMLSelectElement;
		await waitFor(() => expect(model.value).toBe('gpt-4o'));
		expect(
			screen.getByRole('option', { name: 'GPT-4o mini' }),
		).toBeInTheDocument();

		expect((screen.getByLabelText('API Key') as HTMLInputElement).value).toBe(
			'sk-stored',
		);
		expect(screen.getByLabelText('Base URL')).toBeInTheDocument();
	});

	it('shows the provider state error when the boot query fails', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider') throw new Error('malformed settings');
			if (cmd === 'get_provider_status') return idleStatus;
			if (cmd === 'list_provider_models') return MODELS['ollama/openai'];
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		expect(
			await screen.findByText(/failed to load provider settings/i),
		).toHaveTextContent('malformed settings');
		// The form stays visible so the settings can be inspected and fixed.
		expect(screen.getByLabelText('Provider')).toBeInTheDocument();
	});

	it('says why a configured provider cannot be used', async () => {
		const reason = 'no registered provider is named `deepseek`';
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
				return { family: 'deepseek', model: 'deepseek-flash', error: reason };
			if (cmd === 'list_provider_models') return [];
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		expect(await screen.findByText(reason)).toBeInTheDocument();
	});

	it('swaps the model list when the provider changes', async () => {
		mockIPC((cmd, args) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status') return idleStatus;
			if (cmd === 'list_provider_models')
				return MODELS[(args as { family: string }).family];
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const provider = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		await waitFor(() => expect(provider.value).toBe('ollama/openai'));
		await waitFor(() =>
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'llama3',
			),
		);

		await fireEvent.change(provider, { target: { value: 'openai/openai' } });

		await waitFor(() =>
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'gpt-4o',
			),
		);
	});

	it('keeps a configured model the catalog does not list', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai/openai',
					model: 'my-private-deployment',
					api_key: 'sk',
					base_url: 'https://gateway.internal/v1',
				};
			if (cmd === 'get_provider_status')
				return {
					family: 'openai/openai',
					model: 'my-private-deployment',
					error: null,
				};
			if (cmd === 'list_provider_models') return MODELS['openai/openai'];
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const model = (await screen.findByLabelText('Model')) as HTMLSelectElement;
		await waitFor(() => expect(model.value).toBe('my-private-deployment'));
		expect(
			screen.getByRole('option', { name: 'my-private-deployment' }),
		).toBeInTheDocument();
	});

	it('invokes configure_provider on save', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'list_provider_models')
				return MODELS[(args as { family: string }).family];
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status') return idleStatus;
			return null;
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const provider = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		// Let the initial selection settle before changing it.
		await waitFor(() =>
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'llama3',
			),
		);
		await fireEvent.change(provider, { target: { value: 'openai/openai' } });
		await fireEvent.input(screen.getByLabelText('API Key'), {
			target: { value: ' sk-typed ' },
		});

		await waitFor(() => {
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'gpt-4o',
			);
		});
		screen.getByRole('button', { name: 'Save' }).click();

		await waitFor(() => {
			const call = calls.find((c) => c.cmd === 'configure_provider');
			expect(call?.args).toMatchObject({
				family: 'openai/openai',
				apiKey: 'sk-typed',
				baseUrl: null,
				model: 'gpt-4o',
			});
		});
	});

	it('shows the message of a rejected configure, not the error object', async () => {
		const reason = "provider 'openai/openai' requires an API key";
		mockIPC((cmd, args) => {
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'list_provider_models')
				return MODELS[(args as { family: string }).family];
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status') return idleStatus;
			if (cmd === 'configure_provider')
				throw { code: 'external_error', message: reason };
			return null;
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const provider = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		// Let the initial selection settle before changing it.
		await waitFor(() =>
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'llama3',
			),
		);
		await fireEvent.change(provider, { target: { value: 'openai/openai' } });
		await waitFor(() =>
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'gpt-4o',
			),
		);

		screen.getByRole('button', { name: 'Save' }).click();

		expect(await screen.findByText(reason)).toBeInTheDocument();
		expect(screen.queryByText(/\[object Object\]/)).not.toBeInTheDocument();
	});

	it('invokes disable_provider for a configured provider', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		let disabled = false;
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_providers') return SELECTIONS;
			if (cmd === 'disable_provider') {
				disabled = true;
				return null;
			}
			if (cmd === 'list_provider_models') return MODELS['openai/openai'];
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
					? idleStatus
					: { family: 'openai/openai', model: 'gpt-4o', error: null };
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		screen.getByRole('button', { name: 'Disable' }).click();

		await waitFor(() => {
			expect(calls.find((c) => c.cmd === 'disable_provider')).toBeTruthy();
		});
	});
});
