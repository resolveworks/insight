<script lang="ts">
	import Chat from '$lib/components/Chat.svelte';
	import CollectionFilter from '$lib/components/CollectionFilter.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';
	import FilterSidebar from '$lib/components/FilterSidebar.svelte';
	import LoadingSpinner from '$lib/components/LoadingSpinner.svelte';
	import SetupScreen from '$lib/components/SetupScreen.svelte';
	import { isLoaded, isProviderReady } from '$lib/stores/provider-state.svelte';
	import * as collectionsStore from '$lib/stores/collections.svelte';
	import * as chat from '$lib/stores/conversations.svelte';

	const loaded = $derived(isLoaded());
	const providerReady = $derived(isProviderReady());

	const collections = $derived(collectionsStore.getCollections());
	const selected = $derived(chat.getActiveCollections());

	async function handleAdd(id: string) {
		const found = collectionsStore.getCollection(id);
		if (found) await chat.addActiveCollection(found);
	}
</script>

{#if !loaded}
	<!-- Provider state unknown: minimal spinner pane -->
	<div class="flex h-full items-center justify-center bg-surface">
		<LoadingSpinner size="lg" />
	</div>
{:else if !providerReady}
	<!-- First-run setup: focused provider configuration screen -->
	<SetupScreen />
{:else}
	<!-- Normal 3-column chat -->
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
{/if}
