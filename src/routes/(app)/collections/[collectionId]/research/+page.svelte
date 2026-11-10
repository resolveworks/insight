<script lang="ts">
	import type { PageProps } from './$types';
	import Chat from '#lib/components/Chat.svelte';
	import ConversationSidebar from '#lib/components/ConversationSidebar.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import { isProviderReady } from '#lib/api/provider.js';
	import * as chat from '#lib/stores/conversations.svelte.js';

	let { data }: PageProps = $props();

	const collection = $derived(data.collection);
	const collectionId = $derived(data.collection.id);
	const conversations = $derived(data.conversations);
	const providerReady = $derived(isProviderReady(data.provider));
	const activeCollectionId = $derived(chat.getActiveCollectionId());
	const activeId = $derived(chat.getActiveId());

	// Conversation history is local: bind it as soon as the collection is
	// known, independent of the chat provider. Only the main pane is gated.
	$effect(() => {
		void chat.initialize(collectionId, conversations);
	});

	function handleDelete(id: string) {
		void chat.deleteConversation(
			id,
			conversations.filter((conversation) => conversation.id !== id),
		);
	}
</script>

<div class="flex h-full overflow-hidden">
	<!-- Left sidebar: this collection's conversation history, always local -->
	<aside
		class="flex w-64 shrink-0 flex-col overflow-hidden border-r border-line bg-surface-dim"
	>
		{#if activeCollectionId === collectionId}
			<ConversationSidebar
				{conversations}
				{activeId}
				onSelect={(id) => chat.selectConversation(id)}
				onNew={() => chat.newConversation()}
				onDelete={handleDelete}
			/>
		{:else}
			<div class="flex h-full items-center justify-center">
				<LoadingSpinner ariaLabel="Loading conversations" />
			</div>
		{/if}
	</aside>

	<!-- Main pane: chat, which points at Settings when no model is configured -->
	<main class="min-w-0 flex-1 overflow-hidden bg-surface">
		{#if activeCollectionId !== collectionId || !collection}
			<div class="flex h-full items-center justify-center bg-surface">
				<LoadingSpinner size="lg" ariaLabel="Loading collection" />
			</div>
		{:else}
			<Chat
				{collectionId}
				collectionName={collection.name}
				documentCount={collection.document_count}
				{providerReady}
			/>
		{/if}
	</main>
</div>
