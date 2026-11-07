import { invoke } from '@tauri-apps/api/core';
import { invalidate } from '$app/navigation';
import type { ConversationSummary, ConversationView } from '#lib/types/chat.js';

// =============================================================================
// Commands
// =============================================================================

/** List a collection's conversations, most recently updated first. */
export function listConversations(
	collectionId: string,
): Promise<ConversationSummary[]> {
	return invoke<ConversationSummary[]>('list_conversations', { collectionId });
}

/** Load a conversation's title and full transcript. */
export function loadConversation(
	collectionId: string,
	conversationId: string,
): Promise<ConversationView> {
	return invoke<ConversationView>('load_conversation', {
		collectionId,
		conversationId,
	});
}

/** Start a chat and re-read the collection's conversation list. */
export async function startChat(
	collectionId: string,
): Promise<ConversationView> {
	const conversation = await invoke<ConversationView>('start_chat', {
		collectionId,
	});
	await invalidate(`app:conversations:${collectionId}`);
	return conversation;
}

/** Delete a conversation and re-read the collection's conversation list. */
export async function deleteConversation(
	collectionId: string,
	conversationId: string,
): Promise<void> {
	await invoke('delete_conversation', { collectionId, conversationId });
	await invalidate(`app:conversations:${collectionId}`);
}
