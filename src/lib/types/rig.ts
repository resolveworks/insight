/**
 * TypeScript mirrors of the rig serde payloads the backend forwards to the
 * frontend (conversations and the `agent-event-{id}` stream).
 *
 * Sources (rig v0.42.0, from crates.io):
 * - rig-core `completion/message.rs` — `Message`, `AssistantContent`,
 *   `UserContent`, `ToolCall`, `ToolResult`, `Reasoning`
 * - rig-core `streaming/mod.rs` — `StreamedAssistantContent`,
 *   `StreamedUserContent`, `ToolCallDeltaContent`, `StreamFinal`
 *   (all `#[serde(untagged)]`, so their variants flatten into the enclosing
 *   `MultiTurnStreamItem` map)
 * - rig-agent `agent/prompt_request/streaming.rs` — `MultiTurnStreamItem`
 *   (`#[serde(tag = "type", rename_all = "camelCase")]`)
 * - rig-agent `agent/prompt_request` — `PromptResponse`, `CompletionCall`
 *   (snake_case keys)
 *
 * Key serde facts encoded here:
 * - `Message` is tagged on `role` (lowercase); assistant messages also carry
 *   an `id` that serializes even when `null`.
 * - `AssistantContent`/`UserContent`/`ToolResultContent` are tagged on
 *   `"type"` (lowercase) with struct fields inlined.
 * - `ToolCallDeltaContent` has NO serde tag: externally tagged, i.e.
 *   `{"Name": "search"}` / `{"Delta": "{\"q\""}`.
 * - `ReasoningContent` is adjacently tagged: `{"type": "text", "content": …}`.
 * - `PromptResponse`/`CompletionCall`/`Usage` use snake_case keys (no
 *   `rename_all` on those types).
 */

// =============================================================================
// Content primitives
// =============================================================================

export interface RigText {
	text: string;
	additional_params?: Record<string, unknown>;
}

/** rig `ProviderCallId` (struct, plain fields). */
export interface RigProviderCallId {
	call_id: string;
	item_id?: string;
}

/** rig `ToolCall` (struct). */
export interface RigToolCall {
	id: string;
	provider?: RigProviderCallId;
	function: { name: string; arguments: unknown };
	signature?: string | null;
	additional_params?: unknown;
}

/** rig `ReasoningContent` — adjacently tagged (`type` + `content`). */
export type RigReasoningContent =
	| { type: 'text'; content: { text: string; signature?: string } }
	| { type: 'encrypted'; content: string }
	| { type: 'redacted'; content: { data: string } }
	| { type: 'summary'; content: string };

/** rig `Reasoning` (struct). */
export interface RigReasoning {
	id: string | null;
	content: RigReasoningContent[];
}

/** rig `ToolResultContent` — tagged on `"type"` (lowercase). */
export type RigToolResultContent =
	| { type: 'text'; text: string }
	| { type: 'image'; [key: string]: unknown }
	| { type: 'json'; value: unknown };

/** rig `ToolResult` (struct). */
export interface RigToolResult {
	call: string;
	provider?: RigProviderCallId;
	name: string;
	content: RigToolResultContent[];
}

// =============================================================================
// Messages (conversation history)
// =============================================================================

/** rig `AssistantContent` — tagged on `"type"` (lowercase). */
export type AssistantContentBlock =
	| { type: 'text'; text: string; additional_params?: Record<string, unknown> }
	| {
			type: 'tool_call';
			id: string;
			function: { name: string; arguments: unknown };
			provider?: RigProviderCallId;
			signature?: string | null;
	  }
	| { type: 'reasoning'; id: string | null; content: RigReasoningContent[] }
	| { type: 'image'; [key: string]: unknown };

/** rig `UserContent` — tagged on `"type"` (lowercase). */
export type UserContentBlock =
	| { type: 'text'; text: string; additional_params?: Record<string, unknown> }
	| {
			type: 'tool_result';
			call: string;
			name: string;
			content: RigToolResultContent[];
	  }
	| { type: 'image' | 'audio' | 'video' | 'document'; [key: string]: unknown };

/**
 * rig `Message` — tagged on `role`. The frontend contract only surfaces
 * user/assistant roles; assistant messages carry `id` (serialized even when
 * null).
 */
export type RigMessage =
	| { role: 'user'; content: UserContentBlock[] }
	| { role: 'assistant'; id: string | null; content: AssistantContentBlock[] };

// =============================================================================
// Usage / completion calls
// =============================================================================

/** rig `Usage` (snake_case struct fields). */
export interface RigUsage {
	input_tokens: number;
	output_tokens: number;
	total_tokens: number;
	cached_input_tokens: number;
	cache_creation_input_tokens: number;
	tool_use_prompt_tokens?: number;
	reasoning_tokens: number;
}

/** rig `CompletionCall` (snake_case struct fields). */
export interface RigCompletionCall {
	call_index: number;
	usage: RigUsage;
	message_id?: string;
	response_id?: string;
	provider_request_id?: string;
	finish_reason?: { [key: string]: unknown };
	raw?: unknown;
}

// =============================================================================
// PromptResponse (terminal payload of `finalResponse`)
// =============================================================================

/** rig `PromptResponse` — snake_case keys. */
export interface PromptResponse {
	output: string;
	usage: RigUsage;
	completion_calls?: RigCompletionCall[];
	messages: RigMessage[] | null;
	content: AssistantContentBlock[];
}

// =============================================================================
// MultiTurnStreamItem (the `agent-event-{conversationId}` payloads)
// =============================================================================

/**
 * The provider's normalized terminal record (`StreamFinal`), flattened into
 * the stream assistant item map by the untagged serde encoding.
 */
export interface StreamFinalItem {
	kind: 'final';
	usage: RigUsage;
	finish_reason?: { [key: string]: unknown } | null;
	message_id?: string | null;
	response_id?: string | null;
	provider_request_id?: string;
	provider: string;
	model?: string | null;
	raw?: unknown;
}

/**
 * `MultiTurnStreamItem::StreamAssistantItem` — the payload is the untagged
 * `StreamedAssistantContent`, so its fields flatten beside `type`. The
 * variants are discriminated by key shape; use the guards below.
 */
export type StreamAssistantItemPayload =
	| { text: string } // Text delta
	| { tool_call: RigToolCall; internal_call_id: string } // Complete tool call
	| { internal_call_id: string; content: { Name?: string; Delta?: string } } // Tool call delta
	| { reasoning: RigReasoning; id: string } // Complete reasoning block
	| { id: string; reasoning: string; provider_id?: string } // Reasoning delta
	| StreamFinalItem // Provider terminal record (usage etc.)
	// UnknownPayload — provider-native item rig does not model
	| Record<string, unknown>;

export type StreamAssistantItem = { type: 'streamAssistantItem' } & (
	| { text: string }
	| { tool_call: RigToolCall; internal_call_id: string }
	| { internal_call_id: string; content: { Name?: string; Delta?: string } }
	| { reasoning: RigReasoning; id: string }
	| { id: string; reasoning: string; provider_id?: string }
	| StreamFinalItem
	| Record<string, unknown>
);

export type StreamUserItem = {
	type: 'streamUserItem';
	tool_result: RigToolResult;
	internal_call_id: string;
};

export type ToolExecutionCommitted = {
	type: 'toolExecutionCommitted';
	tool_call: RigToolCall;
	internal_call_id: string;
};

export type CompletionCallItem = {
	type: 'completionCall';
} & RigCompletionCall;

export type ModelTurnRetried = {
	type: 'modelTurnRetried';
	turn: number;
};

export type FinalResponseItem = {
	type: 'finalResponse';
} & PromptResponse;

/** Stream failure, emitted by the Insight backend (not rig itself). */
export type StreamErrorItem = {
	type: 'streamError';
	message: string;
};

export type MultiTurnStreamItem =
	| StreamAssistantItem
	| ToolExecutionCommitted
	| StreamUserItem
	| CompletionCallItem
	| ModelTurnRetried
	| FinalResponseItem
	| StreamErrorItem;

// =============================================================================
// Guards for the untagged stream assistant payload
// =============================================================================

export function isTextDelta(
	item: StreamAssistantItem,
): item is StreamAssistantItem & { text: string } {
	return 'text' in item && typeof item.text === 'string';
}

export function isCompleteToolCall(
	item: StreamAssistantItem,
): item is StreamAssistantItem & {
	tool_call: RigToolCall;
	internal_call_id: string;
} {
	return 'tool_call' in item && typeof item.internal_call_id === 'string';
}

export function isToolCallDelta(
	item: StreamAssistantItem,
): item is StreamAssistantItem & {
	internal_call_id: string;
	content: { Name?: string; Delta?: string };
} {
	return (
		'content' in item &&
		typeof item.internal_call_id === 'string' &&
		!('tool_call' in item)
	);
}

export function isReasoningDelta(
	item: StreamAssistantItem,
): item is StreamAssistantItem & { id: string; reasoning: string } {
	return 'reasoning' in item && typeof item.reasoning === 'string';
}

export function isCompleteReasoning(
	item: StreamAssistantItem,
): item is StreamAssistantItem & { reasoning: RigReasoning; id: string } {
	return (
		'reasoning' in item &&
		typeof item.reasoning === 'object' &&
		item.reasoning !== null
	);
}

export function isStreamFinal(
	item: StreamAssistantItem,
): item is StreamAssistantItem & StreamFinalItem {
	return 'kind' in item && (item as StreamFinalItem).kind === 'final';
}
