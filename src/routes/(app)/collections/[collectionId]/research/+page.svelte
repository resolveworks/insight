<script lang="ts">
	import type { PageProps } from './$types';
	import Chat from '#lib/components/Chat.svelte';
	import CollectionHeader from '#lib/components/CollectionHeader.svelte';
	import ConversationSidebar from '#lib/components/ConversationSidebar.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import SplitPane from '#lib/components/SplitPane.svelte';
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

<div class="flex h-full flex-col bg-neutral-50">
	<CollectionHeader {collection} active="research" />
	<SplitPane>
		{#snippet sidebar()}
			<!-- This collection's conversation history, always local -->
			{#if activeCollectionId === collectionId}
				<ConversationSidebar
					{conversations}
					{activeId}
					onSelect={(id) => chat.selectConversation(id)}
					onNew={() => chat.newConversation()}
					onDelete={handleDelete}
				/>
			{:else}
				<div class="flex h-full items-center justify-center text-neutral-400">
					<LoadingSpinner size="lg" ariaLabel="Loading conversations" />
				</div>
			{/if}
		{/snippet}

		<!-- Chat points at Settings when no model is configured -->
		{#if activeCollectionId !== collectionId || !collection}
			<div class="flex h-full items-center justify-center text-neutral-400">
				<LoadingSpinner size="lg" ariaLabel="Loading collection" />
			</div>
		{:else}
			<Chat {collection} {providerReady} />
		{/if}
	</SplitPane>
</div>
