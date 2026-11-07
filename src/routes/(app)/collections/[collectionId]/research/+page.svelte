<script lang="ts">
	import { page } from '$app/state';
	import Chat from '#lib/components/Chat.svelte';
	import ConversationSidebar from '#lib/components/ConversationSidebar.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import SetupScreen from '#lib/components/SetupScreen.svelte';
	import {
		isLoaded,
		isProviderReady,
	} from '#lib/stores/provider-state.svelte.js';
	import * as chat from '#lib/stores/conversations.svelte.js';

	const collectionId = $derived(page.params.collectionId ?? '');
	const loaded = $derived(isLoaded());
	const providerReady = $derived(isProviderReady());
	const activeCollectionId = $derived(chat.getActiveCollectionId());

	// The page owns initialization: bind the store to this collection once the
	// provider is known, then the sidebar and chat below render for it.
	$effect(() => {
		if (loaded && providerReady && collectionId) {
			void chat.initialize(collectionId);
		}
	});
</script>

{#if !loaded}
	<!-- Provider state unknown: minimal spinner pane -->
	<div class="flex h-full items-center justify-center bg-surface">
		<LoadingSpinner size="lg" />
	</div>
{:else if !providerReady}
	<!-- First-run setup: focused provider configuration screen -->
	<SetupScreen />
{:else if activeCollectionId !== collectionId}
	<!-- Store still bound to another collection; never render a mismatched list -->
	<div class="flex h-full items-center justify-center bg-surface">
		<LoadingSpinner size="lg" />
	</div>
{:else}
	<div class="flex h-full overflow-hidden">
		<!-- Left sidebar: this collection's conversation history -->
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
	</div>
{/if}
