<script lang="ts">
	import { onMount } from 'svelte';
	import * as chat from '#lib/stores/conversations.svelte.js';
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

<div class="flex h-full flex-col">
	<div class="border-b border-neutral-200 p-3">
		<button
			onclick={() => onNew?.()}
			class="w-full rounded-md border border-neutral-300 bg-surface-bright px-3 py-2 text-sm font-medium text-neutral-700 transition-colors hover:border-neutral-400 hover:text-neutral-900"
		>
			New Chat
		</button>
	</div>

	<div class="flex-1 overflow-y-auto p-2">
		<h3
			class="px-2 py-1 text-xs font-medium uppercase tracking-wide text-neutral-400"
		>
			Conversations
		</h3>

		{#if !listLoaded}
			<p class="px-2 py-4 text-sm text-neutral-400">Loading...</p>
		{:else if conversations.length === 0}
			<p class="px-2 py-4 text-sm italic text-neutral-400">
				No conversations yet
			</p>
		{:else}
			<ul class="space-y-1">
				{#each conversations as conv (conv.id)}
					<li
						class="group flex items-center rounded transition-colors
							{activeId === conv.id
							? 'bg-neutral-200 text-neutral-900'
							: 'text-neutral-600 hover:bg-neutral-100'}"
					>
						<button
							onclick={() => onSelect?.(conv.id)}
							aria-current={activeId === conv.id ? 'true' : undefined}
							class="min-w-0 flex-1 truncate rounded px-2 py-1.5 text-left text-sm focus-visible:ring-2 focus-visible:ring-primary-400 focus-visible:outline-none"
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
							class="rounded px-2 py-1.5 text-neutral-400 transition-colors hover:text-error focus-visible:ring-2 focus-visible:ring-primary-400 focus-visible:outline-none"
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
