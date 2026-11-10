<script lang="ts">
	import type { ConversationSummary } from '#lib/types/chat.js';
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
	<div class="px-4 pt-4 pb-2">
		<Button variant="ghost" fullWidth onclick={() => onNew?.()}>
			<svg
				class="mr-1.5 h-4 w-4"
				viewBox="0 0 24 24"
				fill="none"
				stroke="currentColor"
				aria-hidden="true"
			>
				<path
					stroke-linecap="round"
					stroke-linejoin="round"
					stroke-width="2"
					d="M12 4v16m8-8H4"
				/>
			</svg>
			New Chat
		</Button>
	</div>

	<div class="flex-1 overflow-y-auto px-4 pb-4">
		{#if conversations.length === 0}
			<p class="px-2 py-3 text-sm text-subtle">No conversations yet</p>
		{:else}
			<ul class="space-y-0.5">
				{#each conversations as conv (conv.id)}
					<li
						class="group flex items-center rounded-md transition-colors
							{activeId === conv.id
							? 'bg-surface-bright text-neutral-900'
							: 'text-neutral-600 hover:bg-neutral-100'}"
					>
						<button
							onclick={() => onSelect?.(conv.id)}
							aria-current={activeId === conv.id ? 'true' : undefined}
							class="min-w-0 flex-1 truncate rounded-md px-2 py-1.5 text-left text-sm focus-visible:focus-ring"
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
							class="mr-1 shrink-0 rounded-sm px-1.5 py-1.5 text-subtle opacity-0 transition-opacity hover:text-error focus-visible:opacity-100 focus-visible:focus-ring group-hover:opacity-100 group-focus-within:opacity-100"
							title="Delete chat"
							aria-label="Delete chat"
						>
							<svg
								class="h-4 w-4"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
								aria-hidden="true"
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
