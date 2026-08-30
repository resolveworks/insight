<script lang="ts">
	import { onMount } from 'svelte';
	import * as chat from '$lib/stores/conversations.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';

	type Props = {
		onSelect?: (id: string) => void;
		onNew?: () => void;
	};

	let { onSelect, onNew }: Props = $props();

	const conversations = $derived(chat.getConversations());
	const activeId = $derived(chat.getActiveId());
	const listLoaded = $derived(chat.getIsListLoaded());

	// Conversation pending delete confirmation, if any.
	let pendingDelete = $state<{
		id: string;
		title: string;
	} | null>(null);

	// The list is independent of provider configuration; fetch it as soon as
	// the sidebar mounts so users see their history even before picking a
	// model.
	onMount(() => {
		chat.loadList();
	});

	function confirmDelete() {
		if (pendingDelete) chat.deleteConversation(pendingDelete.id);
		pendingDelete = null;
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
							onclick={(e) => {
								e.stopPropagation();
								pendingDelete = {
									id: conv.id,
									title: conv.title,
								};
							}}
							class="px-2 py-1.5 text-primary-200 opacity-60 transition-opacity hover:opacity-100 hover:text-error"
							title="Delete chat"
							aria-label="Delete chat"
						>
							<svg
								class="h-4 w-4"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
							>
								<path
									stroke-linecap="round"
									stroke-linejoin="round"
									stroke-width="2"
									d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
								/>
							</svg>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

{#if pendingDelete}
	<ConfirmDialog
		title="Delete chat"
		message={`Delete "${pendingDelete.title}"? This conversation's history will be removed.`}
		onConfirm={confirmDelete}
		onCancel={() => (pendingDelete = null)}
	/>
{/if}
