import { describe, it, expect, beforeEach, vi } from 'vitest';
import { mockIPC, clearMocks } from '@tauri-apps/api/mocks';
import { Channel } from '@tauri-apps/api/core';
import type {
	MultiTurnStreamItem,
	RigMessage,
	RigStreamEvent,
	RigToolResultContent,
} from '#lib/types/rig.js';

let channel: Channel<MultiTurnStreamItem> | undefined;
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
	channel = (args as { onEvent: Channel<MultiTurnStreamItem> }).onEvent;
	messageIndex = 0;
}

function emit(payload: MultiTurnStreamItem) {
	if (!channel) throw new Error('send_message has not opened a channel');
	(
		window as unknown as {
			__TAURI_INTERNALS__: {
				runCallback: (id: number, data: unknown) => void;
			};
		}
	).__TAURI_INTERNALS__.runCallback(channel.id, {
		index: messageIndex++,
		message: payload,
	});
}

const callId = (id: string) => ({ provider: id });

const streamEvent = (value: RigStreamEvent): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	item: 'event',
	value,
});

const start = (part: number, kind: 'text' | 'reasoning') =>
	streamEvent({ event: 'start', part, kind });

const textDelta = (part: number, text: string): MultiTurnStreamItem =>
	streamEvent({ event: 'text', part, text });

const endText = (part: number, text: string): MultiTurnStreamItem =>
	streamEvent({ event: 'end', part, content: { type: 'text', text } });

const startToolCall = (part: number, name: string): MultiTurnStreamItem =>
	streamEvent({ event: 'start', part, kind: 'tool_call', name });

const argumentsDelta = (part: number, json: string): MultiTurnStreamItem =>
	streamEvent({ event: 'arguments', part, json });

const endToolCall = (
	part: number,
	id: string,
	name: string,
	args: Record<string, unknown>,
): MultiTurnStreamItem =>
	streamEvent({
		event: 'end',
		part,
		content: {
			type: 'tool_call',
			id: callId(id),
			function: { name, arguments: args },
		},
	});

const reasoningDelta = (part: number, text: string): MultiTurnStreamItem =>
	streamEvent({ event: 'reasoning', part, text });

const endReasoning = (part: number, text: string): MultiTurnStreamItem =>
	streamEvent({
		event: 'end',
		part,
		content: { type: 'reasoning', text },
	});

const committedToolCall = (
	id: string,
	name: string,
	args: Record<string, unknown> = {},
): MultiTurnStreamItem => ({
	type: 'toolCall',
	tool_call: { id: callId(id), function: { name, arguments: args } },
});

const executedToolCall = (
	id: string,
	name: string,
	args: Record<string, unknown> = {},
): MultiTurnStreamItem => ({
	type: 'toolExecutionCommitted',
	tool_call: { id: callId(id), function: { name, arguments: args } },
});

const toolResult = (
	id: string,
	name: string,
	content: RigToolResultContent[],
): MultiTurnStreamItem => ({
	type: 'toolResult',
	tool_result: { call: callId(id), name, content },
});

const USAGE = {
	input_tokens: 0,
	output_tokens: 0,
	total_tokens: 0,
	cached_input_tokens: 0,
	cache_creation_input_tokens: 0,
	reasoning_tokens: 0,
};

function summary(id: string) {
	return { id, title: `Chat ${id}` };
}

function conversationView(
	id: string,
	collectionId: string,
	messages: unknown[] = [],
) {
	return {
		id,
		collection_id: collectionId,
		title: `Chat ${id}`,
		messages,
	};
}

const userMessage = (text: string): RigMessage => ({
	role: 'user',
	content: [{ type: 'text', text }],
});

describe('conversations store event handling', () => {
	beforeEach(() => {
		clearMocks();
		localStorage.clear();
	});

	it('builds a chronological transcript from a streamed multi-turn sequence', async () => {
		const finalMessages: RigMessage[] = [
			{ role: 'user', content: [{ type: 'text', text: 'find papers' }] },
			{
				role: 'assistant',
				content: [
					{ type: 'reasoning', text: 'Thinking' },
					{ type: 'text', text: 'Let me search the index.' },
					{
						type: 'tool_call',
						id: callId('call1'),
						function: { name: 'search', arguments: { q: 'papers' } },
					},
				],
			},
			{
				role: 'user',
				content: [
					{
						type: 'tool_result',
						call: callId('call1'),
						name: 'search',
						content: [{ type: 'text', text: '3 results' }],
					},
				],
			},
			{
				role: 'assistant',
				content: [{ type: 'text', text: 'Found 3 results.' }],
			},
		];
		const views: Record<string, unknown> = {
			// First load (initialize): empty history
			empty: conversationView('conv1', 'col1'),
			// Authoritative refresh after finalResponse
			final: conversationView('conv1', 'col1', finalMessages),
		};
		let currentView = views.empty;
		const loadCalls: Array<Record<string, unknown>> = [];
		const sendCalls: Array<Record<string, unknown>> = [];

		mockIPC((cmd, args) => {
			if (cmd === 'load_conversation') {
				loadCalls.push(args as Record<string, unknown>);
				return currentView;
			}
			if (cmd === 'send_message') {
				sendCalls.push(args as Record<string, unknown>);
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', [summary('conv1')]);
		expect(store.getActiveId()).toBe('conv1');
		loadCalls.length = 0;

		await store.sendMessage('find papers');
		expect(sendCalls).toEqual([
			expect.objectContaining({
				collectionId: 'col1',
				conversationId: 'conv1',
				message: 'find papers',
			}),
		]);
		expect(store.getIsGenerating()).toBe(true);
		expect(store.getActiveMessages()).toEqual([
			{ role: 'user', content: [{ type: 'text', text: 'find papers' }] },
		]);

		// Stream: reasoning, text, tool-call arguments, complete call,
		// committed execution, tool result, usage record.
		emit(start(0, 'reasoning'));
		emit(reasoningDelta(0, 'Thinking'));
		emit(endReasoning(0, 'Thinking'));
		emit(start(1, 'text'));
		emit(textDelta(1, 'Let me search'));
		emit(textDelta(1, ' the index.'));

		const items = store.getTranscript().map((item) => item.kind);
		expect(items).toEqual(['userText', 'reasoning', 'assistantText']);
		const liveItems = store.getTranscript().slice(1);
		expect(liveItems.every((item) => item.provisional)).toBe(true);
		expect(
			store.getTranscript().find((item) => item.kind === 'assistantText'),
		).toMatchObject({ text: 'Let me search the index.', streaming: true });

		emit(endText(1, 'Let me search the index.'));
		emit(startToolCall(2, 'search'));
		emit(argumentsDelta(2, '{"q": "pap'));
		emit(argumentsDelta(2, 'ers"}'));
		emit(endToolCall(2, 'call1', 'search', { q: 'papers' }));
		emit(committedToolCall('call1', 'search', { q: 'papers' }));

		const callItem = () =>
			store.getTranscript().find((item) => item.kind === 'toolCall')!;
		const prettyArgs = JSON.stringify({ q: 'papers' }, null, 2);
		expect(callItem()).toMatchObject({
			name: 'search',
			args: prettyArgs,
			executing: true,
		});

		emit(executedToolCall('call1', 'search', { q: 'papers' }));
		expect(callItem().executing).toBe(false);

		emit(toolResult('call1', 'search', [{ type: 'text', text: '3 results' }]));
		emit({
			type: 'completionCall',
			call_index: 0,
			usage: USAGE,
			raw: null,
		});

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
		emit({
			type: 'finalResponse',
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
			expect(loadCalls).toContainEqual({
				collectionId: 'col1',
				conversationId: 'conv1',
			});
		});
		expect(store.getActiveMessages()[3]?.content[0]).toEqual({
			type: 'text',
			text: 'Found 3 results.',
		});
	});

	it('surfaces stream errors and refreshes history', async () => {
		const currentView = conversationView('conv2', 'col1');
		const loadCalls: string[] = [];
		mockIPC((cmd, args) => {
			if (cmd === 'start_chat') return currentView;
			if (cmd === 'load_conversation') {
				loadCalls.push('load');
				return currentView;
			}
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		await store.initialize('col1', []);
		await store.sendMessage('hello');
		emit(start(0, 'text'));
		emit(textDelta(0, 'partial'));
		emit(endText(0, 'partial'));
		emit({ type: 'streamError', message: 'provider exploded' });

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
					userMessage(collectionId),
				]);
			}
		});

		const store = await importStore();
		await store.initialize('col1', lists.col1.map(summary));
		expect(store.getActiveId()).toBe('a1');
		expect(store.getActiveMessages()).toEqual([userMessage('col1')]);

		await store.initialize('col2', lists.col2.map(summary));
		expect(store.getActiveCollectionId()).toBe('col2');
		expect(store.getActiveId()).toBe('b1');
		expect(store.getActiveMessages()).toEqual([userMessage('col2')]);
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
		expect(store.getActiveMessages()).toEqual([userMessage('hello')]);
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
		expect(store.getActiveMessages()).toEqual([]);
	});

	it('predicts the next message within the active collection', async () => {
		const calls: Array<{ cmd: string; args: Record<string, unknown> }> = [];
		mockIPC((cmd, args) => {
			calls.push({ cmd, args: args as Record<string, unknown> });
			if (cmd === 'load_conversation') return conversationView('c1', 'col1');
			if (cmd === 'predict_next_message') return 'next words';
		});

		const store = await importStore();
		await store.initialize('col1', [summary('c1')]);

		expect(await store.predictNextMessage()).toBe('next words');
		expect(calls.find((c) => c.cmd === 'predict_next_message')?.args).toEqual({
			collectionId: 'col1',
			conversationId: 'c1',
		});
	});

	it('refreshes the conversation list when a run terminates', async () => {
		const view = conversationView('c1', 'col1');
		mockIPC((cmd, args) => {
			if (cmd === 'load_conversation') return view;
			if (cmd === 'send_message') {
				captureChannel(args);
				return null;
			}
		});

		const store = await importStore();
		const { invalidate } = await import('$app/navigation');
		await store.initialize('col1', [summary('c1')]);
		await store.sendMessage('hello');
		emit({
			type: 'finalResponse',
			usage: USAGE,
			messages: [userMessage('hello')],
			content: [{ type: 'text', text: 'hi' }],
		});

		await vi.waitFor(() => {
			expect(invalidate).toHaveBeenCalledWith('app:conversations:col1');
		});
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
					return conversationView('c1', 'col1', [userMessage('hi')]);
			});

			const store = await importStore();
			await store.initialize('col1', [summary('c1')]);
			expect(store.getActiveId()).toBe('c1');

			store.newConversation();
			expect(store.getActiveId()).toBeNull();
			expect(store.getActiveMessages()).toEqual([]);
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
