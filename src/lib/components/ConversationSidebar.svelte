<script lang="ts">
	import type { ConversationSummary } from '#lib/types/chat.js';
	import Button from './Button.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import Icon from './Icon.svelte';
	import List from './List.svelte';
	import ListRow from './ListRow.svelte';

	type Props = {
		conversations: ConversationSummary[];
		activeId: string | null;
		onSelect: (id: string) => void;
		onNew: () => void;
		onDelete: (id: string) => void;
	};

	let { conversations, activeId, onSelect, onNew, onDelete }: Props = $props();

	// Conversation pending delete confirmation, if any.
	let pendingDelete = $state<{
		id: string;
		title: string;
	} | null>(null);

	function confirmDelete() {
		if (pendingDelete) onDelete(pendingDelete.id);
		pendingDelete = null;
	}
</script>

<div class="flex h-full flex-col">
	<div class="px-4 pt-4 pb-2">
		<Button variant="ghost" fullWidth onclick={() => onNew()}>
			<Icon name="plus" size="sm" />
			New Chat
		</Button>
	</div>

	<div class="flex-1 overflow-y-auto px-4 pb-4">
		{#if conversations.length === 0}
			<p class="px-2 py-1.5 text-sm text-neutral-500">No conversations yet</p>
		{:else}
			<List>
				{#each conversations as conv (conv.id)}
					<ListRow
						variant="nav"
						active={activeId === conv.id}
						onclick={() => onSelect(conv.id)}
					>
						{conv.title}
						{#snippet actions()}
							<Button
								size="icon"
								variant="ghost"
								label="Delete conversation"
								onclick={() => {
									pendingDelete = {
										id: conv.id,
										title: conv.title,
									};
								}}
							>
								<Icon name="trash" size="sm" />
							</Button>
						{/snippet}
					</ListRow>
				{/each}
			</List>
		{/if}
	</div>
</div>

{#if pendingDelete}
	<ConfirmDialog
		title="Delete conversation"
		confirmLabel="Delete conversation"
		message={`Delete "${pendingDelete.title}"? This conversation's history will be removed.`}
		onConfirm={confirmDelete}
		onCancel={() => (pendingDelete = null)}
	/>
{/if}
