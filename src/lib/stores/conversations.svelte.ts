/**
 * Conversations store. Owns the conversation list, the active conversation's
 * state (messages + active collection scope), and the streaming chat pipeline.
 * Everything chat-related that isn't pure input/view state lives here.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { Collection } from './collections.svelte';

// =============================================================================
// Types
// =============================================================================

export type ContentBlock =
	| { type: 'text'; text: string }
	| { type: 'tool_use'; id: string; name: string; arguments: object }
	| {
			type: 'tool_result';
			tool_use_id: string;
			content: string;
			is_error: boolean;
	  };

export type ChatMessageRole = 'user' | 'assistant' | 'context';
type BackendMessageRole = ChatMessageRole | 'system';

export interface ChatMessage {
	role: ChatMessageRole;
	block: ContentBlock;
}

export interface ConversationSummary {
	id: string;
	title: string;
	updated_at: string;
}

interface BackendMessage {
	role: BackendMessageRole;
	content: ContentBlock[];
}

interface Conversation {
	id: string;
	title: string;
	messages: BackendMessage[];
	created_at: string;
	updated_at: string;
	collections: Collection[];
}

type ContentDelta = { type: 'text'; text: string };

type AgentEvent =
	| { type: 'content_block_start'; data: { block: ContentBlock } }
	| { type: 'content_block_delta'; data: { delta: ContentDelta } }
	| { type: 'content_block_stop' }
	| { type: 'done' }
	| { type: 'error'; data: { message: string } };

// =============================================================================
// State
// =============================================================================

const STORAGE_KEY = 'insight:activeConversationId';

let conversations = $state<ConversationSummary[]>([]);
let activeId = $state<string | null>(null);
let activeMessages = $state<ChatMessage[]>([]);
let activeCollections = $state<Collection[]>([]);
let streamingBlocks = $state<ContentBlock[]>([]);
let isGenerating = $state(false);
let isLoading = $state(false);
let initialized = $state(false);
let error = $state<string | null>(null);

let unlistenAgent: UnlistenFn | undefined;
let initializing = false;

// =============================================================================
// Internal helpers
// =============================================================================

function persistActiveId() {
	if (typeof window === 'undefined') return;
	if (activeId) {
		localStorage.setItem(STORAGE_KEY, activeId);
	} else {
		localStorage.removeItem(STORAGE_KEY);
	}
}

function isChatMessage(
	m: BackendMessage,
): m is BackendMessage & { role: ChatMessageRole } {
	return m.role !== 'system';
}

function flattenMessages(messages: BackendMessage[]): ChatMessage[] {
	return messages
		.filter(isChatMessage)
		.flatMap((m) => m.content.map((block) => ({ role: m.role, block })));
}

function adoptConversation(conv: Conversation) {
	activeId = conv.id;
	activeMessages = flattenMessages(conv.messages);
	activeCollections = conv.collections ?? [];
	streamingBlocks = [];
}

/** Drop the active conversation: detach listener and reset per-chat state. */
function clearActive() {
	unlistenAgent?.();
	unlistenAgent = undefined;
	activeId = null;
	activeMessages = [];
	activeCollections = [];
	streamingBlocks = [];
	isGenerating = false;
	persistActiveId();
}

async function attachListener(convId: string) {
	unlistenAgent?.();
	unlistenAgent = await listen<AgentEvent>(
		`agent-event-${convId}`,
		handleAgentEvent,
	);
}

function handleAgentEvent(event: { payload: AgentEvent }) {
	const payload = event.payload;

	switch (payload.type) {
		case 'content_block_start':
			streamingBlocks = [...streamingBlocks, payload.data.block];
			break;

		case 'content_block_delta': {
			const lastIdx = streamingBlocks.length - 1;
			if (lastIdx >= 0) {
				const block = streamingBlocks[lastIdx];
				const delta = payload.data.delta;
				if (delta.type === 'text' && block.type === 'text') {
					streamingBlocks = [
						...streamingBlocks.slice(0, lastIdx),
						{ type: 'text', text: block.text + delta.text },
					];
				}
			}
			break;
		}

		case 'content_block_stop':
			break;

		case 'done': {
			const newMessages: ChatMessage[] = streamingBlocks.map((block) => ({
				role: 'assistant',
				block,
			}));
			activeMessages = [...activeMessages, ...newMessages];
			streamingBlocks = [];
			isGenerating = false;
			// Refresh list so titles/timestamps update in the sidebar.
			refreshList();
			break;
		}

		case 'error':
			error = payload.data?.message || 'Unknown error';
			console.error('Agent error:', payload.data?.message);
			isGenerating = false;
			break;
	}
}

async function refreshList() {
	try {
		conversations = await invoke<ConversationSummary[]>('list_conversations');
	} catch (e) {
		console.error('Failed to list conversations:', e);
	}
}

async function loadById(
	id: string,
	opts: { silent?: boolean } = {},
): Promise<boolean> {
	try {
		isLoading = true;
		if (!opts.silent) error = null;

		const conv = await invoke<Conversation>('load_conversation', {
			conversationId: id,
		});

		adoptConversation(conv);

		await attachListener(conv.id);
		persistActiveId();
		return true;
	} catch (e) {
		if (!opts.silent) {
			error = `Failed to load conversation: ${e}`;
		}
		console.error('Failed to load conversation:', e);
		return false;
	} finally {
		isLoading = false;
	}
}

async function createNew(): Promise<boolean> {
	try {
		isLoading = true;
		error = null;

		// Fresh conversations always start unscoped — the user picks a collection
		// via the filter bar before they can chat.
		const conv = await invoke<Conversation>('start_chat', {});

		adoptConversation(conv);

		await attachListener(conv.id);
		persistActiveId();
		await refreshList();
		return true;
	} catch (e) {
		error = `Failed to start chat: ${e}`;
		console.error('Failed to start chat:', e);
		return false;
	} finally {
		isLoading = false;
	}
}

// =============================================================================
// Public API
// =============================================================================

/**
 * Resolve the initial active conversation. Idempotent — safe to call multiple
 * times. Order: stored ID → most recent → create new.
 */
export async function ensureInitialized(): Promise<void> {
	if (initialized || initializing) return;
	initializing = true;

	try {
		const stored =
			typeof window !== 'undefined' ? localStorage.getItem(STORAGE_KEY) : null;

		await refreshList();

		if (stored && conversations.some((c) => c.id === stored)) {
			if (await loadById(stored, { silent: true })) {
				initialized = true;
				return;
			}
		}
		if (conversations.length > 0) {
			if (await loadById(conversations[0].id, { silent: true })) {
				initialized = true;
				return;
			}
		}
		await createNew();
		initialized = true;
	} finally {
		initializing = false;
	}
}

/** Load a conversation by ID (called when the user clicks a history item). */
export async function selectConversation(id: string): Promise<void> {
	if (id === activeId) return;
	await loadById(id);
}

/** Create a brand-new conversation (called by the "New Chat" button). */
export async function newConversation(): Promise<void> {
	await createNew();
}

/**
 * Delete a conversation. Optimistically removes it from the sidebar; on
 * failure the list is restored. If the deleted chat was active, the next
 * most-recent chat is opened, or a fresh one is created if none remain.
 */
export async function deleteConversation(id: string): Promise<boolean> {
	const previous = conversations;
	const wasActive = activeId === id;
	conversations = conversations.filter((c) => c.id !== id);

	try {
		await invoke('delete_conversation', { conversationId: id });
	} catch (e) {
		error = `Failed to delete conversation: ${e}`;
		console.error('Failed to delete conversation:', e);
		conversations = previous;
		return false;
	}

	if (wasActive) {
		clearActive();
		if (conversations.length > 0) {
			await loadById(conversations[0].id, { silent: true });
		} else {
			await createNew();
		}
	}
	return true;
}

/**
 * Replace the active conversation's collection scope. The backend appends a
 * breadcrumb message to the transcript when the selection actually changes,
 * so the model (and the user) can see when scope shifted.
 */
export async function setActiveCollections(cols: Collection[]): Promise<void> {
	if (!activeId) return;

	// Skip the round-trip when the id-set is already what we have locally.
	const currentIds = activeCollections.map((c) => c.id);
	if (
		cols.length === currentIds.length &&
		cols.every((c) => currentIds.includes(c.id))
	) {
		return;
	}

	try {
		const conv = await invoke<Conversation>('set_conversation_collections', {
			conversationId: activeId,
			collections: cols,
		});
		adoptConversation(conv);
	} catch (e) {
		error = `Failed to update collections: ${e}`;
		console.error('Failed to update collections:', e);
	}
}

export async function addActiveCollection(col: Collection): Promise<void> {
	await setActiveCollections([...activeCollections, col]);
}

export async function removeActiveCollection(id: string): Promise<void> {
	await setActiveCollections(activeCollections.filter((c) => c.id !== id));
}

/** Send a user message to the active conversation. */
export async function sendMessage(text: string): Promise<void> {
	const trimmed = text.trim();
	if (!activeId || !trimmed || isGenerating) return;

	error = null;
	activeMessages = [
		...activeMessages,
		{ role: 'user', block: { type: 'text', text: trimmed } },
	];
	isGenerating = true;
	streamingBlocks = [];

	try {
		await invoke('send_message', {
			conversationId: activeId,
			message: trimmed,
		});
	} catch (e) {
		error = `Failed to send message: ${e}`;
		console.error('Failed to send message:', e);
		isGenerating = false;
	}
}

/** Cancel an in-flight generation. */
export async function cancelGeneration(): Promise<void> {
	if (!activeId) return;
	try {
		await invoke('cancel_generation', { conversationId: activeId });
	} finally {
		isGenerating = false;
	}
}

/** Ask the backend for a tab-completion prediction of the user's next message. */
export async function predictNextMessage(): Promise<string | null> {
	if (!activeId) return null;
	try {
		return await invoke<string | null>('predict_next_message', {
			conversationId: activeId,
		});
	} catch (e) {
		console.error('Prediction failed:', e);
		return null;
	}
}

/** Cancel any in-flight prediction request. */
export async function cancelPrediction(): Promise<void> {
	if (!activeId) return;
	try {
		await invoke('cancel_prediction', { conversationId: activeId });
	} catch {
		// Ignore cancellation errors
	}
}

/** Dismiss the current error banner. */
export function clearError(): void {
	error = null;
}

// =============================================================================
// Reactive getters
// =============================================================================

export function getConversations(): ConversationSummary[] {
	return conversations;
}

export function getActiveId(): string | null {
	return activeId;
}

export function getActiveMessages(): ChatMessage[] {
	return activeMessages;
}

export function getActiveCollections(): Collection[] {
	return activeCollections;
}

export function getStreamingBlocks(): ContentBlock[] {
	return streamingBlocks;
}

export function getIsGenerating(): boolean {
	return isGenerating;
}

export function getIsLoading(): boolean {
	return isLoading;
}

export function getIsInitialized(): boolean {
	return initialized;
}

export function getError(): string | null {
	return error;
}
