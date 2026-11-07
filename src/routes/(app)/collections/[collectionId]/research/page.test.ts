import { describe, it, expect, beforeAll, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

let Page: typeof import('./+page.svelte').default;

beforeAll(async () => {
	mockIPC((cmd) => {
		if (cmd === 'get_current_provider') return null;
		if (cmd === 'get_provider_status') {
			return { family: null, model: null, error: null };
		}
		if (cmd === 'list_conversations') {
			return [{ id: 'c1', title: 'Local history' }];
		}
		if (cmd === 'load_conversation') {
			return {
				id: 'c1',
				collection_id: 'col1',
				title: 'Local history',
				messages: [
					{
						role: 'user',
						content: [{ type: 'text', text: 'Earlier question' }],
					},
				],
			};
		}
		if (cmd === 'list_providers') return [];
	});
	page.url = new URL('http://localhost/collections/col1/research');
	page.params = { collectionId: 'col1' };
	page.data = { collection };
	Page = (await import('./+page.svelte')).default;
});

describe('Research workspace without a chat provider', () => {
	it('keeps the local conversation sidebar available beside provider setup', async () => {
		render(Page);

		expect(await screen.findByText('Local history')).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: 'New Chat' }),
		).toBeInTheDocument();
		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			await screen.findByRole('heading', {
				name: 'Set up your language model',
			}),
		).toBeInTheDocument();
		expect(
			screen.queryByPlaceholderText('Ask about your documents...'),
		).not.toBeInTheDocument();
	});
});
