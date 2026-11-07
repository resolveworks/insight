import { describe, it, expect, beforeEach, vi } from 'vitest';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { Channel } from '@tauri-apps/api/core';
import type {
	ChatEvent,
	ConversationView,
	TranscriptItem,
	TranscriptKind,
} from '#lib/types/chat.js';

let channel: Channel<ChatEvent> | undefined;
let messageIndex = 0;

/**
 * The conversations store keeps module-level $state, so each test re-imports
 * a fresh module copy with the IPC mock already installed.
 */
async function importStore() {
	vi.resetModules();
	channel = undefined;
	messageIndex = 0;
	return await import('./conversations.svelte');
}

function captureChannel(args: unknown) {
	channel = (args as { onEvent: Channel<ChatEvent> }).onEvent;
	messageIndex = 0;
}

function emit(event: ChatEvent) {
	if (!channel) throw new Error('send_message has not opened a channel');
	(
		window as unknown as {
			__TAURI_INTERNALS__: {
				runCallback: (id: number, data: unknown) => void;
			};
		}
	).__TAURI_INTERNALS__.runCallback(channel.id, {
		index: messageIndex++,
		message: event,
	});
}

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

const userItem = (key: string, text: string, provisional = false) =>
	item(key, 'user_text', { text, provisional });

const upserted = (value: TranscriptItem): ChatEvent => ({
	type: 'item_upserted',
	item: value,
});

const removed = (key: string): ChatEvent => ({ type: 'item_removed', key });

const finished = (items: TranscriptItem[]): ChatEvent => ({
	type: 'run_finished',
	items,
});

const failed = (message: string): ChatEvent => ({
	type: 'run_failed',
	message,
});

function summary(id: string) {
	return { id, title: `Chat ${id}` };
}

function conversationView(
	id: string,
	collectionId: string,
	transcript: TranscriptItem[] = [],
): ConversationView {
	return {
		id,
		collection_id: collectionId,
		title: `Chat ${id}`,
		transcript,
	};
}

const storedTranscript = [
	userItem('0-0', 'find papers'),
	item('1-0', 'reasoning', { text: 'Thinking' }),
	item('1-1', 'tool_call', {
		name: 'search',
		text: JSON.stringify({ q: 'papers' }, null, 2),
	}),
	item('2-0', 'tool_result', { name: 'search', text: '3 results' }),
];

describe('conversations store', () => {
	beforeEach(() => {
		clearMocks();
		localStorage.clear();
	});

	it('loads a stored transcript with its tool call and result', async () => {
		mockIPC((cmd) => {
			if (cmd === 'load_conversation')
				return conversationView('conv1', 'col1', storedTranscript);
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);

		expect(store.getActiveId()).toBe('conv1');
		expect(store.getItems()).toEqual(storedTranscript);
		expect(store.getItems().map((entry) => entry.kind)).toEqual([
			'user_text',
			'reasoning',
			'tool_call',
			'tool_result',
		]);
	});

	it('applies a run’s streamed events to the transcript in place', async () => {
		mockIPC((cmd, args) => {
			if (cmd === 'load_conversation')
				return conversationView('conv1', 'col1', [
					userItem('0-0', 'earlier question'),
				]);
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);
		await store.sendMessage('find papers');
		expect(store.getIsGenerating()).toBe(true);
		expect(store.getItems()).toEqual([userItem('0-0', 'earlier question')]);

		emit(upserted(userItem('1-0', 'find papers', true)));
		emit(
			upserted(
				item('2-0', 'assistant_text', { provisional: true, streaming: true }),
			),
		);
		expect(store.getItems().map((entry) => entry.key)).toEqual([
			'0-0',
			'1-0',
			'2-0',
		]);
		expect(store.getItems()[2]).toMatchObject({ text: '', streaming: true });

		emit(
			upserted(
				item('2-0', 'assistant_text', {
					text: 'Let me search',
					provisional: true,
					streaming: true,
				}),
			),
		);
		expect(store.getItems()[2]).toMatchObject({
			key: '2-0',
			text: 'Let me search',
			streaming: true,
		});
		expect(store.getItems()).toHaveLength(3);

		emit(
			upserted(
				item('2-1', 'tool_call', {
					name: 'search',
					text: JSON.stringify({ q: 'papers' }, null, 2),
					provisional: true,
					executing: true,
				}),
			),
		);
		expect(store.getItems()[3]).toMatchObject({
			kind: 'tool_call',
			name: 'search',
			executing: true,
		});

		emit(removed('2-1'));
		expect(store.getItems().map((entry) => entry.key)).toEqual([
			'0-0',
			'1-0',
			'2-0',
		]);
	});

	it('commits a finished run over its provisional items without duplicating it', async () => {
		let currentView = conversationView('conv1', 'col1');
		const loadCalls: Array<Record<string, unknown>> = [];
		mockIPC((cmd, args) => {
			if (cmd === 'load_conversation') {
				loadCalls.push(args as Record<string, unknown>);
				return currentView;
			}
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);
		loadCalls.length = 0;

		await store.sendMessage('find papers');
		emit(upserted(userItem('0-0', 'find papers', true)));
		emit(
			upserted(
				item('1-0', 'assistant_text', {
					text: 'Found 3 results.',
					provisional: true,
				}),
			),
		);

		const final = [
			userItem('0-0', 'find papers'),
			item('1-0', 'reasoning', { text: 'Thinking' }),
			item('1-1', 'tool_call', {
				name: 'search',
				text: JSON.stringify({ q: 'papers' }, null, 2),
			}),
			item('2-0', 'tool_result', { name: 'search', text: '3 results' }),
			item('3-0', 'assistant_text', { text: 'Found 3 results.' }),
		];
		currentView = conversationView('conv1', 'col1', final);
		emit(finished(final));

		// The commit is visible without waiting for the reconciliation reload.
		expect(store.getIsGenerating()).toBe(false);
		expect(store.getItems()).toEqual(final);
		expect(store.getItems().map((entry) => entry.key)).toEqual(
			final.map((entry) => entry.key),
		);

		await vi.waitFor(() => {
			expect(loadCalls).toContainEqual({
				collectionId: 'col1',
				conversationId: 'conv1',
			});
		});
		const { invalidate } = await import('$app/navigation');
		expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');
		expect(store.getItems()).toEqual(final);
	});

	it('clears a failed run’s provisional items and shows its error', async () => {
		const stored = [userItem('0-0', 'hello')];
		const loadCalls: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'load_conversation') {
				loadCalls.push((args as { conversationId: string }).conversationId);
				return conversationView('conv1', 'col1', stored);
			}
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);
		loadCalls.length = 0;

		await store.sendMessage('hello');
		emit(
			upserted(
				item('1-0', 'assistant_text', { text: 'partial', provisional: true }),
			),
		);
		emit(failed('provider exploded'));

		expect(store.getItems()).toEqual(stored);
		expect(store.getError()).toBe('provider exploded');
		expect(store.getIsGenerating()).toBe(false);

		await vi.waitFor(() => {
			expect(loadCalls).toEqual(['conv1']);
		});
		const { invalidate } = await import('$app/navigation');
		expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');

		store.clearError();
		expect(store.getError()).toBeNull();
	});

	it('ignores events for a conversation that is no longer active', async () => {
		mockIPC((cmd, args) => {
			const { conversationId } = args as { conversationId: string };
			if (cmd === 'load_conversation') {
				return conversationView(
					conversationId,
					'col1',
					conversationId === 'conv2' ? [userItem('0-0', 'other chat')] : [],
				);
			}
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);
		await store.sendMessage('hello');

		await store.selectConversation('conv2');
		const otherTranscript = store.getItems();
		expect(otherTranscript).toEqual([userItem('0-0', 'other chat')]);

		emit(upserted(item('1-0', 'assistant_text', { text: 'stale' })));
		emit(failed('stale failure'));

		expect(store.getItems()).toEqual(otherTranscript);
		expect(store.getError()).toBeNull();
	});

	it('initializing a collection opens its most recent loaded conversation', async () => {
		const calls: string[] = [];
		const loadCalls: Array<Record<string, unknown>> = [];

		mockIPC((cmd, args) => {
			calls.push(cmd);
			if (cmd === 'load_conversation') {
				loadCalls.push(args as Record<string, unknown>);
				const { conversationId } = args as { conversationId: string };
				return conversationView(conversationId, 'col1');
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('recent'), summary('older')]);

		expect(calls).not.toContain('list_conversations');
		expect(loadCalls).toEqual([
			{ collectionId: 'col1', conversationId: 'recent' },
		]);
		expect(store.getActiveId()).toBe('recent');
		expect(store.getActiveCollectionId()).toBe('col1');
	});

	it('switching collections resets to that collection without leaking the previous chat', async () => {
		const lists: Record<string, string[]> = {
			col1: ['a1'],
			col2: ['b1'],
		};

		mockIPC((cmd, args) => {
			const { collectionId, conversationId } = args as {
				collectionId: string;
				conversationId?: string;
			};
			if (cmd === 'load_conversation') {
				return conversationView(conversationId!, collectionId, [
					userItem('0-0', collectionId),
				]);
			}
		});

		const store = await importStore();
		await store.initialize('col1', lists.col1.map(summary));
		expect(store.getActiveId()).toBe('a1');
		expect(store.getItems()).toEqual([userItem('0-0', 'col1')]);

		await store.initialize('col2', lists.col2.map(summary));
		expect(store.getActiveCollectionId()).toBe('col2');
		expect(store.getActiveId()).toBe('b1');
		expect(store.getItems()).toEqual([userItem('0-0', 'col2')]);
	});

	it('sending from a draft starts the chat in its collection, then sends', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];

		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'start_chat') return conversationView('new1', 'col1');
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', []);
		expect(store.getActiveId()).toBeNull();

		await store.sendMessage('hello');

		expect(calls.find((c) => c.cmd === 'start_chat')?.args).toEqual({
			collectionId: 'col1',
		});
		expect(calls.find((c) => c.cmd === 'send_message')?.args).toEqual(
			expect.objectContaining({
				collectionId: 'col1',
				conversationId: 'new1',
				message: 'hello',
				onEvent: expect.anything(),
			}),
		);
		expect(store.getActiveId()).toBe('new1');
		// No optimistic user item: the run's first upsert carries it.
		expect(store.getItems()).toEqual([]);
		expect(store.getIsGenerating()).toBe(true);
	});

	it('remembers the active conversation per collection', async () => {
		localStorage.setItem('insight:activeConversationId:col2', 'b1');

		const lists: Record<string, string[]> = {
			col1: ['a1'],
			col2: ['b2', 'b1'],
		};

		mockIPC((cmd, args) => {
			const { collectionId, conversationId } = args as {
				collectionId: string;
				conversationId?: string;
			};
			if (cmd === 'load_conversation') {
				return conversationView(conversationId!, collectionId);
			}
		});

		const store = await importStore();
		// No stored chat for col1: it opens the most recent one.
		await store.initialize('col1', lists.col1.map(summary));
		expect(store.getActiveId()).toBe('a1');
		expect(localStorage.getItem('insight:activeConversationId:col1')).toBe(
			'a1',
		);

		// col2 has a stored chat that is not the most recent one.
		await store.initialize('col2', lists.col2.map(summary));
		expect(store.getActiveId()).toBe('b1');
	});

	it('deletes a conversation in the active collection', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'load_conversation') return conversationView('c1', 'col1');
			if (cmd === 'delete_conversation') return null;
		});

		const store = await importStore();
		const { invalidate } = await import('$app/navigation');
		await store.initialize('col1', [summary('c1')]);
		expect(store.getActiveId()).toBe('c1');

		await store.deleteConversation('c1', []);
		expect(calls.find((c) => c.cmd === 'delete_conversation')?.args).toEqual({
			collectionId: 'col1',
			conversationId: 'c1',
		});
		expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');
		// No auto-create: the user lands in the draft state.
		expect(store.getActiveId()).toBeNull();
		expect(store.getItems()).toEqual([]);
	});

	describe('draft (lazy) conversations', () => {
		it('initializes an empty collection in the draft state', async () => {
			const store = await importStore();
			await store.initialize('col1', []);

			expect(store.getActiveCollectionId()).toBe('col1');
			expect(store.getActiveId()).toBeNull();
		});

		it('newConversation resets to a draft within the collection', async () => {
			mockIPC((cmd) => {
				if (cmd === 'load_conversation')
					return conversationView('c1', 'col1', [userItem('0-0', 'hi')]);
			});

			const store = await importStore();
			await store.initialize('col1', [summary('c1')]);
			expect(store.getActiveId()).toBe('c1');

			store.newConversation();
			expect(store.getActiveId()).toBeNull();
			expect(store.getItems()).toEqual([]);
			expect(store.getActiveCollectionId()).toBe('col1');
		});
	});

	it('cancels generation via cancel_generation', async () => {
		const cancelled: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'start_chat') return conversationView('c9', 'col1');
			if (cmd === 'load_conversation') return conversationView('c9', 'col1');
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
			if (cmd === 'cancel_generation') {
				cancelled.push((args as { conversationId: string }).conversationId);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', []);
		await store.sendMessage('hi');
		await store.cancelGeneration();

		expect(cancelled).toEqual(['c9']);
		expect(store.getIsGenerating()).toBe(false);
	});
});
