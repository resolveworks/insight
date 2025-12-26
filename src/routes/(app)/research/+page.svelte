<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import Chat from '$lib/components/Chat.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';

	interface Collection {
		id: string;
		name: string;
		document_count: number;
	}

	interface CollectionInfo {
		id: string;
		name: string;
	}

	let chatComponent = $state<Chat | null>(null);
	let conversationSidebar = $state<ConversationSidebar | null>(null);
	let activeConversationId = $state<string | null>(null);

	// Collection filters
	let collections = $state<Collection[]>([]);
	let selectedCollections = new SvelteSet<string>();

	// Derive collection info for the Chat component
	let selectedCollectionInfos = $derived.by(() => {
		if (selectedCollections.size === 0) return [];
		return collections
			.filter((c) => selectedCollections.has(c.id))
			.map((c): CollectionInfo => ({ id: c.id, name: c.name }));
	});

	// Track previous selection to detect changes
	let prevSelectionKey = $state('');

	// When filter changes, start a new conversation
	$effect(() => {
		const currentKey = Array.from(selectedCollections).sort().join(',');
		if (prevSelectionKey !== '' && currentKey !== prevSelectionKey) {
			// Filter changed - start new conversation
			handleNewConversation();
		}
		prevSelectionKey = currentKey;
	});

	function toggleCollection(collectionId: string) {
		if (selectedCollections.has(collectionId)) {
			selectedCollections.delete(collectionId);
		} else {
			selectedCollections.add(collectionId);
		}
	}

	async function handleNewConversation() {
		activeConversationId = null;
		await chatComponent?.newConversation();
	}

	async function loadCollections() {
		try {
			collections = await invoke<Collection[]>('get_collections');
		} catch (e) {
			console.error('Failed to load collections:', e);
		}
	}

	onMount(() => {
		loadCollections();
	});
</script>

<div class="flex h-full">
	<!-- Left Sidebar: Filters + Conversation History -->
	<aside class="flex w-64 flex-col border-r border-slate-700 bg-slate-800">
		<!-- Collection Filters -->
		<div class="border-b border-slate-700 p-4">
			<h2
				class="mb-3 text-xs font-medium uppercase tracking-wide text-slate-400"
			>
				Filter by Collection
			</h2>
			{#if collections.length === 0}
				<p class="text-sm italic text-slate-500">No collections</p>
			{:else}
				<ul class="space-y-1">
					{#each collections as collection (collection.id)}
						<li>
							<label
								class="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-slate-700"
							>
								<input
									type="checkbox"
									checked={selectedCollections.has(collection.id)}
									onchange={() => toggleCollection(collection.id)}
									class="h-4 w-4 rounded border-slate-600 bg-slate-900 text-rose-500 focus:ring-rose-500"
								/>
								<span
									class="truncate {selectedCollections.has(collection.id)
										? 'text-rose-400'
										: 'text-slate-300'}"
								>
									{collection.name}
								</span>
								<span class="ml-auto text-xs text-slate-500">
									{collection.document_count}
								</span>
							</label>
						</li>
					{/each}
				</ul>
				{#if selectedCollections.size > 0}
					<button
						onclick={() => selectedCollections.clear()}
						class="mt-3 text-xs text-slate-500 hover:text-slate-300"
					>
						Clear filters
					</button>
				{/if}
			{/if}
		</div>

		<!-- Conversation History -->
		<div class="flex-1 overflow-y-auto">
			<ConversationSidebar
				bind:this={conversationSidebar}
				{activeConversationId}
				onSelect={async (id) => {
					activeConversationId = id;
					await chatComponent?.loadConversation(id);
				}}
				onNew={handleNewConversation}
			/>
		</div>
	</aside>

	<!-- Main: Chat -->
	<div class="flex-1">
		<Chat
			bind:this={chatComponent}
			collections={selectedCollectionInfos}
			onConversationStart={(id) => {
				activeConversationId = id;
				conversationSidebar?.refresh();
			}}
		/>
	</div>
</div>
