/**
 * Pure fold and normalizer for the live agent stream.
 *
 * `applyItem` accumulates rig `MultiTurnStreamItem`s into a `LiveRun` shaped
 * like the stored transcript (chronological segments: assistant turn → tool
 * results → next assistant turn), and `buildTranscript` flattens stored
 * history plus the live run into one renderable list — so live and stored
 * rendering share a single view model. No runes, no IPC, no side effects.
 */
import type {
	MultiTurnStreamItem,
	RigMessage,
	RigReasoning,
	RigToolResultContent,
	StreamAssistantItem,
} from '$lib/types/rig';
import {
	isCompleteReasoning,
	isCompleteToolCall,
	isReasoningDelta,
	isTextDelta,
	isToolCallDelta,
} from '$lib/types/rig';

// --- Live (provisional) run state -----------------------------------------

/** A tool call being assembled from stream deltas inside a live assistant turn. */
export interface LiveToolCallBlock {
	type: 'tool_call';
	/** rig's per-run correlation id: links deltas -> committed call -> result. */
	internal_call_id: string;
	/** Provider call id; '' until the complete call arrives. */
	id: string;
	name: string;
	args: string;
	committed: boolean;
	executed: boolean;
}

export interface LiveTextBlock {
	type: 'text';
	text: string;
}

/** Reasoning accumulated from stream deltas, keyed by the stream item's `id`. */
export interface LiveReasoningBlock {
	type: 'reasoning';
	key: string;
	text: string;
}

export type LiveBlock = LiveTextBlock | LiveReasoningBlock | LiveToolCallBlock;

/** A completed tool result that arrived mid-run. */
export interface LiveToolResultBlock {
	type: 'tool_result';
	call: string;
	name: string;
	text: string;
}

export type LiveMessage =
	| { role: 'assistant'; content: LiveBlock[] }
	| { role: 'user'; content: LiveToolResultBlock[] };

/** The in-flight run, shaped like the stored transcript it will become. */
export interface LiveRun {
	/** Chronological provisional transcript of the run. */
	messages: LiveMessage[];
	/** True while assistant text deltas are arriving (drives the streaming cursor). */
	streamingText: boolean;
}

/** A fresh, empty live run. */
export function emptyLiveRun(): LiveRun {
	return { messages: [], streamingText: false };
}

/** The last message, if any. */
function lastMessage(run: LiveRun): LiveMessage | undefined {
	return run.messages[run.messages.length - 1];
}

/** The last assistant segment, appending a new one if the tail is not one. */
function assistantSegment(run: LiveRun): {
	messages: LiveMessage[];
	content: LiveBlock[];
} {
	const last = lastMessage(run);
	if (last !== undefined && last.role === 'assistant') {
		return { messages: run.messages, content: last.content };
	}
	const content: LiveBlock[] = [];
	return {
		messages: [...run.messages, { role: 'assistant', content }],
		content,
	};
}

/** Replace `content` (a live array) within `messages` with an updated clone. */
function replaceContent(
	messages: LiveMessage[],
	content: LiveBlock[],
	next: LiveBlock[],
): LiveMessage[] {
	const idx = messages.findIndex(
		(m) => m.role === 'assistant' && m.content === content,
	);
	const copy = messages.slice();
	copy[idx] = { role: 'assistant', content: next };
	return copy;
}

/**
 * Pure fold: apply one stream item to the run, returning a new run.
 *
 * Unknown payloads (provider-native items rig preserves verbatim), usage
 * records, terminal records, and `finalResponse`/`streamError` (handled by
 * the store) leave the run unchanged.
 */
export function applyItem(run: LiveRun, item: MultiTurnStreamItem): LiveRun {
	switch (item.type) {
		case 'streamAssistantItem':
			return applyAssistantItem(run, item);

		case 'toolExecutionCommitted': {
			const messages = run.messages.map((message) => {
				if (message.role !== 'assistant') return message;
				return {
					...message,
					content: message.content.map((block) =>
						block.type === 'tool_call' &&
						block.internal_call_id === item.internal_call_id
							? { ...block, executed: true }
							: block,
					),
				};
			});
			return { ...run, messages };
		}

		case 'streamUserItem': {
			const block: LiveToolResultBlock = {
				type: 'tool_result',
				call: item.tool_result.call,
				name: item.tool_result.name,
				text: flattenToolResult(item.tool_result.content),
			};
			return {
				...run,
				messages: [...run.messages, { role: 'user', content: [block] }],
				streamingText: false,
			};
		}

		case 'modelTurnRetried':
			return emptyLiveRun();

		case 'completionCall':
		case 'finalResponse':
		case 'streamError':
		default:
			return run;
	}
}

/** Fold one untagged `StreamAssistantItem` payload into the run. */
function applyAssistantItem(run: LiveRun, item: StreamAssistantItem): LiveRun {
	const it = item as StreamAssistantItem;

	if (isTextDelta(it)) {
		const { messages, content } = assistantSegment(run);
		const lastBlock = content[content.length - 1];
		const next: LiveBlock[] =
			lastBlock !== undefined && lastBlock.type === 'text'
				? [
						...content.slice(0, -1),
						{ type: 'text', text: lastBlock.text + it.text },
					]
				: [...content, { type: 'text', text: it.text }];
		return {
			messages: replaceContent(messages, content, next),
			streamingText: true,
		};
	}

	if (isToolCallDelta(it)) {
		const { messages, content } = assistantSegment(run);
		const next = upsertLiveBlock(
			content,
			(block) => {
				const call = block as LiveToolCallBlock;
				return {
					...call,
					name:
						it.content.Name !== undefined
							? call.name + it.content.Name
							: call.name,
					args:
						it.content.Delta !== undefined
							? call.args + it.content.Delta
							: call.args,
				};
			},
			(block) =>
				block.type === 'tool_call' &&
				block.internal_call_id === it.internal_call_id,
			() => ({
				type: 'tool_call' as const,
				internal_call_id: it.internal_call_id,
				id: '',
				name: it.content.Name ?? '',
				args: it.content.Delta ?? '',
				committed: false,
				executed: false,
			}),
		);
		return {
			messages: replaceContent(messages, content, next),
			streamingText: false,
		};
	}

	if (isCompleteToolCall(it)) {
		const { messages, content } = assistantSegment(run);
		const args =
			typeof it.tool_call.function.arguments === 'string'
				? it.tool_call.function.arguments
				: JSON.stringify(it.tool_call.function.arguments, null, 2);
		const next = upsertLiveBlock(
			content,
			(block) => ({
				...(block as LiveToolCallBlock),
				id: it.tool_call.id,
				name: it.tool_call.function.name,
				args,
				committed: true,
			}),
			(block) =>
				block.type === 'tool_call' &&
				block.internal_call_id === it.internal_call_id,
			() => ({
				type: 'tool_call' as const,
				internal_call_id: it.internal_call_id,
				id: it.tool_call.id,
				name: it.tool_call.function.name,
				args,
				committed: true,
				executed: false,
			}),
		);
		return {
			messages: replaceContent(messages, content, next),
			streamingText: false,
		};
	}

	if (isReasoningDelta(it)) {
		const { messages, content } = assistantSegment(run);
		const next = upsertLiveBlock(
			content,
			(block) => ({
				...block,
				text: (block as LiveReasoningBlock).text + it.reasoning,
			}),
			(block) =>
				block.type === 'reasoning' &&
				(block as LiveReasoningBlock).key === it.id,
			() => ({
				type: 'reasoning' as const,
				key: it.id,
				text: it.reasoning,
			}),
		);
		return {
			messages: replaceContent(messages, content, next),
			streamingText: false,
		};
	}

	if (isCompleteReasoning(it)) {
		// A complete block supersedes the accumulated deltas for its id.
		const text = flattenReasoning(it.reasoning);
		const { messages, content } = assistantSegment(run);
		const next = upsertLiveBlock(
			content,
			() => ({ type: 'reasoning' as const, key: it.id, text }),
			(block) =>
				block.type === 'reasoning' &&
				(block as LiveReasoningBlock).key === it.id,
			() => ({ type: 'reasoning' as const, key: it.id, text }),
		);
		return {
			messages: replaceContent(messages, content, next),
			streamingText: false,
		};
	}

	// StreamFinal-shaped terminal records and unknown provider payloads.
	return run;
}

/** Upsert a live block within `content` (immutable), matched by `matches`. */
function upsertLiveBlock(
	content: LiveBlock[],
	mutate: (block: LiveBlock) => LiveBlock,
	matches: (block: LiveBlock) => boolean,
	create: () => LiveBlock,
): LiveBlock[] {
	const idx = content.findIndex(matches);
	if (idx === -1) return [...content, create()];
	const next = content.slice();
	next[idx] = mutate(next[idx]);
	return next;
}

// --- Flattening helpers -----------------------------------------------------

/** Flatten a complete reasoning block's content parts into display text. */
export function flattenReasoning(reasoning: RigReasoning): string {
	return reasoning.content
		.map((part) => {
			if (part.type === 'text') return part.content.text;
			if (part.type === 'summary') return part.content;
			return '';
		})
		.filter(Boolean)
		.join('\n');
}

/** Flatten tool result content parts into display text. */
export function flattenToolResult(content: RigToolResultContent[]): string {
	return content
		.map((part) => {
			if (part.type === 'text') return part.text;
			if (part.type === 'json') return JSON.stringify(part.value, null, 2);
			return '';
		})
		.filter(Boolean)
		.join('\n');
}

/** Stringify tool-call arguments: pass strings through, format objects. */
function toolCallArgs(args: unknown): string {
	return typeof args === 'string' ? args : JSON.stringify(args, null, 2);
}

// --- Unified view model -----------------------------------------------------

export type TranscriptKind =
	'userText' | 'assistantText' | 'reasoning' | 'toolCall' | 'toolResult';

/** One renderable block. All fields always present; kind selects which matter. */
export interface TranscriptItem {
	key: string;
	role: 'user' | 'assistant';
	kind: TranscriptKind;
	/** userText / assistantText / reasoning / toolResult text; '' for toolCall. */
	text: string;
	/** toolCall / toolResult tool name; '' otherwise. */
	name: string;
	/** toolCall argument text; '' otherwise. */
	args: string;
	/** Part of the in-flight run (drives expanded state / cursor / spinner). */
	provisional: boolean;
	/** Tool call whose execution has not been observed yet. */
	executing: boolean;
	/** Assistant text currently receiving deltas (streaming cursor). */
	streaming: boolean;
}

/** Map one stored message's content blocks to transcript items. */
function storedItems(
	message: RigMessage,
	messageIndex: number,
): TranscriptItem[] {
	const item = (
		blockIndex: number,
		kind: TranscriptKind,
		fields: Partial<TranscriptItem>,
	): TranscriptItem => ({
		key: `s${messageIndex}-${blockIndex}`,
		role: message.role,
		kind,
		text: '',
		name: '',
		args: '',
		provisional: false,
		executing: false,
		streaming: false,
		...fields,
	});
	if (message.role === 'user') {
		return message.content.flatMap(
			(block, i) =>
				block.type === 'text'
					? [item(i, 'userText', { text: block.text })]
					: block.type === 'tool_result'
						? [
								item(i, 'toolResult', {
									name: block.name,
									text: flattenToolResult(block.content),
								}),
							]
						: [], // unknown block kinds (images, …) render nothing
		);
	}
	return message.content.flatMap((block, i) => {
		if (block.type === 'text')
			return [item(i, 'assistantText', { text: block.text })];
		if (block.type === 'reasoning')
			return [
				item(i, 'reasoning', {
					text: flattenReasoning({ id: block.id, content: block.content }),
				}),
			];
		if (block.type === 'tool_call') {
			return [
				item(i, 'toolCall', {
					name: block.function.name,
					args: toolCallArgs(block.function.arguments),
				}),
			];
		}
		return []; // unknown block kinds (images, …) render nothing
	});
}

/** Map one live message's content blocks to transcript items. */
function liveItems(
	message: LiveMessage,
	messageIndex: number,
): TranscriptItem[] {
	const item = (
		blockIndex: number,
		kind: TranscriptKind,
		fields: Partial<TranscriptItem>,
	): TranscriptItem => ({
		key: `l${messageIndex}-${blockIndex}`,
		role: message.role,
		kind,
		text: '',
		name: '',
		args: '',
		provisional: true,
		executing: false,
		streaming: false,
		...fields,
	});
	if (message.role === 'user') {
		return message.content.map((block, i) =>
			item(i, 'toolResult', { name: block.name, text: block.text }),
		);
	}
	return message.content.map((block, i) => {
		if (block.type === 'text')
			return item(i, 'assistantText', { text: block.text });
		if (block.type === 'reasoning')
			return item(i, 'reasoning', { text: block.text });
		return item(i, 'toolCall', {
			name: block.name,
			args: block.args,
			executing: !block.executed,
		});
	});
}

/** Normalize stored history + live run into one flat renderable list. */
export function buildTranscript(
	stored: RigMessage[],
	live: LiveRun,
): TranscriptItem[] {
	const items = [
		...stored.flatMap((message, i) => storedItems(message, i)),
		...live.messages.flatMap((message, i) => liveItems(message, i)),
	];
	if (live.streamingText) {
		for (let i = items.length - 1; i >= 0; i--) {
			if (items[i].kind === 'assistantText') {
				items[i] = { ...items[i], streaming: true };
				break;
			}
		}
	}
	return items;
}
