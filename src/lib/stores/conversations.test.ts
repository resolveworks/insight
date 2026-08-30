import { describe, it, expect, beforeEach, vi } from 'vitest';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import type { MultiTurnStreamItem, RigMessage } from '$lib/types/rig';
import { emptyLiveRun } from '$lib/chat/transcript';

type EventListener = (event: { payload: unknown }) => void;
const listeners = new Map<string, EventListener>();

vi.mock('@tauri-apps/api/event', () => ({
	listen: vi.fn(async (name: string, cb: EventListener) => {
		listeners.set(name, cb);
		return () => listeners.delete(name);
	}),
}));

/**
 * The conversations store keeps module-level $state, so each test re-imports
 * a fresh module copy with the IPC mock already installed.
 */
async function importStore() {
	vi.resetModules();
	listeners.clear();
	return await import('./conversations.svelte');
}

function emit(conversationId: string, payload: MultiTurnStreamItem) {
	listeners.get(`agent-event-${conversationId}`)?.({ payload });
}

const USAGE = {
	input_tokens: 0,
	output_tokens: 0,
	total_tokens: 0,
	cached_input_tokens: 0,
	cache_creation_input_tokens: 0,
	reasoning_tokens: 0,
};

function conversationView(
	id: string,
	opts: { messages?: unknown[]; collections?: unknown[] } = {},
) {
	return {
		id,
		title: 'Test chat',
		created_at: '2024-01-01T00:00:00Z',
		updated_at: '2024-01-01T00:00:00Z',
		collections: opts.collections ?? [],
		messages: opts.messages ?? [],
	};
}

describe('conversations store event handling', () => {
	beforeEach(() => {
		clearMocks();
	});

	it('builds a chronological transcript from a streamed multi-turn sequence', async () => {
		const finalMessages: RigMessage[] = [
			{ role: 'user', content: [{ type: 'text', text: 'find papers' }] },
			{
				role: 'assistant',
				id: null,
				content: [
					{
						type: 'reasoning',
						id: null,
						content: [{ type: 'text', content: { text: 'Thinking' } }],
					},
					{ type: 'text', text: 'Let me search the index.' },
					{
						type: 'tool_call',
						id: 'call1',
						function: { name: 'search', arguments: { q: 'papers' } },
					},
				],
			},
			{
				role: 'user',
				content: [
					{
						type: 'tool_result',
						call: 'call1',
						name: 'search',
						content: [{ type: 'text', text: '3 results' }],
					},
				],
			},
			{
				role: 'assistant',
				id: null,
				content: [{ type: 'text', text: 'Found 3 results.' }],
			},
		];
		const views: Record<string, unknown> = {
			// First load (start_chat): empty history
			empty: conversationView('conv1'),
			// Authoritative refresh after finalResponse
			final: conversationView('conv1', { messages: finalMessages }),
		};
		let currentView = views.empty;
		const loadCalls: string[] = [];

		mockIPC((cmd) => {
			if (cmd === 'list_conversations')
				return [
					{
						id: 'conv1',
						title: 'Test chat',
						updated_at: '2024-01-01T00:00:00Z',
					},
				];
			if (cmd === 'start_chat') return currentView;
			if (cmd === 'load_conversation') {
				loadCalls.push('load');
				return currentView;
			}
			if (cmd === 'send_message') return null;
		});

		const store = await importStore();
		await store.ensureInitialized();
		expect(store.getActiveId()).toBe('conv1');
		loadCalls.length = 0;

		await store.sendMessage('find papers');
		expect(store.getIsGenerating()).toBe(true);
		expect(store.getActiveMessages()).toEqual([
			{ role: 'user', content: [{ type: 'text', text: 'find papers' }] },
		]);

		// Stream: reasoning delta, text deltas, tool-call deltas, complete call,
		// committed execution, tool result, usage record.
		emit('conv1', {
			type: 'streamAssistantItem',
			id: 'r1',
			reasoning: 'Thinking',
		});
		emit('conv1', { type: 'streamAssistantItem', text: 'Let me search' });
		emit('conv1', { type: 'streamAssistantItem', text: ' the index.' });

		const items = store.getTranscript().map((item) => item.kind);
		expect(items).toEqual(['userText', 'reasoning', 'assistantText']);
		const liveItems = store.getTranscript().slice(1);
		expect(liveItems.every((item) => item.provisional)).toBe(true);
		expect(
			store.getTranscript().find((item) => item.kind === 'assistantText'),
		).toMatchObject({ text: 'Let me search the index.', streaming: true });

		emit('conv1', {
			type: 'streamAssistantItem',
			internal_call_id: 'ic1',
			content: { Name: 'search' },
		});
		emit('conv1', {
			type: 'streamAssistantItem',
			internal_call_id: 'ic1',
			content: { Delta: '{"q": "pap' },
		});
		emit('conv1', {
			type: 'streamAssistantItem',
			internal_call_id: 'ic1',
			content: { Delta: 'ers"}' },
		});

		emit('conv1', {
			type: 'streamAssistantItem',
			tool_call: {
				id: 'call1',
				function: { name: 'search', arguments: { q: 'papers' } },
			},
			internal_call_id: 'ic1',
		});

		const callItem = () =>
			store.getTranscript().find((item) => item.kind === 'toolCall')!;
		const prettyArgs = JSON.stringify({ q: 'papers' }, null, 2);
		expect(callItem()).toMatchObject({
			name: 'search',
			args: prettyArgs,
			executing: true,
		});

		emit('conv1', {
			type: 'toolExecutionCommitted',
			tool_call: { id: 'call1', function: { name: 'search', arguments: {} } },
			internal_call_id: 'ic1',
		});
		expect(callItem().executing).toBe(false);

		emit('conv1', {
			type: 'streamUserItem',
			tool_result: {
				call: 'call1',
				name: 'search',
				content: [{ type: 'text', text: '3 results' }],
			},
			internal_call_id: 'ic1',
		});
		emit('conv1', { type: 'completionCall', call_index: 0, usage: USAGE });

		expect(store.getTranscript().map((item) => item.kind)).toEqual([
			'userText',
			'reasoning',
			'assistantText',
			'toolCall',
			'toolResult',
		]);
		expect(
			store.getTranscript().find((item) => item.kind === 'toolResult'),
		).toMatchObject({ text: '3 results' });
		expect(store.getIsGenerating()).toBe(true);

		// Keys of the live tail — committing the run must keep every one of
		// them (same keys = same DOM nodes = nothing remounts or collapses).
		const liveKeys = store.getTranscript().map((item) => item.key);

		// Terminal: adopt the event's authoritative transcript synchronously.
		currentView = views.final;
		emit('conv1', {
			type: 'finalResponse',
			output: 'Found 3 results.',
			usage: USAGE,
			messages: finalMessages,
			content: [{ type: 'text', text: 'Found 3 results.' }],
		});

		// No await: the swap must already be rendered.
		expect(store.getIsGenerating()).toBe(false);
		expect(store.getActiveMessages()).toHaveLength(4);
		const adopted = store.getTranscript();
		expect(adopted.map((item) => item.kind)).toEqual([
			'userText',
			'reasoning',
			'assistantText',
			'toolCall',
			'toolResult',
			'assistantText',
		]);
		expect(adopted.every((item) => !item.provisional)).toBe(true);
		// The live blocks keep their exact keys; only the final answer is new.
		expect(adopted.map((item) => item.key).slice(0, -1)).toEqual(liveKeys);

		await vi.waitFor(() => {
			expect(loadCalls).toContain('load');
		});
		expect(store.getLiveState()).toEqual(emptyLiveRun());
		expect(store.getActiveMessages()[3]?.content[0]).toEqual({
			type: 'text',
			text: 'Found 3 results.',
		});
	});

	it('replaces accumulated reasoning when the complete block arrives', async () => {
		const store = await importStore();
		store.applyStreamItem({
			type: 'streamAssistantItem',
			id: 'r1',
			reasoning: 'partial',
		});
		store.applyStreamItem({
			type: 'streamAssistantItem',
			id: 'r1',
			reasoning: ' more',
		});
		store.applyStreamItem({
			type: 'streamAssistantItem',
			reasoning: {
				id: null,
				content: [{ type: 'text', content: { text: 'final text' } }],
			},
			id: 'r1',
		});
		const reasoning = store
			.getTranscript()
			.find((item) => item.kind === 'reasoning');
		expect(reasoning?.text).toBe('final text');
	});

	it('discards provisional live output on modelTurnRetried', async () => {
		const store = await importStore();
		store.applyStreamItem({ type: 'streamAssistantItem', text: 'draft' });
		store.applyStreamItem({
			type: 'streamAssistantItem',
			id: 'r1',
			reasoning: 'thinking',
		});
		store.applyStreamItem({ type: 'modelTurnRetried', turn: 1 });
		expect(store.getTranscript()).toEqual([]);
		expect(store.getLiveState()).toEqual(emptyLiveRun());
	});

	it('ignores the provider terminal record (StreamFinal) and unknown payloads', async () => {
		const store = await importStore();
		store.applyStreamItem({
			type: 'streamAssistantItem',
			kind: 'final',
			usage: USAGE,
			finish_reason: null,
			message_id: null,
			response_id: null,
			provider: 'openai',
		});
		store.applyStreamItem({
			type: 'streamAssistantItem',
			web_search_call: { id: 'x' },
		});
		expect(store.getTranscript()).toEqual([]);
	});

	it('surfaces stream errors and refreshes history', async () => {
		const currentView = conversationView('conv2');
		const loadCalls: string[] = [];
		mockIPC((cmd) => {
			if (cmd === 'list_conversations') return [];
			if (cmd === 'start_chat') return currentView;
			if (cmd === 'load_conversation') {
				loadCalls.push('load');
				return currentView;
			}
			if (cmd === 'send_message') return null;
		});

		const store = await importStore();
		store.newConversation();
		await store.addActiveCollection({
			id: 'col1',
			name: 'Test',
			document_count: 0,
			total_pages: 0,
		});
		await store.sendMessage('hello');
		emit('conv2', { type: 'streamAssistantItem', text: 'partial' });
		emit('conv2', { type: 'streamError', message: 'provider exploded' });

		await vi.waitFor(() => {
			expect(store.getError()).toBe('provider exploded');
		});
		expect(store.getIsGenerating()).toBe(false);
		await vi.waitFor(() => {
			expect(store.getTranscript()).toEqual([]);
		});
		expect(loadCalls).toContain('load');
		store.clearError();
		expect(store.getError()).toBeNull();
	});

	it('switches collection scope via set_conversation_collections', async () => {
		let scope: unknown[] | null = null;
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations')
				return [
					{
						id: 'conv3',
						title: 'Test chat',
						updated_at: '2024-01-01T00:00:00Z',
					},
				];
			if (cmd === 'load_conversation') return conversationView('conv3');
			if (cmd === 'set_conversation_collections') {
				scope = (args as { collections: unknown[] }).collections;
				return conversationView('conv3', {
					collections: scope.map((c) => ({
						...(c as object),
						document_count: 5,
						total_pages: 100,
					})),
				});
			}
		});

		const store = await importStore();
		await store.ensureInitialized();
		expect(store.getActiveId()).toBe('conv3');

		await store.addActiveCollection({
			id: 'col1',
			name: 'Climate',
			document_count: 0,
			total_pages: 0,
		});

		expect(scope).toEqual([
			{ id: 'col1', name: 'Climate', document_count: 0, total_pages: 0 },
		]);
		// The server-enriched counts are adopted.
		expect(store.getActiveCollections()).toEqual([
			{ id: 'col1', name: 'Climate', document_count: 5, total_pages: 100 },
		]);

		await store.removeActiveCollection('col1');
		expect(scope).toEqual([]);
		expect(store.getActiveCollections()).toEqual([]);
	});

	it('deletes the active conversation and lands in a draft when none remain', async () => {
		const deleted: string[] = [];
		const commands: string[] = [];
		mockIPC((cmd, args) => {
			commands.push(cmd);
			if (cmd === 'list_conversations')
				return [
					{ id: 'c1', title: 'Only chat', updated_at: '2024-01-01T00:00:00Z' },
				];
			if (cmd === 'load_conversation') return conversationView('c1');
			if (cmd === 'delete_conversation') {
				deleted.push((args as { conversationId: string }).conversationId);
				return null;
			}
		});

		const store = await importStore();
		await store.ensureInitialized();
		expect(store.getActiveId()).toBe('c1');

		commands.length = 0;
		await store.deleteConversation('c1');
		expect(deleted).toEqual(['c1']);
		// No auto-create: the user lands in the draft state.
		expect(store.getActiveId()).toBeNull();
		expect(store.getActiveMessages()).toEqual([]);
		expect(commands).not.toContain('start_chat');
	});

	describe('draft (lazy) conversations', () => {
		beforeEach(() => {
			clearMocks();
		});

		it('ensureInitialized lands in a draft when no conversations exist', async () => {
			const commands: string[] = [];
			mockIPC((cmd) => {
				commands.push(cmd);
				if (cmd === 'list_conversations') return [];
			});

			const store = await importStore();
			await store.ensureInitialized();

			expect(store.getIsInitialized()).toBe(true);
			expect(store.getActiveId()).toBeNull();
			expect(commands).not.toContain('start_chat');
		});

		it('newConversation makes no IPC call and resets to draft', async () => {
			const commands: string[] = [];
			mockIPC((cmd) => {
				commands.push(cmd);
				if (cmd === 'list_conversations')
					return [
						{
							id: 'c1',
							title: 'Test chat',
							updated_at: '2024-01-01T00:00:00Z',
						},
					];
				if (cmd === 'load_conversation')
					return conversationView('c1', {
						messages: [
							{ role: 'user', content: [{ type: 'text', text: 'hi' }] },
						],
					});
			});

			const store = await importStore();
			await store.ensureInitialized();
			expect(store.getActiveId()).toBe('c1');
			commands.length = 0;

			store.newConversation();
			expect(store.getActiveId()).toBeNull();
			expect(store.getActiveMessages()).toEqual([]);
			expect(store.getActiveCollections()).toEqual([]);
			expect(commands).toEqual([]);
		});

		it('addActiveCollection in a draft updates local scope without IPC', async () => {
			const commands: string[] = [];
			mockIPC((cmd) => {
				commands.push(cmd);
				if (cmd === 'list_conversations') return [];
			});

			const store = await importStore();
			await store.ensureInitialized();
			const col = {
				id: 'col1',
				name: 'Climate',
				document_count: 0,
				total_pages: 0,
			};
			await store.addActiveCollection(col);

			expect(store.getActiveId()).toBeNull();
			expect(store.getActiveCollections()).toEqual([col]);
			expect(commands).toEqual(['list_conversations']);

			await store.removeActiveCollection('col1');
			expect(store.getActiveCollections()).toEqual([]);
			expect(commands).toEqual(['list_conversations']);
		});

		it('sendMessage from a draft creates the conversation with the draft scope, then sends', async () => {
			const commands: Array<{ cmd: string; args: Record<string, unknown> }> =
				[];
			mockIPC((cmd, args) => {
				commands.push({ cmd, args: args as Record<string, unknown> });
				if (cmd === 'list_conversations') return [];
				if (cmd === 'start_chat')
					return conversationView('new1', {
						collections: [
							{
								id: 'col1',
								name: 'Climate',
								document_count: 2,
								total_pages: 10,
							},
						],
					});
				if (cmd === 'send_message') return null;
			});

			const store = await importStore();
			await store.ensureInitialized();
			await store.addActiveCollection({
				id: 'col1',
				name: 'Climate',
				document_count: 0,
				total_pages: 0,
			});

			commands.length = 0;
			await store.sendMessage('hello');

			expect(commands.map((c) => c.cmd)).toEqual([
				'start_chat',
				'list_conversations',
				'send_message',
			]);
			const startChat = commands.find((c) => c.cmd === 'start_chat');
			expect(startChat?.args.collections).toEqual([
				{ id: 'col1', name: 'Climate', document_count: 0, total_pages: 0 },
			]);
			const send = commands.find((c) => c.cmd === 'send_message');
			expect(send?.args).toEqual({
				conversationId: 'new1',
				message: 'hello',
			});
			expect(store.getActiveId()).toBe('new1');
			expect(store.getActiveMessages()).toEqual([
				{ role: 'user', content: [{ type: 'text', text: 'hello' }] },
			]);
			expect(store.getIsGenerating()).toBe(true);
		});

		it('sendMessage from an unscoped draft does nothing', async () => {
			const commands: string[] = [];
			mockIPC((cmd) => {
				commands.push(cmd);
				if (cmd === 'list_conversations') return [];
			});

			const store = await importStore();
			await store.ensureInitialized();
			commands.length = 0;

			await store.sendMessage('hello');
			expect(commands).toEqual([]);
			expect(store.getIsGenerating()).toBe(false);
		});
	});

	it('cancels generation via cancel_generation', async () => {
		const cancelled: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations') return [];
			if (cmd === 'start_chat') return conversationView('c9');
			if (cmd === 'load_conversation') return conversationView('c9');
			if (cmd === 'send_message') return null;
			if (cmd === 'cancel_generation') {
				cancelled.push((args as { conversationId: string }).conversationId);
				return null;
			}
		});

		const store = await importStore();
		store.newConversation();
		await store.addActiveCollection({
			id: 'col1',
			name: 'Test',
			document_count: 0,
			total_pages: 0,
		});
		await store.sendMessage('hi');
		await store.cancelGeneration();

		expect(cancelled).toEqual(['c9']);
		expect(store.getIsGenerating()).toBe(false);
	});
});
