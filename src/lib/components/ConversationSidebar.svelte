<script lang="ts">
	import type { ConversationSummary } from '#lib/api/conversation.js';
	import Button from './Button.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';

	type Props = {
		conversations: ConversationSummary[];
		activeId: string | null;
		onSelect?: (id: string) => void;
		onNew?: () => void;
		onDelete?: (id: string) => void;
	};

	let { conversations, activeId, onSelect, onNew, onDelete }: Props = $props();

	// Conversation pending delete confirmation, if any.
	let pendingDelete = $state<{
		id: string;
		title: string;
	} | null>(null);

	function confirmDelete() {
		if (pendingDelete) onDelete?.(pendingDelete.id);
		pendingDelete = null;
	}
</script>

<div class="flex h-full flex-col">
	<div class="border-b border-line p-3">
		<Button variant="secondary" fullWidth onclick={() => onNew?.()}>
			New Chat
		</Button>
	</div>

	<div class="flex-1 overflow-y-auto p-2">
		<h3
			class="px-2 py-1 text-xs font-medium uppercase tracking-wide text-subtle"
		>
			Conversations
		</h3>

		{#if conversations.length === 0}
			<p class="px-2 py-4 text-sm italic text-subtle">No conversations yet</p>
		{:else}
			<ul class="space-y-1">
				{#each conversations as conv (conv.id)}
					<li
						class="group flex items-center rounded-sm transition-colors
							{activeId === conv.id
							? 'bg-surface-dim text-neutral-900'
							: 'text-neutral-600 hover:bg-neutral-100'}"
					>
						<button
							onclick={() => onSelect?.(conv.id)}
							aria-current={activeId === conv.id ? 'true' : undefined}
							class="min-w-0 flex-1 truncate rounded-sm px-2 py-1.5 text-left text-sm focus-visible:focus-ring"
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
							class="rounded-sm px-2 py-1.5 text-subtle transition-colors hover:text-error focus-visible:focus-ring"
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
