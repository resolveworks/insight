/**
 * Presentation derivations over the stored transcript: pair tool calls with
 * their results and turn activity items into labelled, summarised steps.
 */
import type { TranscriptItem } from '#lib/types/chat.js';

/** One reasoning step of an activity trail. */
export interface ReasoningStep {
	kind: 'reasoning';
	key: string;
	text: string;
	streaming: boolean;
	/** Reasoning collapsed to a single line for the summary. */
	preview: string;
}

/** One tool call of an activity trail, paired with its result. */
export interface ToolStep {
	kind: 'tool';
	key: string;
	name: string;
	/** Verb and human-readable arguments, e.g. `Search “climate budget”`. */
	label: string;
	/** Short output summary when the tool's output shape provides one. */
	hint: string | null;
	/** Raw argument text, shown when it does not parse as a JSON object. */
	argsText: string;
	argsParseable: boolean;
	argsLines: ArgumentLine[];
	/** A string value was cut for display; the full value is `argsText`. */
	argsTruncated: boolean;
	resultText: string | null;
	isError: boolean;
	executing: boolean;
}

export type ActivityStep = ReasoningStep | ToolStep;

/** A line of a tool call's expanded argument body. */
export interface ArgumentLine {
	key: string;
	value: string;
}

export type TranscriptBlock =
	| { kind: 'item'; key: string; item: TranscriptItem }
	| { kind: 'activity'; key: string; steps: ActivityStep[] };

type ActivityEntry = ReasoningStep | ToolDraft;

interface ToolDraft {
	kind: 'tool';
	key: string;
	name: string;
	argsText: string;
	resultText: string | null;
	isError: boolean;
}

interface ActivityBlockDraft {
	kind: 'activity';
	key: string;
	entries: ActivityEntry[];
}

interface ItemBlock {
	kind: 'item';
	key: string;
	item: TranscriptItem;
}

type PendingBlock = ItemBlock | ActivityBlockDraft;

const MAX_ARG_VALUE = 160;

/** Collapse reasoning to one line for the summary. */
export function reasoningPreview(text: string): string {
	return text.replace(/\s+/g, ' ').trim();
}

/** `read_chunk` → `Read chunk`: capitalise the first word, lower the rest. */
export function sentenceCase(name: string): string {
	return name
		.split('_')
		.filter(Boolean)
		.map((word, index) =>
			index === 0
				? word.charAt(0).toUpperCase() + word.slice(1)
				: word.toLowerCase(),
		)
		.join(' ');
}

function parseJson(text: string): unknown {
	try {
		return JSON.parse(text);
	} catch {
		return undefined;
	}
}

function asObject(value: unknown): Record<string, unknown> | null {
	if (value === null || typeof value !== 'object' || Array.isArray(value)) {
		return null;
	}
	return value as Record<string, unknown>;
}

/** Render an argument value on the one-line summary. */
function summaryValue(value: unknown): string {
	return typeof value === 'string' ? value : JSON.stringify(value);
}

/** `Unknown tool: key: value, key: value`. */
function pairsLabel(name: string, args: Record<string, unknown>): string {
	const pairs = Object.entries(args).map(
		([key, value]) => `${key}: ${summaryValue(value)}`,
	);
	const verb = sentenceCase(name);
	return pairs.length === 0 ? verb : `${verb}: ${pairs.join(', ')}`;
}

/** The human-readable verb and arguments for a tool call. */
export function toolLabel(name: string, argsText: string): string {
	const parsed = parseJson(argsText);
	const args = asObject(parsed);

	// A tool's arguments are a JSON object; other valid JSON carries no pairs.
	if (parsed !== undefined && args === null) {
		return sentenceCase(name);
	}

	if (args !== null) {
		switch (name) {
			case 'search':
				if (typeof args.query === 'string') {
					return `Search “${args.query}”`;
				}
				return pairsLabel(name, args);

			case 'read_chunk':
				if (typeof args.chunk_index === 'number') {
					return `Read passage ${args.chunk_index}`;
				}
				return pairsLabel(name, args);

			case 'list_documents':
				return 'List documents';
		}
		return pairsLabel(name, args);
	}

	const raw = argsText.trim();
	return raw ? `${sentenceCase(name)}: ${raw}` : sentenceCase(name);
}

function plural(count: number, noun: string): string {
	return `${count} ${noun}${count === 1 ? '' : 's'}`;
}

/** A muted output summary, only when the tool's output shape gives one. */
export function resultHint(
	name: string,
	resultText: string,
	isError: boolean,
): string | null {
	if (isError) return null;

	const result = asObject(parseJson(resultText));
	if (result === null) return null;

	if (name === 'search' && Array.isArray(result.hits)) {
		return plural(result.hits.length, 'passage');
	}
	if (name === 'list_documents' && typeof result.total === 'number') {
		return plural(result.total, 'document');
	}
	return null;
}

/**
 * Split a tool call's arguments into key/value lines. Long string values are
 * cut on the summary; `truncated` tells the renderer to show the full text.
 */
export function argumentLines(argsText: string): {
	lines: ArgumentLine[];
	truncated: boolean;
	parseable: boolean;
} {
	const args = asObject(parseJson(argsText));
	if (args === null) {
		return { lines: [], truncated: false, parseable: false };
	}

	let truncated = false;
	const lines = Object.entries(args).map(([key, value]) => {
		if (typeof value === 'string' && value.length > MAX_ARG_VALUE) {
			truncated = true;
			return { key, value: `${value.slice(0, MAX_ARG_VALUE)}…` };
		}
		return {
			key,
			value: typeof value === 'string' ? value : JSON.stringify(value, null, 2),
		};
	});

	return { lines, truncated, parseable: true };
}

/** Pretty-print a JSON result; leave anything else as it arrived. */
export function formatResult(resultText: string): string {
	const value = parseJson(resultText);
	return value === undefined ? resultText : JSON.stringify(value, null, 2);
}

function toolStep(draft: ToolDraft): ToolStep {
	const args = argumentLines(draft.argsText);
	return {
		kind: 'tool',
		key: draft.key,
		name: draft.name,
		label: toolLabel(draft.name, draft.argsText),
		hint:
			draft.resultText === null
				? null
				: resultHint(draft.name, draft.resultText, draft.isError),
		argsText: draft.argsText,
		argsParseable: args.parseable,
		argsLines: args.lines,
		argsTruncated: args.truncated,
		resultText: draft.resultText,
		isError: draft.isError,
		executing: draft.resultText === null,
	};
}

function finalizeBlock(block: PendingBlock): TranscriptBlock {
	if (block.kind === 'item') return block;

	return {
		kind: 'activity',
		key: block.key,
		steps: block.entries.map((entry) =>
			entry.kind === 'reasoning' ? entry : toolStep(entry),
		),
	};
}

/**
 * Turn the chronological transcript into renderable blocks: prose items on
 * their own and consecutive reasoning/tool items as one activity trail.
 * Tool results attach FIFO to the pending call of the same name.
 */
export function transcriptBlocks(items: TranscriptItem[]): TranscriptBlock[] {
	const blocks: PendingBlock[] = [];
	const pending = new Map<string, ToolDraft[]>();
	let activity: ActivityBlockDraft | null = null;

	const flush = () => {
		if (activity !== null) {
			blocks.push(activity);
			activity = null;
		}
	};

	const trail = (key: string): ActivityBlockDraft => {
		if (activity === null) {
			activity = { kind: 'activity', key, entries: [] };
		}
		return activity;
	};

	for (const item of items) {
		switch (item.kind) {
			case 'user_text':
			case 'assistant_text':
				flush();
				blocks.push({ kind: 'item', key: item.key, item });
				break;

			case 'reasoning':
				trail(item.key).entries.push({
					kind: 'reasoning',
					key: item.key,
					text: item.text,
					streaming: item.streaming,
					preview: reasoningPreview(item.text),
				});
				break;

			case 'tool_call': {
				const draft: ToolDraft = {
					kind: 'tool',
					key: item.key,
					name: item.name,
					argsText: item.text,
					resultText: null,
					isError: false,
				};
				trail(item.key).entries.push(draft);
				const queue = pending.get(item.name) ?? [];
				queue.push(draft);
				pending.set(item.name, queue);
				break;
			}

			case 'tool_result': {
				const draft = pending.get(item.name)?.shift();
				if (draft === undefined) {
					throw new Error(
						`Tool result "${item.name}" has no matching tool call`,
					);
				}
				draft.resultText = item.text;
				draft.isError = item.is_error;
				break;
			}
		}
	}

	flush();
	return blocks.map(finalizeBlock);
}
