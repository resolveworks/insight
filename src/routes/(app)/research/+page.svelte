<script lang="ts">
	import { SvelteSet } from 'svelte/reactivity';
	import Chat from '$lib/components/Chat.svelte';
	import CollectionFilter from '$lib/components/CollectionFilter.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';
	import FilterSidebar from '$lib/components/FilterSidebar.svelte';
	import * as collectionsStore from '$lib/stores/collections.svelte';
	import * as chat from '$lib/stores/conversations.svelte';
	import type { Collection } from '$lib/stores/collections.svelte';

	// Collection filters. Empty set means "all collections" (no filter).
	const collections = $derived(collectionsStore.getCollections());
	let selectedCollections = new SvelteSet<string>();

	const selectedCollectionInfos = $derived.by(() => {
		if (selectedCollections.size === 0) return [];
		return collections
			.filter((c) => selectedCollections.has(c.id))
			.map((c): Collection => ({ ...c }));
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
				onNew={() => chat.newConversation(selectedCollectionInfos)}
			/>
		</div>
	</aside>

	<!-- Main: Chat -->
	<div class="flex-1">
		<Chat collections={selectedCollectionInfos} />
	</div>

	<!-- Right Sidebar: Filters -->
	<FilterSidebar>
		<CollectionFilter {collections} selected={selectedCollections} />
	</FilterSidebar>
</div>
