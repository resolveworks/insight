import { describe, expect, it } from 'vitest';
import type { MultiTurnStreamItem, RigMessage } from '$lib/types/rig';
import {
	applyItem,
	buildTranscript,
	emptyLiveRun,
	flattenReasoning,
	flattenToolResult,
} from './transcript';

/** Convenience fold over a list of items. */
function fold(items: MultiTurnStreamItem[]) {
	return items.reduce(applyItem, emptyLiveRun());
}

const textDelta = (text: string): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	text,
});

const toolCallDelta = (
	internal_call_id: string,
	content: { Name?: string; Delta?: string },
): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	internal_call_id,
	content,
});

const completeToolCall = (
	internal_call_id: string,
	name: string,
	arguments_?: unknown,
	id = 'call_1',
): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	internal_call_id,
	tool_call: { id, function: { name, arguments: arguments_ ?? {} } },
});

const reasoningDelta = (id: string, text: string): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	id,
	reasoning: text,
});

const completeReasoning = (
	id: string,
	parts: { type: string; [k: string]: unknown }[],
): MultiTurnStreamItem => ({
	type: 'streamAssistantItem',
	id,
	reasoning: { id: null, content: parts as never },
});

const streamUserItem = (
	name: string,
	content: { type: string; [k: string]: unknown }[],
	call = 'call_1',
): MultiTurnStreamItem => ({
	type: 'streamUserItem',
	internal_call_id: call,
	tool_result: { call, name, content: content as never },
});

describe('applyItem — text', () => {
	it('accumulates consecutive text deltas into one block', () => {
		const run = fold([textDelta('Hello '), textDelta('world')]);
		expect(run.streamingText).toBe(true);
		expect(run.messages).toEqual([
			{ role: 'assistant', content: [{ type: 'text', text: 'Hello world' }] },
		]);
	});

	it('starts a new text block after a non-text block', () => {
		const run = fold([
			textDelta('first'),
			toolCallDelta('a', { Name: 'search' }),
			textDelta('second'),
		]);
		expect(run.messages[0]).toEqual({
			role: 'assistant',
			content: [
				{ type: 'text', text: 'first' },
				{
					type: 'tool_call',
					internal_call_id: 'a',
					id: '',
					name: 'search',
					args: '',
					committed: false,
					executed: false,
				},
				{ type: 'text', text: 'second' },
			],
		});
	});
});

describe('applyItem — chronological interleaving', () => {
	it('text → tool call → tool result → text lands in that order with segmented text', () => {
		const run = fold([
			textDelta('before'),
			completeToolCall('a', 'search'),
			streamUserItem('search', [{ type: 'text', text: 'hits' }]),
			textDelta('after'),
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
	it('stitches name/args fragments by internal_call_id', () => {
		const run = fold([
			toolCallDelta('a', { Name: 'sea' }),
			toolCallDelta('b', { Name: 'other' }),
			toolCallDelta('a', { Delta: '{"q"' }),
			toolCallDelta('a', { Delta: ': 1}' }),
		]);
		const calls = (
			run.messages[0] as {
				content: { type: string; name?: string; args?: string }[];
			}
		).content;
		expect(calls[0]).toMatchObject({ name: 'sea', args: '{"q": 1}' });
		expect(calls[1]).toMatchObject({ name: 'other', args: '' });
		expect(run.streamingText).toBe(false);
	});

	it('complete call overwrites and sets committed; execution sets executed', () => {
		const run = fold([
			toolCallDelta('a', { Name: 'sea', Delta: '{"q"' }),
			completeToolCall('a', 'search', { q: 'hi' }),
			{
				type: 'toolExecutionCommitted',
				internal_call_id: 'a',
				tool_call: {
					id: 'call_1',
					function: { name: 'search', arguments: {} },
				},
			},
		]);
		const block = (
			run.messages[0] as unknown as { content: Record<string, unknown>[] }
		).content[0];
		expect(block).toMatchObject({
			type: 'tool_call',
			internal_call_id: 'a',
			id: 'call_1',
			name: 'search',
			args: '{\n  "q": "hi"\n}',
			committed: true,
			executed: true,
		});
	});
});

describe('applyItem — reasoning', () => {
	it('accumulates deltas keyed by id and replaces on complete', () => {
		const run = fold([
			reasoningDelta('r1', 'think '),
			reasoningDelta('r1', 'more'),
			completeReasoning('r1', [
				{ type: 'text', content: { text: 'final' } },
				{ type: 'summary', content: 'sum' },
				{ type: 'encrypted', content: 'xxx' },
			]),
		]);
		expect(run.messages[0]).toEqual({
			role: 'assistant',
			content: [{ type: 'reasoning', key: 'r1', text: 'final\nsum' }],
		});
	});
});

describe('applyItem — control items', () => {
	it('streamUserItem flattens text and json content', () => {
		const run = fold([
			streamUserItem('search', [
				{ type: 'text', text: 'hits' },
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

	it('modelTurnRetried resets to empty', () => {
		const run = fold([textDelta('bye'), { type: 'modelTurnRetried', turn: 2 }]);
		expect(run).toEqual(emptyLiveRun());
	});

	it('completionCall, StreamFinal-shaped payloads, and unknown payloads are no-ops', () => {
		const base = fold([textDelta('keep')]);
		const noop = (item: MultiTurnStreamItem) => {
			const clone = structuredClone(base);
			const next = applyItem(base, item);
			expect(next).toEqual(base);
			expect(base).toEqual(clone);
		};
		noop({
			type: 'completionCall',
			call_index: 0,
			usage: {
				input_tokens: 1,
				output_tokens: 1,
				total_tokens: 2,
				cached_input_tokens: 0,
				cache_creation_input_tokens: 0,
				reasoning_tokens: 0,
			},
		});
		noop({
			type: 'streamAssistantItem',
			kind: 'final',
			usage: {
				input_tokens: 1,
				output_tokens: 1,
				total_tokens: 2,
				cached_input_tokens: 0,
				cache_creation_input_tokens: 0,
				reasoning_tokens: 0,
			},
			provider: 'openai',
		});
		noop({
			type: 'streamAssistantItem',
			whatever: true,
		} as unknown as MultiTurnStreamItem);
	});
});

describe('applyItem — immutability', () => {
	it('does not mutate the run argument', () => {
		const run = fold([textDelta('a'), toolCallDelta('x', { Name: 't' })]);
		const clone = structuredClone(run);
		applyItem(run, textDelta('b'));
		applyItem(run, completeToolCall('x', 't'));
		applyItem(run, {
			type: 'toolExecutionCommitted',
			internal_call_id: 'x',
			tool_call: { id: 'call_1', function: { name: 't', arguments: {} } },
		});
		applyItem(run, streamUserItem('t', [{ type: 'text', text: 'r' }]));
		applyItem(run, { type: 'modelTurnRetried', turn: 1 });
		expect(run).toEqual(clone);
	});
});

describe('flattening helpers', () => {
	it('flattenReasoning maps text/summary and skips others', () => {
		expect(
			flattenReasoning({
				id: null,
				content: [
					{ type: 'text', content: { text: 'a' } },
					{ type: 'encrypted', content: 'x' },
					{ type: 'redacted', content: { data: 'y' } },
					{ type: 'summary', content: 'b' },
				],
			}),
		).toBe('a\nb');
	});

	it('flattenToolResult maps text/json and skips others', () => {
		expect(
			flattenToolResult([
				{ type: 'text', text: 'hi' },
				{ type: 'image', url: 'u' },
				{ type: 'json', value: [1] },
			]),
		).toBe('hi\n[\n  1\n]');
	});
});

describe('buildTranscript', () => {
	const stored: RigMessage[] = [
		{ role: 'user', content: [{ type: 'text', text: 'hi' }] },
		{
			role: 'assistant',
			id: null,
			content: [
				{
					type: 'reasoning',
					id: null,
					content: [{ type: 'text', content: { text: 'think' } }],
				},
				{ type: 'text', text: 'hello' },
				{
					type: 'tool_call',
					id: 'c1',
					function: { name: 'search', arguments: { q: 'x' } },
				},
				{
					type: 'tool_call',
					id: 'c2',
					function: { name: 'read', arguments: '{"id": 1}' },
				},
			],
		},
		{
			role: 'user',
			content: [
				{
					type: 'tool_result',
					call: 'c1',
					name: 'search',
					content: [{ type: 'text', text: 'result' }],
				},
			],
		},
	];

	it('maps all stored block kinds with stringified vs passthrough args', () => {
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
		expect(items[4]).toMatchObject({ name: 'read', args: '{"id": 1}' });
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
			textDelta('live '),
			textDelta('text'),
			completeToolCall('a', 'search', { q: 1 }),
			{
				type: 'toolExecutionCommitted',
				internal_call_id: 'a',
				tool_call: {
					id: 'call_1',
					function: { name: 'search', arguments: {} },
				},
			},
			completeToolCall('b', 'read', {}),
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
			{ role: 'assistant', id: null, content: [{ type: 'text', text: 'old' }] },
		];
		const live = fold([
			textDelta('first'),
			toolCallDelta('a', { Name: 'search' }),
			textDelta('second'),
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
			id: null,
			content: [
				{ type: 'text', text: 'before' },
				{ type: 'image', image_url: 'https://example.invalid/x.png' },
			],
		} as unknown as RigMessage;
		const items = buildTranscript([message], emptyLiveRun());
		expect(items).toHaveLength(1);
		expect(items[0]).toMatchObject({ kind: 'assistantText', text: 'before' });
	});
});
