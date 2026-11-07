import { describe, expect, it } from 'vitest';
import type {
	MultiTurnStreamItem,
	RigCallId,
	RigMessage,
	RigStreamEvent,
	RigToolResultContent,
} from '#lib/types/rig.js';
import { applyItem, buildTranscript, emptyLiveRun } from './transcript';

/** Convenience fold over a list of items. */
function fold(items: MultiTurnStreamItem[]) {
	return items.reduce(applyItem, emptyLiveRun());
}

const callId = (id: string): RigCallId => ({ provider: id });

const streamItem = (value: RigStreamEvent): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	item: 'event',
	value,
});

const start = (part: number, kind: 'text' | 'reasoning') =>
	streamItem({ event: 'start', part, kind });

const textDelta = (part: number, text: string): MultiTurnStreamItem =>
	streamItem({ event: 'text', part, text });

const endText = (part: number, text: string): MultiTurnStreamItem =>
	streamItem({ event: 'end', part, content: { type: 'text', text } });

const startToolCall = (part: number, name: string): MultiTurnStreamItem =>
	streamItem({ event: 'start', part, kind: 'tool_call', name });

const argumentsDelta = (part: number, json: string): MultiTurnStreamItem =>
	streamItem({ event: 'arguments', part, json });

const endToolCall = (
	part: number,
	id: string,
	name: string,
	args: Record<string, unknown>,
): MultiTurnStreamItem =>
	streamItem({
		event: 'end',
		part,
		content: {
			type: 'tool_call',
			id: callId(id),
			function: { name, arguments: args },
		},
	});

const reasoningDelta = (part: number, text: string): MultiTurnStreamItem =>
	streamItem({ event: 'reasoning', part, text });

const endReasoning = (part: number, text: string): MultiTurnStreamItem =>
	streamItem({
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

describe('applyItem — text', () => {
	it('accumulates text deltas and finalizes on end', () => {
		const streaming = fold([
			start(0, 'text'),
			textDelta(0, 'Hello '),
			textDelta(0, 'world'),
		]);
		expect(streaming.streamingText).toBe(true);
		expect(streaming.messages).toEqual([
			{
				role: 'assistant',
				content: [{ type: 'text', part: 0, text: 'Hello world' }],
			},
		]);

		const run = fold([
			start(0, 'text'),
			textDelta(0, 'Hello '),
			textDelta(0, 'world'),
			endText(0, 'Hello world'),
		]);
		expect(run.streamingText).toBe(false);
		expect(run.messages).toEqual([
			{
				role: 'assistant',
				content: [{ type: 'text', part: 0, text: 'Hello world' }],
			},
		]);
	});

	it('starts a new text block after a non-text part', () => {
		const run = fold([
			start(0, 'text'),
			textDelta(0, 'first'),
			endText(0, 'first'),
			startToolCall(1, 'search'),
			argumentsDelta(1, '{}'),
			endToolCall(1, 'call_1', 'search', {}),
			start(2, 'text'),
			textDelta(2, 'second'),
		]);
		expect(run.messages[0]).toEqual({
			role: 'assistant',
			content: [
				{ type: 'text', part: 0, text: 'first' },
				{
					type: 'tool_call',
					part: 1,
					id: 'call_1',
					name: 'search',
					args: '{}',
					committed: false,
					executed: false,
				},
				{ type: 'text', part: 2, text: 'second' },
			],
		});
	});
});

describe('applyItem — chronological interleaving', () => {
	it('text → tool call → tool result → text lands in that order with segmented text', () => {
		const run = fold([
			start(0, 'text'),
			textDelta(0, 'before'),
			endText(0, 'before'),
			startToolCall(1, 'search'),
			argumentsDelta(1, '{}'),
			endToolCall(1, 'call_1', 'search', {}),
			committedToolCall('call_1', 'search'),
			executedToolCall('call_1', 'search'),
			toolResult('call_1', 'search', [{ type: 'text', text: 'hits' }]),
			start(0, 'text'),
			textDelta(0, 'after'),
		]);
		expect(run.messages).toHaveLength(3);
		expect(run.messages[0].role).toBe('assistant');
		expect(run.messages[1]).toEqual({
			role: 'user',
			content: [
				{ type: 'tool_result', call: 'call_1', name: 'search', text: 'hits' },
			],
		});
		expect(run.messages[2].role).toBe('assistant');
		const items = buildTranscript([], run);
		expect(items.map((i) => i.kind)).toEqual([
			'assistantText',
			'toolCall',
			'toolResult',
			'assistantText',
		]);
		expect(items[0].text).toBe('before');
		expect(items[3].text).toBe('after');
	});
});

describe('applyItem — tool calls', () => {
	it('stitches argument fragments by part', () => {
		const run = fold([
			startToolCall(0, 'search'),
			startToolCall(1, 'other'),
			argumentsDelta(0, '{"q"'),
			argumentsDelta(0, ': 1}'),
		]);
		const calls = (
			run.messages[0] as {
				content: { type: string; name?: string; args?: string }[];
			}
		).content;
		expect(calls[0]).toMatchObject({ name: 'search', args: '{"q": 1}' });
		expect(calls[1]).toMatchObject({ name: 'other', args: '' });
		expect(run.streamingText).toBe(false);
	});

	it('end supersedes streamed fragments; commit and execution set flags', () => {
		const run = fold([
			startToolCall(0, 'sea'),
			argumentsDelta(0, '{"q"'),
			endToolCall(0, 'call_1', 'search', { q: 'hi' }),
			committedToolCall('call_1', 'search', { q: 'hi' }),
			executedToolCall('call_1', 'search', { q: 'hi' }),
		]);
		const block = (
			run.messages[0] as unknown as { content: Record<string, unknown>[] }
		).content[0];
		expect(block).toMatchObject({
			type: 'tool_call',
			part: 0,
			id: 'call_1',
			name: 'search',
			args: '{\n  "q": "hi"\n}',
			committed: true,
			executed: true,
		});
	});
});

describe('applyItem — reasoning', () => {
	it('accumulates deltas by part and replaces them on end', () => {
		const streaming = fold([
			start(0, 'reasoning'),
			reasoningDelta(0, 'think '),
			reasoningDelta(0, 'more'),
		]);
		expect(streaming.messages[0]).toEqual({
			role: 'assistant',
			content: [{ type: 'reasoning', part: 0, text: 'think more' }],
		});

		const run = fold([
			start(0, 'reasoning'),
			reasoningDelta(0, 'think '),
			reasoningDelta(0, 'more'),
			endReasoning(0, 'final'),
		]);
		expect(run.messages[0]).toEqual({
			role: 'assistant',
			content: [{ type: 'reasoning', part: 0, text: 'final' }],
		});
	});
});

describe('applyItem — control items', () => {
	it('toolResult flattens text and json content', () => {
		const run = fold([
			toolResult('call_1', 'search', [
				{ type: 'text', text: 'hits' },
				{ type: 'image', data: 'u' },
				{ type: 'json', value: { n: 1 } },
			]),
		]);
		expect(run.messages[0]).toEqual({
			role: 'user',
			content: [
				{
					type: 'tool_result',
					call: 'call_1',
					name: 'search',
					text: 'hits\n{\n  "n": 1\n}',
				},
			],
		});
	});

	it('modelTurnRetried drops only the rejected turn, keeping earlier segments', () => {
		const run = fold([
			start(0, 'text'),
			textDelta(0, 'Searching.'),
			endText(0, 'Searching.'),
			startToolCall(1, 'search'),
			endToolCall(1, 'call_1', 'search', { query: 'x' }),
			committedToolCall('call_1', 'search', { query: 'x' }),
			executedToolCall('call_1', 'search', { query: 'x' }),
			toolResult('call_1', 'search', [{ type: 'text', text: 'hits' }]),
			start(0, 'text'),
			textDelta(0, 'draft'),
			start(1, 'reasoning'),
			reasoningDelta(1, 'thinking'),
			{ type: 'modelTurnRetried', turn: 2 },
		]);
		expect(run).toEqual({
			streamingText: false,
			messages: [
				{
					role: 'assistant',
					content: [
						{ type: 'text', part: 0, text: 'Searching.' },
						{
							type: 'tool_call',
							part: 1,
							id: 'call_1',
							name: 'search',
							args: '{\n  "query": "x"\n}',
							committed: true,
							executed: true,
						},
					],
				},
				{
					role: 'user',
					content: [
						{
							type: 'tool_result',
							call: 'call_1',
							name: 'search',
							text: 'hits',
						},
					],
				},
			],
		});
	});

	it('modelTurnRetried keeps earlier turns when the rejected attempt streamed nothing', () => {
		const run = fold([
			start(0, 'text'),
			textDelta(0, 'kept'),
			{ type: 'modelTurnRetried', turn: 2 },
			{ type: 'modelTurnRetried', turn: 3 },
		]);
		expect(run).toEqual({
			streamingText: false,
			messages: [
				{
					role: 'assistant',
					content: [{ type: 'text', part: 0, text: 'kept' }],
				},
			],
		});
	});

	it('completionCall, finalResponse, and unknown payloads are no-ops', () => {
		const base = fold([start(0, 'text'), textDelta(0, 'keep')]);
		const noop = (item: MultiTurnStreamItem) => {
			const next = applyItem(base, item);
			expect(next).toEqual(base);
		};
		noop({
			type: 'completionCall',
			call_index: 0,
			usage: { input_tokens: 1, output_tokens: 1, total_tokens: 2 },
			raw: null,
		});
		noop({
			type: 'finalResponse',
			usage: {},
			messages: [],
			content: [],
		});
		noop({
			type: 'streamAssistantItem',
			item: 'unknown',
			value: { whatever: true },
		});
	});
});

describe('applyItem — immutability', () => {
	it('does not mutate the run argument', () => {
		const run = fold([
			start(0, 'text'),
			textDelta(0, 'a'),
			startToolCall(1, 't'),
		]);
		const clone = structuredClone(run);
		applyItem(run, textDelta(0, 'b'));
		applyItem(run, endToolCall(1, 'call_1', 't', {}));
		applyItem(run, committedToolCall('call_1', 't'));
		applyItem(run, executedToolCall('call_1', 't'));
		applyItem(run, toolResult('call_1', 't', [{ type: 'text', text: 'r' }]));
		applyItem(run, { type: 'modelTurnRetried', turn: 1 });
		expect(run).toEqual(clone);
	});
});

describe('buildTranscript', () => {
	const stored: RigMessage[] = [
		{ role: 'user', content: [{ type: 'text', text: 'hi' }] },
		{
			role: 'assistant',
			content: [
				{ type: 'reasoning', text: 'think' },
				{ type: 'text', text: 'hello' },
				{
					type: 'tool_call',
					id: callId('c1'),
					function: { name: 'search', arguments: { q: 'x' } },
				},
				{
					type: 'tool_call',
					id: callId('c2'),
					function: { name: 'read', arguments: { id: 1 } },
				},
			],
		},
		{
			role: 'user',
			content: [
				{
					type: 'tool_result',
					call: callId('c1'),
					name: 'search',
					content: [{ type: 'text', text: 'result' }],
				},
			],
		},
	];

	it('maps all stored block kinds with stringified args', () => {
		const items = buildTranscript(stored, emptyLiveRun());
		expect(items).toHaveLength(6);
		expect(items.map((i) => i.key)).toEqual([
			'0-0',
			'1-0',
			'1-1',
			'1-2',
			'1-3',
			'2-0',
		]);
		expect(items.map((i) => i.kind)).toEqual([
			'userText',
			'reasoning',
			'assistantText',
			'toolCall',
			'toolCall',
			'toolResult',
		]);
		expect(items[0]).toMatchObject({ role: 'user', text: 'hi' });
		expect(items[1]).toMatchObject({ role: 'assistant', text: 'think' });
		expect(items[2]).toMatchObject({ role: 'assistant', text: 'hello' });
		expect(items[3]).toMatchObject({
			name: 'search',
			args: '{\n  "q": "x"\n}',
		});
		expect(items[4]).toMatchObject({
			name: 'read',
			args: '{\n  "id": 1\n}',
		});
		expect(items[5]).toMatchObject({
			role: 'user',
			name: 'search',
			text: 'result',
		});
		for (const item of items) {
			expect(item.provisional).toBe(false);
			expect(item.executing).toBe(false);
			expect(item.streaming).toBe(false);
		}
	});

	it('maps live blocks with provisional/executing flags', () => {
		const live = fold([
			start(0, 'text'),
			textDelta(0, 'live '),
			textDelta(0, 'text'),
			startToolCall(1, 'search'),
			argumentsDelta(1, '{}'),
			endToolCall(1, 'call_1', 'search', { q: 1 }),
			committedToolCall('call_1', 'search', { q: 1 }),
			executedToolCall('call_1', 'search', { q: 1 }),
			startToolCall(2, 'read'),
			argumentsDelta(2, '{}'),
			endToolCall(2, 'call_2', 'read', {}),
			committedToolCall('call_2', 'read'),
		]);
		const items = buildTranscript([], live);
		expect(items.map((i) => i.key)).toEqual(['0-0', '0-1', '0-2']);
		expect(items[1]).toMatchObject({
			provisional: true,
			name: 'search',
			executing: false,
		});
		expect(items[2]).toMatchObject({ name: 'read', executing: true });
		expect(items[0]).toMatchObject({
			kind: 'assistantText',
			text: 'live text',
		});
	});

	it('places the streaming cursor on the last assistantText item only', () => {
		const stored2: RigMessage[] = [
			{ role: 'user', content: [{ type: 'text', text: 'q' }] },
			{ role: 'assistant', content: [{ type: 'text', text: 'old' }] },
		];
		const live = fold([
			start(0, 'text'),
			textDelta(0, 'first'),
			endText(0, 'first'),
			startToolCall(1, 'search'),
			argumentsDelta(1, '{}'),
			endToolCall(1, 'call_1', 'search', {}),
			start(2, 'text'),
			textDelta(2, 'second'),
		]);
		const items = buildTranscript(stored2, live);
		const streaming = items.filter((i) => i.streaming);
		expect(streaming).toHaveLength(1);
		expect(streaming[0].key).toBe('2-2');
		expect(streaming[0].text).toBe('second');
	});

	it('handles empty inputs', () => {
		expect(buildTranscript([], emptyLiveRun())).toEqual([]);
	});

	it('renders nothing for unknown stored block kinds', () => {
		const message = {
			role: 'assistant',
			content: [
				{ type: 'text', text: 'before' },
				{ type: 'image', data: 'https://example.invalid/x.png' },
			],
		} as unknown as RigMessage;
		const items = buildTranscript([message], emptyLiveRun());
		expect(items).toHaveLength(1);
		expect(items[0]).toMatchObject({ kind: 'assistantText', text: 'before' });
	});
});
