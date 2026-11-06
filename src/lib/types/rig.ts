/**
 * TypeScript mirrors of the rig serde payloads the backend forwards to the
 * frontend (conversations and the `agent-event-{id}` stream).
 *
 * Sources (rig v0.46.0, from crates.io):
 * - rig-core `completion/message.rs` — `Message`, `AssistantMessage`,
 *   `AssistantContent`, `UserContent`, `ToolCall`, `ToolResult`, `Reasoning`
 * - rig-core `completion/message/identity.rs` — `CallId`, `ProviderCallId`
 * - rig-core `completion/message/native.rs` — `Origin`, `StopReason`, `Native`
 * - rig-core `completion/request.rs` — `Usage`, `FinishReason`
 * - rig-core `streaming/event.rs` — `Item<StreamEvent>`, `StreamEvent`,
 *   `Part`, `PartKind` (both serde-tagged by `item` / `event`)
 * - rig-agent `agent/streaming.rs` — `MultiTurnStreamItem`
 *   (`#[serde(tag = "type", rename_all = "camelCase")]`)
 * - rig-agent `run/response.rs` — `PromptResponse`, `CompletionCall`
 *
 * Key serde facts encoded here:
 * - `Message` is tagged on `role` (lowercase); the assistant variant is an
 *   `AssistantMessage` whose fields inline beside the role (`content`, plus
 *   optional `origin` / `stop`).
 * - `AssistantContent`/`UserContent`/`ToolResultContent` are tagged on
 *   `"type"` (lowercase) with struct fields inlined.
 * - `CallId` is externally tagged: `{"provider": "call_1"}` or
 *   `{"local": "<uuid>"}`.
 * - `Item<StreamEvent>` is adjacently tagged: `{"item": "event", "value": …}`
 *   or `{"item": "unknown", "value": …}`; `StreamEvent` is tagged on `event`.
 * - `Usage` only serializes counters the provider reported; unreported ones
 *   are absent rather than zero.
 * - `PromptResponse`/`CompletionCall` use snake_case keys (no `rename_all`
 *   on those types).
 */

// =============================================================================
// Content primitives
// =============================================================================

/** The provider item an assistant block was decoded from (rig `Native`). */
export interface RigNative {
	item: unknown;
	fingerprint: string;
	baseline?: unknown;
	edited?: boolean;
	blob_slot?: string;
}

/** rig `Text` (struct). */
export interface RigText {
	text: string;
	citations?: unknown;
	native?: RigNative;
}

/** rig `CallId` — externally tagged provider or rig-issued identity. */
export type RigCallId = { provider: string } | { local: string };

/** rig `ToolFunction` (struct; arguments are always a JSON object). */
export interface RigToolFunction {
	name: string;
	arguments: Record<string, unknown>;
	invalid_arguments?: string;
}

/** rig `ToolCall` (struct). */
export interface RigToolCall {
	id: RigCallId;
	function: RigToolFunction;
	native?: RigNative;
}

/** rig `Reasoning` (struct: text, redaction flag and provider item). */
export interface RigReasoning {
	text: string;
	redacted?: boolean;
	native?: RigNative;
}

/** rig `ToolResultContent` — tagged on `"type"` (lowercase). */
export type RigToolResultContent =
	| { type: 'text'; text: string }
	| { type: 'image'; [key: string]: unknown }
	| { type: 'json'; value: unknown };

/** rig `ToolResult` (struct). */
export interface RigToolResult {
	call: RigCallId;
	name: string;
	content: RigToolResultContent[];
	is_error?: boolean;
}

/** rig `Origin` — where an assistant turn came from. */
export interface RigOrigin {
	api: string;
	provider: string;
	model: string;
	response_model?: string;
	response_id?: string;
	context?: string;
}

/** rig `StopReason` — how an assistant turn ended. */
export type RigStopReason =
	'stop' | 'length' | 'tool_use' | { error: string } | { aborted: string };

// =============================================================================
// Messages (conversation history)
// =============================================================================

/** rig `AssistantContent` — tagged on `"type"` (lowercase). */
export type AssistantContentBlock =
	| ({ type: 'text' } & RigText)
	| ({ type: 'tool_call' } & RigToolCall)
	| ({ type: 'reasoning' } & RigReasoning)
	| {
			type: 'image';
			data: unknown;
			media_type?: string;
			detail?: string;
			native?: RigNative;
	  }
	| { type: 'opaque'; item: unknown; replay: boolean };

/** rig `UserContent` — tagged on `"type"` (lowercase). */
export type UserContentBlock =
	| ({ type: 'text' } & RigText)
	| ({ type: 'tool_result' } & RigToolResult)
	| {
			type: 'image';
			data: unknown;
			media_type?: string;
			detail?: string;
			native?: RigNative;
	  }
	| { type: 'audio'; data: unknown; media_type?: string }
	| {
			type: 'video';
			data: unknown;
			media_type?: string;
			additional_params?: unknown;
	  }
	| {
			type: 'document';
			data: unknown;
			media_type?: string;
			additional_params?: unknown;
	  };

/**
 * rig `Message` — tagged on `role`. The frontend contract only surfaces
 * user/assistant roles; the assistant variant is an `AssistantMessage`
 * whose content and optional origin/stop inline beside the role.
 */
export type RigMessage =
	| { role: 'user'; content: UserContentBlock[] }
	| {
			role: 'assistant';
			content: AssistantContentBlock[];
			origin?: RigOrigin;
			stop?: RigStopReason;
	  };

// =============================================================================
// Usage / completion calls
// =============================================================================

/** rig `Cost` (struct; unreported parts are absent). */
export interface RigCost {
	input?: number;
	output?: number;
	cache_read?: number;
	cache_write?: number;
	total: number;
	incomplete?: boolean;
}

/** rig `Usage` (snake_case struct fields; unreported counters are absent). */
export interface RigUsage {
	input_tokens?: number;
	output_tokens?: number;
	total_tokens?: number;
	cached_input_tokens?: number;
	cache_creation_input_tokens?: number;
	tool_use_prompt_tokens?: number;
	reasoning_tokens?: number;
	cost?: RigCost;
}

/** rig `FinishReason` — snake_case unit variants, or a provider-specific one. */
export type RigFinishReason =
	'stop' | 'length' | 'tool_calls' | 'content_filter' | { other: string };

/** rig `CompletionCall` (snake_case struct fields). */
export interface RigCompletionCall {
	call_index: number;
	usage: RigUsage;
	response_id?: string;
	provider_request_id?: string;
	finish_reason?: RigFinishReason;
	raw: unknown;
}

// =============================================================================
// PromptResponse (terminal payload of `finalResponse`)
// =============================================================================

/** rig `PromptResponse` — snake_case keys. */
export interface PromptResponse {
	usage: RigUsage;
	completion_calls?: RigCompletionCall[];
	messages: RigMessage[];
	content: AssistantContentBlock[];
}

// =============================================================================
// Provider stream events (`StreamAssistantItem` payloads)
// =============================================================================

/** rig `PartKind` — what kind of part a `start` event opened. */
export type RigPartKind =
	'text' | 'reasoning' | 'tool_call' | 'image' | 'opaque';

/** rig `StreamEvent` — tagged on `"event"` (snake_case). */
export type RigStreamEvent =
	| {
			event: 'start';
			part: number;
			kind: RigPartKind;
			name?: string;
	  }
	| { event: 'text'; part: number; text: string }
	| { event: 'reasoning'; part: number; text: string }
	| { event: 'arguments'; part: number; json: string }
	| { event: 'end'; part: number; content: AssistantContentBlock };

/** rig `Item<StreamEvent>` — adjacently tagged on `item` / `value`. */
export type RigStreamItem =
	| { item: 'event'; value: RigStreamEvent }
	| { item: 'unknown'; value: unknown };

// =============================================================================
// MultiTurnStreamItem (the `agent-event-{conversationId}` payloads)
// =============================================================================

export type StreamAssistantItem = {
	type: 'streamAssistantItem';
} & RigStreamItem;

/** A tool call committed to the run's history. */
export type ToolCallItem = {
	type: 'toolCall';
	tool_call: RigToolCall;
};

export type ToolExecutionCommitted = {
	type: 'toolExecutionCommitted';
	tool_call: RigToolCall;
};

/** A tool result committed to the run's history. */
export type ToolResultItem = {
	type: 'toolResult';
	tool_result: RigToolResult;
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
	| ToolCallItem
	| ToolExecutionCommitted
	| ToolResultItem
	| CompletionCallItem
	| ModelTurnRetried
	| FinalResponseItem
	| StreamErrorItem;
