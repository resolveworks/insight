/**
 * Conversations store. Owns the active conversation's transcript and the
 * streaming chat run.
 *
 * Each send opens a Tauri Channel; the backend streams `ChatEvent`s carrying
 * app-owned `TranscriptItem`s, which the store applies to the active
 * transcript in place. After the run finishes or fails, the authoritative
 * history is re-read via `load_conversation`.
 */
import { Channel, invoke } from '@tauri-apps/api/core';
import { refreshConversations } from '#lib/invalidation.js';
import {
	deleteConversation as deleteConversationCommand,
	loadConversation,
	startChat,
} from '#lib/api/conversation.js';
import type {
	ChatEvent,
	ConversationSummary,
	ConversationView,
	TranscriptItem,
} from '#lib/types/chat.js';

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

let activeCollectionId = $state<string | null>(null);
let activeId = $state<string | null>(null);
let items = $state<TranscriptItem[]>([]);
let isGenerating = $state(false);
let isLoading = $state(false);
let error = $state<string | null>(null);

let initPromise: Promise<void> | null = null;

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

/** Drop the active conversation's state. */
function resetActive() {
	activeId = null;
	items = [];
	isGenerating = false;
}

/** Drop the active conversation and land in the draft state (no IPC). */
function clearActive() {
	resetActive();
	persistActiveId();
}

function adoptConversation(conv: ConversationView) {
	activeId = conv.id;
	items = conv.transcript;
}

/** Replace the item with the same key, or append it when absent. */
function upsertItem(item: TranscriptItem): void {
	const index = items.findIndex((existing) => existing.key === item.key);
	if (index === -1) {
		items = [...items, item];
		return;
	}
	const next = items.slice();
	next[index] = item;
	items = next;
}

/** Re-read the active conversation after a run, then refresh the list. */
function reconcile(): void {
	const conversationId = activeId;
	const collectionId = activeCollectionId;
	if (!conversationId || !collectionId) {
		throw new Error('Cannot reconcile a run without an active conversation');
	}
	void (async () => {
		await loadById(conversationId, { silent: true });
		await refreshConversations(collectionId);
	})();
}

/** Apply one streamed event to the active transcript. */
export function applyEvent(event: ChatEvent): void {
	switch (event.type) {
		case 'item_upserted':
			upsertItem(event.item);
			break;

		case 'item_removed':
			items = items.filter((item) => item.key !== event.key);
			break;

		case 'run_finished':
			items = [...items.filter((item) => !item.provisional), ...event.items];
			isGenerating = false;
			reconcile();
			break;

		case 'run_failed':
			items = items.filter((item) => !item.provisional);
			error = event.message;
			isGenerating = false;
			reconcile();
			break;
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

		const conv = await loadConversation(collectionId, id);

		adoptConversation(conv);
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

		const conv = await startChat(collectionId);

		adoptConversation(conv);
		persistActiveId();
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
export async function initialize(
	collectionId: string,
	conversations: ConversationSummary[],
): Promise<void> {
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
		error = null;

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
 * Delete a conversation. If it was active, open the next one in
 * `remainingConversations`, or enter the draft state if none remain.
 */
export async function deleteConversation(
	id: string,
	remainingConversations: ConversationSummary[],
): Promise<boolean> {
	const collectionId = activeCollectionId;
	if (!collectionId) return false;

	const wasActive = activeId === id;

	try {
		await deleteConversationCommand(collectionId, id);
	} catch (e) {
		error = `Failed to delete conversation: ${e}`;
		console.error('Failed to delete conversation:', e);
		return false;
	}

	if (wasActive) {
		clearActive();
		if (remainingConversations.length > 0) {
			await loadById(remainingConversations[0].id, { silent: true });
		}
		// Otherwise: stay in draft state (already set by clearActive).
	}
	return true;
}

/** Send a user message to the active conversation. */
export async function sendMessage(text: string): Promise<void> {
	const trimmed = text.trim();
	if (!trimmed || isGenerating) return;
	const collectionId = activeCollectionId;
	if (!collectionId) {
		throw new Error('Cannot send a message without a collection');
	}

	// The run is marked from the moment the send lands, before the draft
	// conversation exists; the run's own events clear it.
	isGenerating = true;

	// Draft state: persist the conversation in the active collection now.
	if (!activeId) {
		const created = await createNew();
		if (!created) {
			isGenerating = false;
			return;
		}
	}

	const convId = activeId!;
	error = null;

	const channel = new Channel<ChatEvent>();
	channel.onmessage = (event) => {
		if (convId !== activeId) return;
		applyEvent(event);
	};

	try {
		await invoke('send_message', {
			collectionId,
			conversationId: convId,
			message: trimmed,
			onEvent: channel,
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

/** Dismiss the current error banner. */
export function clearError(): void {
	error = null;
}

// =============================================================================
// Reactive getters
// =============================================================================

export function getActiveCollectionId(): string | null {
	return activeCollectionId;
}

export function getActiveId(): string | null {
	return activeId;
}

export function getItems(): TranscriptItem[] {
	return items;
}

export function getIsGenerating(): boolean {
	return isGenerating;
}

export function getIsLoading(): boolean {
	return isLoading;
}

export function getError(): string | null {
	return error;
}
