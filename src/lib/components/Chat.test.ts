import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest';
import { render, screen, cleanup } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import Chat from './Chat.svelte';
import * as chat from '#lib/stores/conversations.svelte.js';

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async () => () => {}),
}));

const props = {
	collectionId: 'col1',
	collectionName: 'Climate Research',
	documentCount: 2,
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
		if (cmd === 'list_conversations') return opts.conversations ?? [];
		if (cmd === 'load_conversation') {
			return {
				id: 'c1',
				collection_id: collectionId,
				title: 'Existing chat',
				messages: opts.messages ?? [],
			};
		}
	});

	await chat.initialize(collectionId);
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

	it('does not request a next-message prediction without documents', async () => {
		const calls: string[] = [];
		mockIPC((cmd) => {
			calls.push(cmd);
			if (cmd === 'list_conversations') {
				return [{ id: 'c1', title: 'Existing chat' }];
			}
			if (cmd === 'load_conversation') {
				return {
					id: 'c1',
					collection_id: 'no-prediction',
					title: 'Existing chat',
					messages: [
						{
							role: 'user',
							content: [{ type: 'text', text: 'Earlier question' }],
						},
					],
				};
			}
		});

		await chat.initialize('no-prediction');
		render(Chat, { props: { ...props, documentCount: 0 } });

		await screen.findByText('Earlier question');
		await new Promise((resolve) => setTimeout(resolve, 600));
		expect(calls).not.toContain('predict_next_message');
	});
});
