<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onDestroy, onMount } from 'svelte';
	import Chat from '$lib/components/Chat.svelte';
	import CollectionFilter from '$lib/components/CollectionFilter.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';
	import FilterSidebar from '$lib/components/FilterSidebar.svelte';
	import * as collectionsStore from '$lib/stores/collections.svelte';
	import * as chat from '$lib/stores/conversations.svelte';

	const collections = $derived(collectionsStore.getCollections());
	const selected = $derived(chat.getActiveCollections());

	async function handleAdd(id: string) {
		const found = collectionsStore.getCollection(id);
		if (found) await chat.addActiveCollection(found);
	}

	// Tell the backend that chat has priority while this page is mounted.
	// Embed (and later OCR) workers yield at their next job boundary so local
	// chat has uncontested VRAM. Leaving the page releases the signal so
	// background work resumes.
	onMount(() => {
		invoke('research_focus_enter').catch((e) =>
			console.error('Failed to enter research focus:', e),
		);
	});

	onDestroy(() => {
		invoke('research_focus_leave').catch((e) =>
			console.error('Failed to leave research focus:', e),
		);
	});
</script>

<div class="flex h-full">
	<!-- Left Sidebar: Conversation History -->
	<aside
		class="flex w-64 flex-col border-r border-primary-700 bg-primary-600 text-surface"
	>
		<div class="flex-1 overflow-y-auto">
			<ConversationSidebar
				onSelect={(id) => chat.selectConversation(id)}
				onNew={() => chat.newConversation()}
			/>
		</div>
	</aside>

	<!-- Main: Chat -->
	<div class="flex-1">
		<Chat />
	</div>

	<!-- Right Sidebar: Filters (bound to active conversation) -->
	<FilterSidebar>
		<CollectionFilter
			{collections}
			{selected}
			onAdd={handleAdd}
			onRemove={chat.removeActiveCollection}
		/>
	</FilterSidebar>
</div>
