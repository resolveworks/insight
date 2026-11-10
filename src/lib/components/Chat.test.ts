import { describe, it, expect, afterEach, beforeEach } from 'vitest';
import { render, screen, cleanup, fireEvent } from '@testing-library/svelte';
import { mockIPC } from '@tauri-apps/api/mocks';
import { page } from '$app/state';
import Chat from './Chat.svelte';
import * as chat from '#lib/stores/conversations.svelte.js';
import type {
	ConversationSummary,
	TranscriptItem,
	TranscriptKind,
} from '#lib/types/chat.js';

const collection = {
	id: 'col1',
	name: 'Climate Research',
	document_count: 2,
	total_pages: null,
};

const props = {
	collection,
	providerReady: true,
};

type RenderOptions = {
	conversations?: ConversationSummary[];
	transcript?: TranscriptItem[];
};

function item(
	key: string,
	kind: TranscriptKind,
	fields: Partial<TranscriptItem> = {},
): TranscriptItem {
	return {
		key,
		kind,
		name: '',
		text: '',
		provisional: false,
		is_error: false,
		executing: false,
		streaming: false,
		...fields,
	};
}

const userItem = (text: string) => item('0-0', 'user_text', { text });

const toolCall = (key: string, name: string, args: unknown): TranscriptItem =>
	item(key, 'tool_call', {
		name,
		text: JSON.stringify(args, null, 2),
		executing: true,
	});

const toolResult = (
	key: string,
	name: string,
	result: unknown,
	fields: Partial<TranscriptItem> = {},
): TranscriptItem =>
	item(key, 'tool_result', {
		name,
		text: typeof result === 'string' ? result : JSON.stringify(result),
		...fields,
	});

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
				transcript: opts.transcript ?? [],
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
		await renderChat('no-docs', {
			collection: { ...collection, document_count: 0 },
		});

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

	it('keeps a stored transcript and disables the composer in an empty collection', async () => {
		await renderChat(
			'no-docs-with-history',
			{ collection: { ...collection, document_count: 0 } },
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [userItem('Earlier question')],
			},
		);

		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			screen.queryByText('No documents in Climate Research'),
		).not.toBeInTheDocument();
		expect(screen.getByRole('button', { name: 'Send' })).toBeDisabled();
	});

	it('shows a stored transcript with a link to Settings and no input', async () => {
		await renderChat(
			'no-provider',
			{ providerReady: false },
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [userItem('Earlier question')],
			},
		);

		expect(await screen.findByText('Earlier question')).toBeInTheDocument();
		expect(
			screen.getByText('No language model configured. Research needs one.'),
		).toBeInTheDocument();
		expect(screen.getByRole('link', { name: 'Open Settings' })).toHaveAttribute(
			'href',
			'/settings',
		);
		expect(
			screen.queryByPlaceholderText('Ask about your documents...'),
		).not.toBeInTheDocument();
		expect(
			screen.queryByRole('button', { name: 'Send' }),
		).not.toBeInTheDocument();
	});

	it('renders an answer as prose and the question as a message', async () => {
		await renderChat(
			'answer',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					userItem('What changed in 2024?'),
					item('1-0', 'assistant_text', {
						text: 'Emissions fell by 4%.',
					}),
				],
			},
		);

		expect(
			await screen.findByText('What changed in 2024?'),
		).toBeInTheDocument();
		expect(screen.getByText('Emissions fell by 4%.')).toBeInTheDocument();
	});

	it('merges a tool call and its result into one collapsed step', async () => {
		await renderChat(
			'paired',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					userItem('Question'),
					toolCall('1-0', 'search', { query: 'climate budget' }),
					toolResult('2-0', 'search', {
						total_hits: 2,
						hits: [{ passage: 'unique passage content' }, { passage: 'other' }],
					}),
				],
			},
		);

		const label = await screen.findByText('Search “climate budget”');
		const details = label.closest('details');
		expect(details).not.toHaveAttribute('open');
		expect(details?.querySelector('summary')).toHaveTextContent(
			'Search “climate budget”',
		);
		expect(screen.getByText('2 passages')).toBeInTheDocument();
		expect(screen.queryByText(/^Tool:/)).not.toBeInTheDocument();
		expect(screen.queryByText(/^Tool result:/)).not.toBeInTheDocument();
		expect(screen.getByText(/unique passage content/)).not.toBeVisible();

		await fireEvent.click(label);
		expect(details).toHaveAttribute('open');
		expect(screen.getByText(/unique passage content/)).toBeVisible();
		expect(screen.getByText(/query:/)).toBeVisible();
	});

	it('shows a running step while its result is pending', async () => {
		await renderChat(
			'running',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [toolCall('1-0', 'search', { query: 'climate' })],
			},
		);

		expect(await screen.findByText('Search “climate”')).toBeInTheDocument();
		expect(screen.getByRole('status', { name: 'Running' })).toBeVisible();
		expect(screen.queryByText(/passages?$/)).not.toBeInTheDocument();
	});

	it('opens a failed step with its message and no result hint', async () => {
		await renderChat(
			'failed',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					toolCall('1-0', 'search', { query: 'climate' }),
					toolResult(
						'2-0',
						'search',
						{ hits: [{}] },
						{ is_error: true, text: 'index unavailable' },
					),
				],
			},
		);

		const message = await screen.findByText('index unavailable');
		expect(message).toBeVisible();
		expect(message.closest('details')).toHaveAttribute('open');
		expect(screen.getByRole('img', { name: 'Failed' })).toBeVisible();
		expect(screen.queryByText('1 passage')).not.toBeInTheDocument();
		expect(
			screen.queryByRole('status', { name: 'Running' }),
		).not.toBeInTheDocument();
	});

	it('labels a passage read by its chunk index', async () => {
		await renderChat(
			'read-chunk',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					toolCall('1-0', 'read_chunk', {
						document_id: 'doc-1',
						chunk_index: 3,
					}),
					toolResult('2-0', 'read_chunk', {
						document_id: 'doc-1',
						chunk_index: 3,
						content: 'passage text',
					}),
				],
			},
		);

		const label = await screen.findByText('Read passage 3');
		expect(label.closest('details')).not.toHaveAttribute('open');
	});

	it('renders an unknown tool from its name and parsed arguments', async () => {
		await renderChat(
			'unknown-tool',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					toolCall('1-0', 'web_fetch', {
						url: 'https://example.com',
						depth: 2,
					}),
				],
			},
		);

		expect(
			await screen.findByText('Web fetch: url: https://example.com, depth: 2'),
		).toBeInTheDocument();
	});

	it('renders streaming reasoning as just Thought with a spinner', async () => {
		await renderChat(
			'reasoning-streaming',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					item('1-0', 'reasoning', {
						text: 'Looking',
						streaming: true,
					}),
				],
			},
		);

		const label = await screen.findByText('Thought');
		expect(label).toBeVisible();
		expect(label.closest('summary')).toHaveTextContent(/^Thought$/);
		expect(screen.queryByText('Reasoning')).not.toBeInTheDocument();
		expect(screen.getByRole('status', { name: 'Thinking' })).toBeVisible();
		expect(label.closest('details')).not.toHaveAttribute('open');
	});

	it('previews completed reasoning on one line', async () => {
		await renderChat(
			'reasoning-complete',
			{},
			{
				conversations: [{ id: 'c1', title: 'Existing chat' }],
				transcript: [
					item('1-0', 'reasoning', {
						text: 'Looking\naround for clues',
					}),
				],
			},
		);

		const label = await screen.findByText('Thought');
		expect(label.closest('summary')).toHaveTextContent(
			'Thought Looking around for clues',
		);
		expect(
			screen.queryByRole('status', { name: 'Thinking' }),
		).not.toBeInTheDocument();
		expect(label.closest('details')).not.toHaveAttribute('open');
	});
});
