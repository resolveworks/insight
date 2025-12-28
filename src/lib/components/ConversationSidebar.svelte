<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';

	interface ConversationSummary {
		id: string;
		title: string;
		updated_at: string;
	}

	type Props = {
		activeConversationId: string | null;
		onSelect: (id: string) => void;
		onNew: () => void;
	};

	let { activeConversationId, onSelect, onNew }: Props = $props();

	let conversations = $state<ConversationSummary[]>([]);
	let loading = $state(true);

	export async function refresh() {
		try {
			conversations = await invoke<ConversationSummary[]>('list_conversations');
		} catch (e) {
			console.error('Failed to load conversations:', e);
		} finally {
			loading = false;
		}
	}

	$effect(() => {
		refresh();
	});
</script>

<div class="flex flex-col">
	<div class="border-b border-primary-700 p-3">
		<button
			onclick={onNew}
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

		{#if loading}
			<p class="px-2 py-4 text-sm text-primary-300">Loading...</p>
		{:else if conversations.length === 0}
			<p class="px-2 py-4 text-sm italic text-primary-300">
				No conversations yet
			</p>
		{:else}
			<ul class="space-y-1">
				{#each conversations as conv (conv.id)}
					<li>
						<button
							onclick={() => onSelect(conv.id)}
							class="w-full truncate rounded px-2 py-1.5 text-left text-sm transition
								{activeConversationId === conv.id
								? 'bg-primary-500 text-surface'
								: 'text-primary-100 hover:bg-primary-500'}"
						>
							{conv.title}
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
