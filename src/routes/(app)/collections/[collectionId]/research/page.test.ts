import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest';
import { render, screen } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import { load as rootLoad } from '../../../../+layout';
import { load as researchLoad } from './+page';
import type { PageProps } from './$types';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: 10,
};

const conversations = [{ id: 'c1', title: 'Local history' }];

const provider = {
	current: null,
	status: { family: null, model: null, error: null },
};

let Page: typeof import('./+page.svelte').default;

function mockDefaultIPC() {
	mockIPC((cmd) => {
		if (cmd === 'get_current_provider') return null;
		if (cmd === 'get_provider_status') {
			return { family: null, model: null, error: null };
		}
		if (cmd === 'load_conversation') {
			return {
				id: 'c1',
				collection_id: 'col1',
				title: 'Local history',
				transcript: [
					{
						key: '0-0',
						kind: 'user_text',
						name: '',
						text: 'Earlier question',
						provisional: false,
						is_error: false,
						executing: false,
						streaming: false,
					},
				],
			};
		}
		if (cmd === 'list_providers') return [];
	});
}

beforeAll(async () => {
	mockDefaultIPC();
	Page = (await import('./+page.svelte')).default;
});

beforeEach(() => {
	mockDefaultIPC();
});

function renderPage(data: PageProps['data']) {
	page.data = data;
	render(Page, {
		props: { data, params: { collectionId: data.collection.id } },
	});
}

describe('Research conversations load', () => {
	it('registers its dependency and returns the collection’s conversations', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_conversations') return conversations;
		});
		const depends = vi.fn();

		const data = (await researchLoad({
			params: { collectionId: 'col1' },
			depends,
		} as unknown as Parameters<typeof researchLoad>[0])) as {
			conversations: typeof conversations;
		};

		expect(depends).toHaveBeenCalledWith('app:conversations:col1');
		expect(data.conversations).toEqual(conversations);
	});
});

describe('Pending collection research workspace', () => {
	it('names the collection after its id in the empty state', () => {
		const pending = {
			...collection,
			id: 'pending1',
			name: null,
			document_count: 0,
		};
		renderPage({
			collections: [pending],
			collection: pending,
			conversations: [],
			provider,
			providerError: null,
		});

		expect(
			screen.getByText('No documents in Shared collection pending1'),
		).toBeInTheDocument();
	});
});

describe('Research workspace without a chat provider', () => {
	it('keeps the local conversation sidebar available beside the Settings link', async () => {
		renderPage({
			collections: [collection],
			collection,
			conversations,
			provider,
			providerError: null,
		});

		expect(screen.getByText('Local history')).toBeInTheDocument();
		expect(
			screen.getByRole('button', { name: 'New Chat' }),
		).toBeInTheDocument();
		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			await screen.findByRole('link', { name: 'Open Settings' }),
		).toHaveAttribute('href', '/settings');
		expect(
			screen.queryByPlaceholderText('Ask about your documents...'),
		).not.toBeInTheDocument();
	});

	it('shows the Settings link when settings cannot be read', async () => {
		const reason = 'malformed settings file';
		mockIPC((cmd) => {
			if (cmd === 'get_current_provider')
				throw { code: 'storage_error', message: reason };
			if (cmd === 'get_provider_status')
				return { family: null, model: null, error: null };
			if (cmd === 'get_collections') return [collection];
			if (cmd === 'get_documents') return [];
			if (cmd === 'get_pipeline_progress') return [];
			if (cmd === 'list_providers') return [];
		});
		const depends = vi.fn();
		const data = (await rootLoad({
			depends,
		} as unknown as Parameters<typeof rootLoad>[0])) as PageProps['data'];
		renderPage({
			...data,
			collection,
			conversations: [],
		});

		expect(
			await screen.findByRole('link', { name: 'Open Settings' }),
		).toHaveAttribute('href', '/settings');
		expect(
			screen.queryByPlaceholderText('Ask about your documents...'),
		).not.toBeInTheDocument();
	});
});
