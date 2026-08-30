import { describe, it, expect, beforeEach, vi } from 'vitest';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import type { MultiTurnStreamItem } from '$lib/types/rig';

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

	it('builds live assistant state from a streamed multi-turn sequence', async () => {
		const views: Record<string, unknown> = {
			// First load (start_chat): empty history
			empty: conversationView('conv1'),
			// Authoritative refresh after finalResponse
			final: conversationView('conv1', {
				messages: [
					{ role: 'user', content: [{ type: 'text', text: 'find papers' }] },
					{
						role: 'assistant',
						id: null,
						content: [
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
				],
			}),
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

		let live = store.getLiveState();
		expect(live.text).toBe('Let me search the index.');
		expect(live.reasoning).toEqual([{ id: 'r1', text: 'Thinking' }]);
		expect(live.toolCalls).toHaveLength(1);
		expect(live.toolCalls[0]).toMatchObject({
			internal_call_id: 'ic1',
			name: 'search',
			args: '{"q": "papers"}',
			committed: false,
			executed: false,
		});

		emit('conv1', {
			type: 'streamAssistantItem',
			tool_call: {
				id: 'call1',
				function: { name: 'search', arguments: { q: 'papers' } },
			},
			internal_call_id: 'ic1',
		});
		emit('conv1', {
			type: 'toolExecutionCommitted',
			tool_call: { id: 'call1', function: { name: 'search', arguments: {} } },
			internal_call_id: 'ic1',
		});
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

		live = store.getLiveState();
		expect(live.toolCalls[0]).toMatchObject({
			committed: true,
			executed: true,
		});
		expect(live.toolResults).toEqual([
			{ internal_call_id: 'ic1', name: 'search', text: '3 results' },
		]);
		expect(store.getIsGenerating()).toBe(true);

		// Terminal: refresh from the server's authoritative history.
		currentView = views.final;
		emit('conv1', {
			type: 'finalResponse',
			output: 'Found 3 results.',
			usage: USAGE,
			messages: null,
			content: [{ type: 'text', text: 'Found 3 results.' }],
		});

		await vi.waitFor(() => {
			expect(store.getIsGenerating()).toBe(false);
		});
		await vi.waitFor(() => {
			expect(store.getActiveMessages()).toHaveLength(4);
		});
		expect(loadCalls).toContain('load');
		expect(store.getLiveState()).toEqual({
			text: '',
			reasoning: [],
			toolCalls: [],
			toolResults: [],
		});
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
		expect(store.getLiveState().reasoning).toEqual([
			{ id: 'r1', text: 'final text' },
		]);
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
		expect(store.getLiveState()).toEqual({
			text: '',
			reasoning: [],
			toolCalls: [],
			toolResults: [],
		});
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
		expect(store.getLiveState().text).toBe('');
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
		await store.newConversation();
		await store.sendMessage('hello');
		emit('conv2', { type: 'streamAssistantItem', text: 'partial' });
		emit('conv2', { type: 'streamError', message: 'provider exploded' });

		await vi.waitFor(() => {
			expect(store.getError()).toBe('provider exploded');
		});
		expect(store.getIsGenerating()).toBe(false);
		expect(store.getLiveState().text).toBe('');
		expect(loadCalls).toContain('load');
		store.clearError();
		expect(store.getError()).toBeNull();
	});

	it('switches collection scope via set_conversation_collections', async () => {
		let scope: unknown[] | null = null;
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations') return [];
			if (cmd === 'start_chat') return conversationView('conv3');
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
		await store.newConversation();

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

	it('deletes the active conversation and falls back to a fresh chat', async () => {
		const deleted: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'list_conversations')
				return [
					{ id: 'c1', title: 'Only chat', updated_at: '2024-01-01T00:00:00Z' },
				];
			if (cmd === 'load_conversation') return conversationView('c1');
			if (cmd === 'start_chat') return conversationView('c2');
			if (cmd === 'delete_conversation') {
				deleted.push((args as { conversationId: string }).conversationId);
				return null;
			}
		});

		const store = await importStore();
		await store.ensureInitialized();
		expect(store.getActiveId()).toBe('c1');

		await store.deleteConversation('c1');
		expect(deleted).toEqual(['c1']);
		expect(store.getActiveId()).toBe('c2');
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
		await store.newConversation();
		await store.sendMessage('hi');
		await store.cancelGeneration();

		expect(cancelled).toEqual(['c9']);
		expect(store.getIsGenerating()).toBe(false);
	});
});
