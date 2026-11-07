/**
 * Conversations store. Owns the conversation list of the active collection,
 * the active conversation's state (messages), and the streaming chat
 * pipeline.
 *
 * The backend streams serialized rig `MultiTurnStreamItem`s on
 * `agent-event-{conversationId}`; the live assistant state (text deltas,
 * streamed tool-call arguments, reasoning, tool results) is assembled here
 * from those events. After `finalResponse` (or `streamError`) the
 * authoritative history is re-fetched via `load_conversation`.
 */
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type { MultiTurnStreamItem, RigMessage } from '#lib/types/rig.js';
import {
	applyItem,
	buildTranscript,
	emptyLiveRun,
	type LiveRun,
	type TranscriptItem,
} from '#lib/chat/transcript.js';

// =============================================================================
// Types (backend contract)
// =============================================================================

export interface ConversationView {
	id: string;
	collection_id: string;
	title: string;
	messages: RigMessage[];
}

export interface ConversationSummary {
	id: string;
	title: string;
}

// =============================================================================
// State
// =============================================================================

const STORAGE_KEY_PREFIX = 'insight:activeConversationId:';

function storageKey(collectionId: string): string {
	return `${STORAGE_KEY_PREFIX}${collectionId}`;
}

function readStoredId(collectionId: string): string | null {
	if (typeof window === 'undefined') return null;
	try {
		return localStorage.getItem(storageKey(collectionId));
	} catch {
		return null;
	}
}

let conversations = $state<ConversationSummary[]>([]);
let activeCollectionId = $state<string | null>(null);
let activeId = $state<string | null>(null);
let activeMessages = $state<RigMessage[]>([]);
let live = $state<LiveRun>(emptyLiveRun());
// Stored-message count when the current run started: slices the optimistic
// user message (and any prior run) out when adopting a finalResponse payload.
let runBase = 0;
let isGenerating = $state(false);
let isLoading = $state(false);
let listLoaded = $state(false);
let error = $state<string | null>(null);

let unlistenAgent: UnlistenFn | undefined;
let initPromise: Promise<void> | null = null;
// Synchronous send guard: blocks re-entry while a draft conversation is
// being created (before `isGenerating`/`activeId` would catch it).
let sendInFlight = false;

// =============================================================================
// Internal helpers
// =============================================================================

function persistActiveId() {
	if (typeof window === 'undefined' || !activeCollectionId) return;
	try {
		const key = storageKey(activeCollectionId);
		if (activeId) {
			localStorage.setItem(key, activeId);
		} else {
			localStorage.removeItem(key);
		}
	} catch {
		// Storage unavailable (private mode / test environments): skip.
	}
}

/** Detach the stream listener and drop the active conversation's state. */
function resetActive() {
	unlistenAgent?.();
	unlistenAgent = undefined;
	activeId = null;
	activeMessages = [];
	runBase = 0;
	live = emptyLiveRun();
	isGenerating = false;
}

/** Drop the active conversation and land in the draft state (no IPC). */
function clearActive() {
	resetActive();
	persistActiveId();
}

function adoptConversation(conv: ConversationView) {
	activeId = conv.id;
	activeMessages = conv.messages ?? [];
	runBase = conv.messages?.length ?? 0;
	live = emptyLiveRun();
}

async function attachListener(convId: string) {
	unlistenAgent?.();
	unlistenAgent = await listen<MultiTurnStreamItem>(
		`agent-event-${convId}`,
		(e) => handleAgentEvent(convId, e.payload),
	);
}

/**
 * Apply one streamed item to the live run (pure fold over `LiveRun`).
 *
 * Unknown provider payloads and usage records are ignored: they carry
 * nothing the transcript view needs. `finalResponse`/`streamError` are
 * terminal and handled by `handleAgentEvent`, not the fold.
 */
export function applyStreamItem(item: MultiTurnStreamItem): void {
	live = applyItem(live, item);
}

async function handleAgentEvent(convId: string, payload: MultiTurnStreamItem) {
	// Events may arrive for a stale listener right after switching chats.
	if (convId !== activeId) return;

	switch (payload.type) {
		case 'finalResponse': {
			// The event carries the run's authoritative transcript (including
			// the user message): adopt it synchronously so Svelte swaps
			// provisional -> committed without a blank frame. The silent reload
			// afterwards is pure reconciliation (meta/title/authority).
			isGenerating = false;
			activeMessages = [
				...activeMessages.slice(0, runBase),
				...payload.messages,
			];
			live = emptyLiveRun();
			await loadById(convId, { silent: true });
			refreshList();
			break;
		}

		case 'streamError': {
			error = payload.message || 'Unknown error';
			console.error('Agent stream error:', payload.message);
			isGenerating = false;
			live = emptyLiveRun();
			await loadById(convId, { silent: true });
			refreshList();
			break;
		}

		default:
			applyStreamItem(payload);
			break;
	}
}

async function refreshList() {
	const collectionId = activeCollectionId;
	if (!collectionId) return;
	try {
		conversations = await invoke<ConversationSummary[]>('list_conversations', {
			collectionId,
		});
	} catch (e) {
		console.error('Failed to list conversations:', e);
	} finally {
		listLoaded = true;
	}
}

async function loadById(
	id: string,
	opts: { silent?: boolean } = {},
): Promise<boolean> {
	try {
		isLoading = true;
		if (!opts.silent) error = null;

		const collectionId = activeCollectionId;
		if (!collectionId) {
			throw new Error('Cannot load a conversation without a collection');
		}

		const conv = await invoke<ConversationView>('load_conversation', {
			collectionId,
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

		const collectionId = activeCollectionId;
		if (!collectionId) {
			throw new Error('Cannot start a chat without a collection');
		}

		const conv = await invoke<ConversationView>('start_chat', {
			collectionId,
		});

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
 * Bind the store to a collection and open its initial conversation.
 * Idempotent per collection. Switching collections resets all per-conversation
 * state, then opens the stored conversation if it is still in the list, else
 * the most recent one, else the draft state (nothing is persisted until the
 * first message is sent).
 */
export async function initialize(collectionId: string): Promise<void> {
	if (activeCollectionId === collectionId) {
		await initPromise;
		return;
	}
	if (initPromise) {
		await initPromise;
		if (activeCollectionId === collectionId) return;
	}

	initPromise = (async () => {
		resetActive();
		activeCollectionId = collectionId;
		conversations = [];
		error = null;
		listLoaded = false;

		await refreshList();

		const stored = readStoredId(collectionId);
		if (stored && conversations.some((c) => c.id === stored)) {
			if (await loadById(stored, { silent: true })) return;
		}
		if (conversations.length > 0) {
			await loadById(conversations[0].id, { silent: true });
			return;
		}
		// Draft state: activeId stays null; the conversation is created on the
		// first sendMessage.
		persistActiveId();
	})();

	try {
		await initPromise;
	} finally {
		initPromise = null;
	}
}

/**
 * Fetch the conversation list of the active collection. Safe to call before a
 * chat provider is configured — listing doesn't require one, only creating a
 * new chat does.
 */
export async function loadList(): Promise<void> {
	await refreshList();
}

/** Load a conversation by ID (called when the user clicks a history item). */
export async function selectConversation(id: string): Promise<void> {
	if (id === activeId) return;
	await loadById(id);
}

/**
 * Start a new draft conversation (called by the "New Chat" button). Local
 * only: no IPC — the conversation is persisted on the first sendMessage.
 */
export function newConversation(): void {
	clearActive();
}

/**
 * Delete a conversation. Optimistically removes it from the sidebar; on
 * failure the list is restored. If the deleted chat was active, the next
 * most-recent chat is opened, or the draft state is entered if none remain.
 */
export async function deleteConversation(id: string): Promise<boolean> {
	const collectionId = activeCollectionId;
	if (!collectionId) return false;

	const previous = conversations;
	const wasActive = activeId === id;
	conversations = conversations.filter((c) => c.id !== id);

	try {
		await invoke('delete_conversation', { collectionId, conversationId: id });
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
		}
		// Otherwise: stay in draft state (already set by clearActive).
	}
	return true;
}

/** Send a user message to the active conversation. */
export async function sendMessage(text: string): Promise<void> {
	const trimmed = text.trim();
	if (!trimmed || isGenerating || sendInFlight) return;
	sendInFlight = true;
	try {
		// Draft state: persist the conversation in the active collection now.
		if (!activeId) {
			if (!(await createNew())) return;
		}

		const convId = activeId!;
		error = null;
		runBase = activeMessages.length;
		activeMessages = [
			...activeMessages,
			{ role: 'user', content: [{ type: 'text', text: trimmed }] },
		];
		isGenerating = true;
		live = emptyLiveRun();

		try {
			await invoke('send_message', {
				conversationId: convId,
				message: trimmed,
			});
		} catch (e) {
			error = `Failed to send message: ${e}`;
			console.error('Failed to send message:', e);
			isGenerating = false;
		}
	} finally {
		sendInFlight = false;
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
	const collectionId = activeCollectionId;
	if (!activeId || !collectionId) return null;
	try {
		return await invoke<string | null>('predict_next_message', {
			collectionId,
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

export function getActiveCollectionId(): string | null {
	return activeCollectionId;
}

export function getActiveId(): string | null {
	return activeId;
}

export function getActiveMessages(): RigMessage[] {
	return activeMessages;
}

/** Stored history + live run as one flat renderable list. */
export function getTranscript(): TranscriptItem[] {
	return buildTranscript(activeMessages, live);
}

export function getIsGenerating(): boolean {
	return isGenerating;
}

export function getIsLoading(): boolean {
	return isLoading;
}

export function getIsListLoaded(): boolean {
	return listLoaded;
}

export function getError(): string | null {
	return error;
}
