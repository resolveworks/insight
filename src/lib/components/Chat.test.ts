import { describe, it, expect, afterEach, beforeEach } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import Chat from './Chat.svelte';
import * as chat from '#lib/stores/conversations.svelte.js';

const props = {
	collectionId: 'col1',
	collectionName: 'Climate Research',
	documentCount: 2,
	providerReady: true,
};

type RenderOptions = {
	conversations?: Array<{ id: string; title: string }>;
	messages?: Array<{
		role: 'user' | 'assistant';
		content: Array<{ type: 'text'; text: string }>;
	}>;
};

beforeEach(() => {
	localStorage.clear();
});

afterEach(() => {
	cleanup();
});

async function renderChat(
	collectionId: string,
	overrides: Partial<typeof props> = {},
	opts: RenderOptions = {},
) {
	mockIPC((cmd) => {
		if (cmd === 'load_conversation') {
			return {
				id: 'c1',
				collection_id: collectionId,
				title: 'Existing chat',
				messages: opts.messages ?? [],
			};
		}
		if (cmd === 'list_providers') return [];
	});

	await chat.initialize(collectionId, opts.conversations ?? []);
	page.data = {
		provider: {
			current: null,
			status: { family: null, model: null, error: null },
		},
	};
	render(Chat, { props: { ...props, ...overrides } });
}

describe('Chat', () => {
	it('names the collection and its documents in the empty state', async () => {
		await renderChat('empty-state');

		expect(
			screen.getByText('Ask questions about Climate Research'),
		).toBeInTheDocument();
		expect(
			screen.getByText('2 documents ready to search and read'),
		).toBeInTheDocument();
	});

	it('directs an empty collection to Documents instead of inviting questions', async () => {
		await renderChat('no-docs', { documentCount: 0 });

		expect(
			screen.getByText('No documents in Climate Research'),
		).toBeInTheDocument();
		expect(
			screen.queryByText('Ask questions about Climate Research'),
		).not.toBeInTheDocument();
		const link = screen.getByRole('link', { name: 'Go to Documents' });
		expect(link).toHaveAttribute('href', '/collections/col1/documents');
		expect(
			screen.getByPlaceholderText('Add documents to start asking questions'),
		).toBeDisabled();
		expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
	});

	it('keeps stored messages and disables the composer in an empty collection', async () => {
		await renderChat(
			'no-docs-with-history',
			{ documentCount: 0 },
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				messages: [
					{
						role: 'user',
						content: [{ type: 'text', text: 'Earlier question' }],
					},
				],
			},
		);

		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			screen.queryByText('No documents in Climate Research'),
		).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
	});

	it('shows stored message bodies with provider setup below and no input', async () => {
		await renderChat(
			'no-provider',
			{ providerReady: false },
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				messages: [
					{
						role: 'user',
						content: [{ type: 'text', text: 'Earlier question' }],
					},
				],
			},
		);

		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			screen.getByRole('heading', { name: 'Set up your language model' }),
		).toBeInTheDocument();
		expect(
			screen.queryByPlaceholderText('Ask about your documents...'),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'Send' }),
		).not.toBeInTheDocument();
	});

	it('renders reasoning and tool activity as open disclosures, flagging a failed result', async () => {
		mockIPC((cmd) => {
			if (cmd === 'list_providers') return [];
			if (cmd === 'load_conversation') {
				return {
					id: 'c1',
					collection_id: 'tools',
					title: 'Existing chat',
					messages: [
						{
							role: 'user',
							content: [{ type: 'text', text: 'Question' }],
						},
						{
							role: 'assistant',
							content: [
								{ type: 'reasoning', text: 'Looking around' },
								{
									type: 'tool_call',
									id: { provider: 'call_1' },
									function: {
										name: 'search',
										arguments: { query: 'x' },
									},
								},
							],
						},
						{
							role: 'user',
							content: [
								{
									type: 'tool_result',
									call: { provider: 'call_1' },
									name: 'search',
									content: [{ type: 'text', text: 'index unavailable' }],
									is_error: true,
								},
							],
						},
					],
				};
			}
		});

		await chat.initialize('tools', [{ id: 'c1', title: 'Existing chat' }]);
		render(Chat, { props: { ...props, collectionId: 'tools' } });

		const titles = await Promise.all([
			screen.findByText('Reasoning'),
			screen.findByText('Tool: search'),
			screen.findByText('Tool result: search'),
		]);
		for (const title of titles) {
			expect(title.closest('details')).toHaveAttribute('open');
		}
		expect(screen.getByText('index unavailable')).toBeInTheDocument();
		expect(titles[2].closest('details')?.className).toContain(
			'border-error/50',
		);
	});
});
