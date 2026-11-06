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
	AssistantContentBlock,
	MultiTurnStreamItem,
	RigCallId,
	RigMessage,
	RigReasoning,
	RigStreamEvent,
	RigStreamItem,
	RigToolCall,
	RigToolFunction,
	RigToolResultContent,
	StreamAssistantItem,
} from '#lib/types/rig.js';

// --- Live (provisional) run state -----------------------------------------

/** A tool call being assembled from stream events inside a live assistant turn. */
export interface LiveToolCallBlock {
	type: 'tool_call';
	/** The part position rig streams the call under. */
	part: number;
	/** Provider call id; '' until the complete call arrives. */
	id: string;
	name: string;
	args: string;
	committed: boolean;
	executed: boolean;
}

export interface LiveTextBlock {
	type: 'text';
	part: number;
	text: string;
}

/** Reasoning accumulated from stream events, identified by part position. */
export interface LiveReasoningBlock {
	type: 'reasoning';
	part: number;
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

/** The display key of a rig call identity. */
function callIdString(id: RigCallId): string {
	if ('provider' in id) return id.provider;
	if ('local' in id) return id.local;
	throw new Error(`invalid rig call id: ${JSON.stringify(id)}`);
}

/**
 * Pure fold: apply one stream item to the run, returning a new run.
 *
 * Unknown provider payloads, usage records, and `finalResponse`/`streamError`
 * (handled by the store) leave the run unchanged.
 */
export function applyItem(run: LiveRun, item: MultiTurnStreamItem): LiveRun {
	switch (item.type) {
		case 'streamAssistantItem':
			return applyStreamItem(run, item);

		case 'toolCall':
			return {
				...upsertToolCall(run, item.tool_call, (block) => ({
					...block,
					id: callIdString(item.tool_call.id),
					name: item.tool_call.function.name,
					args: toolCallArgs(item.tool_call.function),
					committed: true,
				})),
				streamingText: false,
			};

		case 'toolExecutionCommitted':
			return {
				...upsertToolCall(run, item.tool_call, (block) => ({
					...block,
					id: callIdString(item.tool_call.id),
					name: item.tool_call.function.name,
					args: toolCallArgs(item.tool_call.function),
					committed: true,
					executed: true,
				})),
				streamingText: false,
			};

		case 'toolResult': {
			const block: LiveToolResultBlock = {
				type: 'tool_result',
				call: callIdString(item.tool_result.call),
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
			return discardCurrentTurn(run, item.turn);

		case 'completionCall':
		case 'finalResponse':
		case 'streamError':
		default:
			return run;
	}
}

/** Drop the retried model turn's provisional output, keeping committed segments.
 *
 * `turn` is one-based. A turn beyond the assistant segment count streamed
 * nothing (possibly across several consecutive rejected attempts); a turn
 * equal to the count drops the trailing assistant segment. */
function discardCurrentTurn(run: LiveRun, turn: number): LiveRun {
	const assistantTurns = run.messages.filter(
		(message) => message.role === 'assistant',
	).length;
	if (turn > assistantTurns) {
		return { ...run, streamingText: false };
	}
	const last = lastMessage(run);
	if (turn !== assistantTurns || last?.role !== 'assistant') {
		throw new Error(
			`model turn ${turn} retried with ${assistantTurns} assistant segments`,
		);
	}
	return { messages: run.messages.slice(0, -1), streamingText: false };
}

/** Fold one `Item<StreamEvent>` payload into the run. */
function applyStreamItem(run: LiveRun, item: StreamAssistantItem): LiveRun {
	const streamItem: RigStreamItem = item;
	if (streamItem.item === 'unknown') return run;
	return applyStreamEvent(run, streamItem.value);
}

/** Fold one provider stream event into the run. */
function applyStreamEvent(run: LiveRun, event: RigStreamEvent): LiveRun {
	switch (event.event) {
		case 'start': {
			if (event.kind === 'image' || event.kind === 'opaque') return run;
			const { messages, content } = assistantSegment(run);
			let block: LiveBlock;
			if (event.kind === 'text') {
				block = { type: 'text', part: event.part, text: '' };
			} else if (event.kind === 'reasoning') {
				block = { type: 'reasoning', part: event.part, text: '' };
			} else {
				if (event.name === undefined) {
					throw new Error(
						`tool call part ${event.part} started without a tool name`,
					);
				}
				block = {
					type: 'tool_call',
					part: event.part,
					id: '',
					name: event.name,
					args: '',
					committed: false,
					executed: false,
				};
			}
			return {
				...run,
				messages: replaceContent(messages, content, [...content, block]),
			};
		}

		case 'text':
			return updateBlock(
				run,
				event.part,
				'text',
				(block) => ({
					...block,
					text: block.text + event.text,
				}),
				true,
			);

		case 'reasoning':
			return updateBlock(run, event.part, 'reasoning', (block) => ({
				...block,
				text: block.text + event.text,
			}));

		case 'arguments':
			return updateBlock(run, event.part, 'tool_call', (block) => ({
				...block,
				args: block.args + event.json,
			}));

		case 'end':
			return applyEndEvent(run, event.part, event.content);
	}
}

/** Fold a part's finalized content, which supersedes its streamed fragments. */
function applyEndEvent(
	run: LiveRun,
	part: number,
	content: AssistantContentBlock,
): LiveRun {
	switch (content.type) {
		case 'text':
			return updateBlock(run, part, 'text', (block) => ({
				...block,
				text: content.text,
			}));
		case 'reasoning':
			return updateBlock(run, part, 'reasoning', (block) => ({
				...block,
				text: flattenReasoning(content),
			}));
		case 'tool_call':
			return updateBlock(run, part, 'tool_call', (block) => ({
				...block,
				id: callIdString(content.id),
				name: content.function.name,
				args: toolCallArgs(content.function),
			}));
		default:
			return { ...run, streamingText: false };
	}
}

/** Update the live block at `part`, which must be of `type`. */
function updateBlock<T extends LiveBlock['type']>(
	run: LiveRun,
	part: number,
	type: T,
	mutate: (block: Extract<LiveBlock, { type: T }>) => LiveBlock,
	streamingText = false,
): LiveRun {
	const last = lastMessage(run);
	if (last === undefined || last.role !== 'assistant') {
		throw new Error(`stream part ${part} arrived without an assistant turn`);
	}
	const idx = last.content.findIndex(
		(block) => block.part === part && block.type === type,
	);
	if (idx === -1) {
		throw new Error(`stream part ${part} is not an open ${type} block`);
	}
	const next = last.content.slice();
	next[idx] = mutate(last.content[idx] as Extract<LiveBlock, { type: T }>);
	return {
		...run,
		messages: replaceContent(run.messages, last.content, next),
		streamingText,
	};
}

/** Mark the live call with `call.id` as committed/executed, creating it if absent. */
function upsertToolCall(
	run: LiveRun,
	call: RigToolCall,
	mutate: (block: LiveToolCallBlock) => LiveToolCallBlock,
): LiveRun {
	const id = callIdString(call.id);
	const { messages, content } = assistantSegment(run);
	const idx = content.findIndex(
		(block) => block.type === 'tool_call' && block.id === id,
	);
	if (idx !== -1) {
		const next = content.slice();
		next[idx] = mutate(content[idx] as LiveToolCallBlock);
		return { ...run, messages: replaceContent(messages, content, next) };
	}
	const block = mutate({
		type: 'tool_call',
		part: content.length,
		id,
		name: call.function.name,
		args: '',
		committed: false,
		executed: false,
	});
	return {
		...run,
		messages: replaceContent(messages, content, [...content, block]),
	};
}

// --- Flattening helpers -----------------------------------------------------

/** Flatten a reasoning block into display text. */
export function flattenReasoning(reasoning: RigReasoning): string {
	return reasoning.text;
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

/** Stringify tool-call arguments, preserving malformed raw arguments. */
function toolCallArgs(fn: RigToolFunction): string {
	return fn.invalid_arguments ?? JSON.stringify(fn.arguments, null, 2);
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
		key: `${messageIndex}-${blockIndex}`,
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
			return [item(i, 'reasoning', { text: flattenReasoning(block) })];
		if (block.type === 'tool_call') {
			return [
				item(i, 'toolCall', {
					name: block.function.name,
					args: toolCallArgs(block.function),
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
		key: `${messageIndex}-${blockIndex}`,
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

/** Normalize stored history + live run into one flat renderable list.
 *
 * Keys are position-based over the combined list (live messages continue
 * the stored numbering) so that when a run commits and its provisional
 * messages are adopted into the stored history at the same positions, the
 * keys — and therefore the rendered DOM nodes — stay identical: committing
 * a run must not remount or collapse anything. */
export function buildTranscript(
	stored: RigMessage[],
	live: LiveRun,
): TranscriptItem[] {
	const items = [
		...stored.flatMap((message, i) => storedItems(message, i)),
		...live.messages.flatMap((message, i) =>
			liveItems(message, stored.length + i),
		),
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
