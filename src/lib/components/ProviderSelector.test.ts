import { describe, it, expect, beforeEach } from 'vitest';
import { render, screen, waitFor, fireEvent } from '@testing-library/svelte';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import ProviderSelector from './ProviderSelector.svelte';
import * as providerStore from '$lib/stores/provider-state.svelte';

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
		id: 'deepseek',
		name: 'DeepSeek',
		requires_api_key: true,
		supports_base_url: false,
		supports_model_listing: false,
	},
];

describe('ProviderSelector', () => {
	beforeEach(async () => {
		clearMocks();
		// Reset the shared store so each test boots against its own mock.
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, ready: false };
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-stored' };
			return null;
		});
		await providerStore.__resetForTests();
	});

	it('renders the family select and preselects the current provider', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai',
					model: 'gpt-4o',
					api_key: 'sk-stored',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'openai', model: 'gpt-4o', ready: true };
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-stored' };
			return null;
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		const select = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		await waitFor(() => expect(select.value).toBe('openai'));
		expect(screen.getByText('OpenAI: gpt-4o')).toBeInTheDocument();

		// Key-required family: API key input prefilled from stored keys; no
		// base URL input.
		expect((screen.getByLabelText('API Key') as HTMLInputElement).value).toBe(
			'sk-stored',
		);
		expect(screen.queryByLabelText('Base URL')).not.toBeInTheDocument();
	});

	it('shows a fetched model dropdown for families with model listing', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, ready: false };
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-stored' };
			if (cmd === 'list_provider_models')
				return [
					{ id: 'gpt-4o', display_name: 'GPT-4o' },
					{ id: 'gpt-4o-mini', display_name: 'GPT-4o mini' },
				];
			return null;
		});

		render(ProviderSelector);

		await waitFor(() => {
			const el = screen.getByLabelText('Model') as HTMLSelectElement;
			expect(el.tagName).toBe('SELECT');
			expect(el.value).toBe('gpt-4o');
		});
		const modelSelect = screen.getByLabelText('Model') as HTMLSelectElement;
		const options = Array.from(modelSelect.options).map((o) => o.value);
		expect(options).toContain('gpt-4o');
		expect(options).toContain('gpt-4o-mini');
	});

	it('falls back to manual model entry when listing fails', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider') return null;
			if (cmd === 'get_provider_status')
				return { family: null, model: null, ready: false };
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-stored' };
			if (cmd === 'list_provider_models') throw new Error('listing failed');
			return null;
		});

		render(ProviderSelector);

		const input = (await screen.findByLabelText('Model')) as HTMLInputElement;
		expect(input.tagName).toBe('INPUT');
		await waitFor(() => {
			expect(
				screen.getByText(/enter the model id manually/i),
			).toBeInTheDocument();
		});
	});

	it('shows a free-text model input for families without model listing', async () => {
		render(ProviderSelector);

		const select = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		await waitFor(() => expect(select.options.length).toBeGreaterThan(0));
		select.value = 'deepseek';
		await fireEvent.change(select);

		const modelInput = (await screen.findByLabelText(
			'Model',
		)) as HTMLInputElement;
		expect(modelInput.tagName).toBe('INPUT');
		// DeepSeek requires an API key; base URL not supported.
		expect(screen.getByLabelText('API Key')).toBeInTheDocument();
		expect(screen.queryByLabelText('Base URL')).not.toBeInTheDocument();
	});

	it('shows a base URL input for families that support one', async () => {
		render(ProviderSelector);

		const select = (await screen.findByLabelText(
			'Provider',
		)) as HTMLSelectElement;
		await waitFor(() => expect(select.options.length).toBeGreaterThan(0));
		select.value = 'ollama';
		await fireEvent.change(select);

		expect(await screen.findByLabelText('Base URL')).toBeInTheDocument();
		expect(screen.queryByLabelText('API Key')).not.toBeInTheDocument();
	});

	it('invokes configure_provider on save', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_stored_api_keys') return { openai: 'sk-stored' };
			if (cmd === 'list_provider_models')
				return [
					{ id: 'gpt-4o', display_name: 'GPT-4o' },
					{ id: 'gpt-4o-mini', display_name: 'GPT-4o mini' },
				];
			return null;
		});

		render(ProviderSelector);

		await waitFor(() => {
			expect((screen.getByLabelText('Model') as HTMLSelectElement).value).toBe(
				'gpt-4o',
			);
		});

		screen.getByRole('button', { name: /save & activate/i }).click();

		await waitFor(() => {
			const call = calls.find((c) => c.cmd === 'configure_provider');
			expect(call?.args).toMatchObject({
				family: 'openai',
				apiKey: 'sk-stored',
				baseUrl: null,
				model: 'gpt-4o',
			});
		});
	});

	it('invokes disable_provider from the current-provider indicator', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'list_provider_families') return FAMILIES;
			if (cmd === 'get_current_provider')
				return {
					family: 'openai',
					model: 'gpt-4o',
					api_key: 'sk',
					base_url: null,
				};
			if (cmd === 'get_provider_status')
				return { family: 'openai', model: 'gpt-4o', ready: true };
			if (cmd === 'get_stored_api_keys') return {};
			return null;
		});
		await providerStore.__resetForTests();

		render(ProviderSelector);

		await screen.findByTestId('current-provider');
		screen.getByRole('button', { name: 'Disable' }).click();

		await waitFor(() => {
			expect(calls.find((c) => c.cmd === 'disable_provider')).toBeTruthy();
		});
	});
});
