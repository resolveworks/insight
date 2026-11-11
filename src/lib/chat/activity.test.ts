import { describe, it, expect } from 'vitest';
import {
	argumentLines,
	formatResult,
	reasoningPreview,
	resultHint,
	sentenceCase,
	toolLabel,
	transcriptBlocks,
	transcriptTurns,
} from './activity.js';
import type { TranscriptItem, TranscriptKind } from '#lib/types/chat.js';

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

const searchCall = (key: string, query = 'climate') =>
	item(key, 'tool_call', {
		name: 'search',
		text: JSON.stringify({ query }, null, 2),
		executing: true,
	});

const searchResult = (key: string, hits: unknown[], isError = false) =>
	item(key, 'tool_result', {
		name: 'search',
		text: JSON.stringify({ total_hits: hits.length, hits }),
		is_error: isError,
	});

describe('transcriptBlocks', () => {
	it('pairs a call with its result into a single step', () => {
		const blocks = transcriptBlocks([
			item('0-0', 'user_text', { text: 'Question' }),
			searchCall('1-0'),
			searchResult('2-0', [{}, {}]),
		]);

		expect(blocks.map((block) => block.kind)).toEqual(['item', 'activity']);
		const activity = blocks[1];
		if (activity.kind !== 'activity') throw new Error('expected activity');
		expect(activity.steps).toHaveLength(1);
		expect(activity.steps[0]).toMatchObject({
			kind: 'tool',
			key: '1-0',
			label: 'Search “climate”',
			hint: '2 documents',
			executing: false,
			isError: false,
		});
	});

	it('pairs same-name calls FIFO', () => {
		const blocks = transcriptBlocks([
			searchCall('1-0', 'first'),
			searchCall('1-1', 'second'),
			searchResult('2-0', [{}]),
			searchResult('2-1', [{}, {}, {}]),
		]);

		const activity = blocks[0];
		if (activity.kind !== 'activity') throw new Error('expected activity');
		expect(activity.steps).toHaveLength(2);
		expect(activity.steps[0]).toMatchObject({
			label: 'Search “first”',
			hint: '1 document',
		});
		expect(activity.steps[1]).toMatchObject({
			label: 'Search “second”',
			hint: '3 documents',
		});
	});

	it('keeps a call without a result executing', () => {
		const blocks = transcriptBlocks([searchCall('1-0')]);

		const activity = blocks[0];
		if (activity.kind !== 'activity') throw new Error('expected activity');
		expect(activity.steps[0]).toMatchObject({
			executing: true,
			resultText: null,
			hint: null,
		});
	});

	it('groups a contiguous reasoning and tool run into one trail', () => {
		const blocks = transcriptBlocks([
			item('1-0', 'reasoning', { text: 'Looking around', streaming: true }),
			searchCall('1-1'),
			searchResult('2-0', [{}]),
			item('3-0', 'assistant_text', { text: 'Answer' }),
		]);

		expect(blocks.map((block) => block.kind)).toEqual(['activity', 'item']);
		const activity = blocks[0];
		if (activity.kind !== 'activity') throw new Error('expected activity');
		expect(activity.steps.map((step) => step.kind)).toEqual([
			'reasoning',
			'tool',
		]);
		expect(activity.key).toBe('1-0');

		const answer = blocks[1];
		if (answer.kind !== 'item') throw new Error('expected item');
		expect(answer.item.text).toBe('Answer');
	});

	it('starts a new trail after a prose block', () => {
		const blocks = transcriptBlocks([
			searchCall('1-0'),
			searchResult('2-0', []),
			item('3-0', 'assistant_text', { text: 'Answer' }),
			searchCall('4-0', 'again'),
		]);

		expect(blocks.map((block) => block.kind)).toEqual([
			'activity',
			'item',
			'activity',
		]);
	});
});

describe('transcriptTurns', () => {
	it('splits a question from the activity and answer that follow it', () => {
		const blocks = transcriptBlocks([
			item('0-0', 'user_text', { text: 'Question' }),
			searchCall('1-0'),
			searchResult('2-0', [{}]),
			item('3-0', 'assistant_text', { text: 'Answer' }),
		]);

		const turns = transcriptTurns(blocks);

		expect(turns.map((turn) => turn.key)).toEqual(['0-0', '1-0']);
		expect(turns[0].blocks.map((block) => block.kind)).toEqual(['item']);
		expect(turns[1].blocks.map((block) => block.kind)).toEqual([
			'activity',
			'item',
		]);
	});

	it('keeps text, tool and text in one reply turn', () => {
		const blocks = transcriptBlocks([
			item('0-0', 'assistant_text', { text: 'First' }),
			searchCall('1-0'),
			searchResult('2-0', [{}]),
			item('3-0', 'assistant_text', { text: 'Second' }),
		]);

		const turns = transcriptTurns(blocks);

		expect(turns).toHaveLength(1);
		expect(turns[0].key).toBe('0-0');
		expect(turns[0].blocks).toEqual(blocks);
	});

	it('gives each of two consecutive questions its own turn', () => {
		const blocks = transcriptBlocks([
			item('0-0', 'user_text', { text: 'First question' }),
			item('1-0', 'user_text', { text: 'Second question' }),
			searchCall('2-0'),
			searchResult('3-0', [{}]),
			item('4-0', 'assistant_text', { text: 'Answer' }),
		]);

		const turns = transcriptTurns(blocks);

		expect(turns.map((turn) => turn.key)).toEqual(['0-0', '1-0', '2-0']);
		expect(turns[0].blocks.map((block) => block.key)).toEqual(['0-0']);
		expect(turns[1].blocks.map((block) => block.key)).toEqual(['1-0']);
		expect(turns[2].blocks.map((block) => block.key)).toEqual(['2-0', '4-0']);
	});

	it('puts a leading tool call in its own turn before the question', () => {
		const blocks = transcriptBlocks([
			searchCall('0-0'),
			searchResult('1-0', []),
			item('2-0', 'user_text', { text: 'Question' }),
		]);

		const turns = transcriptTurns(blocks);

		expect(turns.map((turn) => turn.key)).toEqual(['0-0', '2-0']);
		expect(turns[0].blocks.map((block) => block.kind)).toEqual(['activity']);
		expect(turns[1].blocks.map((block) => block.kind)).toEqual(['item']);
	});

	it('yields no turns for an empty transcript', () => {
		expect(transcriptTurns([])).toEqual([]);
	});

	it('preserves every block in order across turns', () => {
		const blocks = transcriptBlocks([
			item('0-0', 'user_text', { text: 'Question' }),
			item('1-0', 'reasoning', { text: 'Thinking' }),
			searchCall('1-1'),
			searchResult('2-0', [{}]),
			item('3-0', 'assistant_text', { text: 'Answer' }),
			item('4-0', 'user_text', { text: 'Follow-up' }),
			item('5-0', 'assistant_text', { text: 'Reply' }),
		]);

		const turns = transcriptTurns(blocks);

		expect(turns.flatMap((turn) => turn.blocks)).toEqual(blocks);
	});
});

describe('toolLabel', () => {
	it('labels a search with its quoted query', () => {
		expect(toolLabel('search', '{"query":"climate budget"}')).toBe(
			'Search “climate budget”',
		);
	});

	it('labels a document read generically', () => {
		expect(
			toolLabel('read_document', '{"document_id":"doc1","offset":400}'),
		).toBe('Read document');
	});

	it('labels the document listing', () => {
		expect(toolLabel('list_documents', '{}')).toBe('List documents');
	});

	it('sentence-cases an unknown tool and renders its arguments as pairs', () => {
		expect(
			toolLabel('web_fetch', '{"url":"https://example.com","depth":2}'),
		).toBe('Web fetch: url: https://example.com, depth: 2');
	});

	it('renders unparseable arguments verbatim after the tool name', () => {
		expect(toolLabel('ask_model', 'not json')).toBe('Ask model: not json');
	});
});

describe('resultHint', () => {
	it('counts search hits as documents', () => {
		expect(resultHint('search', '{"hits":[{},{}]}', false)).toBe('2 documents');
		expect(resultHint('search', '{"hits":[{}]}', false)).toBe('1 document');
	});

	it('counts the documents known to list_documents', () => {
		expect(resultHint('list_documents', '{"total":12}', false)).toBe(
			'12 documents',
		);
	});

	it('gives no hint for read_document or a failed result', () => {
		expect(resultHint('read_document', '{"content":"text"}', false)).toBeNull();
		expect(resultHint('search', '{"hits":[{}]}', true)).toBeNull();
	});

	it('gives no hint when the output has no known count', () => {
		expect(resultHint('search', 'search unavailable', false)).toBeNull();
	});
});

describe('argumentLines', () => {
	it('renders scalar arguments as key/value lines and objects pretty-printed', () => {
		const args = argumentLines('{"query":"climate","filters":{"year":2024}}');

		expect(args.parseable).toBe(true);
		expect(args.truncated).toBe(false);
		expect(args.lines).toEqual([
			{ key: 'query', value: 'climate' },
			{ key: 'filters', value: '{\n  "year": 2024\n}' },
		]);
	});

	it('truncates a long string value and flags it', () => {
		const long = 'a'.repeat(200);
		const args = argumentLines(JSON.stringify({ body: long }));

		expect(args.truncated).toBe(true);
		expect(args.lines[0].value).toHaveLength(161);
		expect(args.lines[0].value.endsWith('…')).toBe(true);
	});

	it('reports unparseable arguments instead of inventing lines', () => {
		expect(argumentLines('not json')).toEqual({
			lines: [],
			truncated: false,
			parseable: false,
		});
	});
});

describe('formatResult', () => {
	it('pretty-prints JSON', () => {
		expect(formatResult('{"hits":[]}')).toBe('{\n  "hits": []\n}');
	});

	it('leaves a non-JSON message as it arrived', () => {
		expect(formatResult('index unavailable')).toBe('index unavailable');
	});
});

describe('reasoningPreview', () => {
	it('collapses whitespace to one line', () => {
		expect(reasoningPreview('First line\n\n  second\tline ')).toBe(
			'First line second line',
		);
	});
});

describe('sentenceCase', () => {
	it('capitalises only the first word of a snake_case name', () => {
		expect(sentenceCase('list_related_documents')).toBe(
			'List related documents',
		);
	});
});
