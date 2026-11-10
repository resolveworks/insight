<script lang="ts">
	import type { ConversationSummary } from '#lib/types/chat.js';
	import Button from './Button.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import EmptyState from './EmptyState.svelte';
	import Icon from './Icon.svelte';
	import IconButton from './IconButton.svelte';

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
			<Icon name="plus" size="sm" />
			New Chat
		</Button>
	</div>

	<div class="flex-1 overflow-y-auto px-4 pb-4">
		{#if conversations.length === 0}
			<EmptyState title="No conversations yet" density="inline" />
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
						<IconButton
							label="Delete chat"
							variant="danger"
							size="sm"
							class="mr-1 shrink-0 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 group-focus-within:opacity-100"
							onclick={(e) => {
								e.stopPropagation();
								pendingDelete = {
									id: conv.id,
									title: conv.title,
								};
							}}
						>
							<Icon name="trash" size="sm" />
						</IconButton>
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
