<script lang="ts">
	import { onMount } from 'svelte';
	import * as chat from '$lib/stores/conversations.svelte';

	type Props = {
		onSelect?: (id: string) => void;
		onNew?: () => void;
	};

	let { onSelect, onNew }: Props = $props();

	const conversations = $derived(chat.getConversations());
	const activeId = $derived(chat.getActiveId());
	const listLoaded = $derived(chat.getIsListLoaded());

	// The list is independent of provider configuration; fetch it as soon as
	// the sidebar mounts so users see their history even before picking a
	// model (or while one is still downloading).
	onMount(() => {
		chat.loadList();
	});

	function handleDelete(event: MouseEvent, id: string) {
		event.stopPropagation();
		chat.deleteConversation(id);
	}
</script>

<div class="flex flex-col">
	<div class="border-b border-primary-700 p-3">
		<button
			onclick={() => onNew?.()}
			class="w-full rounded-md bg-secondary-400 px-3 py-2 text-sm font-medium text-neutral-800 hover:bg-secondary-500"
		>
			New Chat
		</button>
	</div>

	<div class="flex-1 overflow-y-auto p-2">
		<h3
			class="px-2 py-1 text-xs font-medium uppercase tracking-wide text-primary-300"
		>
			History
		</h3>

		{#if !listLoaded}
			<p class="px-2 py-4 text-sm text-primary-300">Loading...</p>
		{:else if conversations.length === 0}
			<p class="px-2 py-4 text-sm italic text-primary-300">
				No conversations yet
			</p>
		{:else}
			<ul class="space-y-1">
				{#each conversations as conv (conv.id)}
					<li
						class="group flex items-center rounded transition
							{activeId === conv.id
							? 'bg-primary-500 text-surface'
							: 'text-primary-100 hover:bg-primary-500'}"
					>
						<button
							onclick={() => onSelect?.(conv.id)}
							class="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm"
						>
							{conv.title}
						</button>
						<button
							onclick={(e) => handleDelete(e, conv.id)}
							class="hidden px-2 py-1.5 text-primary-200 hover:text-error group-hover:block"
							title="Delete chat"
							aria-label="Delete chat"
						>
							x
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
