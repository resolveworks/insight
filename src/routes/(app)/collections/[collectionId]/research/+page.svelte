<script lang="ts">
	import { page } from '$app/state';
	import Chat from '#lib/components/Chat.svelte';
	import ConversationSidebar from '#lib/components/ConversationSidebar.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import { isProviderReady } from '#lib/api/provider.js';
	import * as chat from '#lib/stores/conversations.svelte.js';

	const collectionId = $derived(page.params.collectionId ?? '');
	const collection = $derived(page.data.collection);
	const providerReady = $derived(isProviderReady(page.data.provider));
	const activeCollectionId = $derived(chat.getActiveCollectionId());

	// Conversation history is local: bind it as soon as the collection is
	// known, independent of the chat provider. Only the main pane is gated.
	$effect(() => {
		if (collectionId) {
			void chat.initialize(collectionId);
		}
	});
</script>

<div class="flex h-full overflow-hidden">
	<!-- Left sidebar: this collection's conversation history, always local -->
	<aside
		class="flex w-64 shrink-0 flex-col overflow-hidden border-r border-neutral-200 bg-surface-dim"
	>
		{#if activeCollectionId === collectionId}
			<ConversationSidebar
				onSelect={(id) => chat.selectConversation(id)}
				onNew={() => chat.newConversation()}
			/>
		{:else}
			<div class="flex h-full items-center justify-center">
				<LoadingSpinner />
			</div>
		{/if}
	</aside>

	<!-- Main pane: chat, with provider setup in its lower region if needed -->
	<main class="min-w-0 flex-1 overflow-hidden">
		{#if activeCollectionId !== collectionId || !collection}
			<div class="flex h-full items-center justify-center bg-surface">
				<LoadingSpinner size="lg" />
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
