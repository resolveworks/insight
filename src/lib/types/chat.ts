/** App-owned chat contract: what `insight` renders and what a run streams. */

export type TranscriptKind =
	'user_text' | 'assistant_text' | 'reasoning' | 'tool_call' | 'tool_result';

/** One renderable block of a conversation. */
export interface TranscriptItem {
	/** `"<message_index>-<block_index>"` over the conversation. */
	key: string;
	kind: TranscriptKind;
	/** Tool name for tool kinds, '' otherwise. */
	name: string;
	/** The body: text, reasoning, tool-call args, or tool result. */
	text: string;
	/** Part of the in-flight run. */
	provisional: boolean;
	/** Tool result failed. */
	is_error: boolean;
	/** Tool call whose result has not arrived. */
	executing: boolean;
	/** Assistant text receiving deltas. */
	streaming: boolean;
}

export type ChatEvent =
	| { type: 'item_upserted'; item: TranscriptItem }
	| { type: 'item_removed'; key: string }
	| { type: 'run_finished'; items: TranscriptItem[] }
	| { type: 'run_failed'; message: string };

export interface ConversationView {
	id: string;
	collection_id: string;
	title: string;
	transcript: TranscriptItem[];
}

export interface ConversationSummary {
	id: string;
	title: string;
}
