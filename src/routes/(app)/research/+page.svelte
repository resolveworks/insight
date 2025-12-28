<script lang="ts">
	import { browser } from '$app/environment';
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import Chat from '$lib/components/Chat.svelte';
	import ConversationSidebar from '$lib/components/ConversationSidebar.svelte';

	const STORAGE_KEY = 'insight:activeConversationId';

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

	// Restore active conversation from localStorage
	const storedId = browser ? localStorage.getItem(STORAGE_KEY) : null;
	let activeConversationId = $state<string | null>(storedId);

	// Persist active conversation ID changes
	$effect(() => {
		if (browser) {
			if (activeConversationId) {
				localStorage.setItem(STORAGE_KEY, activeConversationId);
			} else {
				localStorage.removeItem(STORAGE_KEY);
			}
		}
	});

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
			// Select all collections by default
			for (const c of collections) {
				selectedCollections.add(c.id);
			}
		} catch (e) {
			console.error('Failed to load collections:', e);
		}
	}

	function selectAll() {
		for (const c of collections) {
			selectedCollections.add(c.id);
		}
	}

	function selectNone() {
		selectedCollections.clear();
	}

	onMount(() => {
		loadCollections();
	});
</script>

<div class="flex h-full">
	<!-- Left Sidebar: Filters + Conversation History -->
	<aside
		class="flex w-64 flex-col border-r border-primary-700 bg-primary-600 text-surface"
	>
		<!-- Collection Filters -->
		<div class="border-b border-primary-700 p-4">
			<h2
				class="mb-3 text-xs font-medium uppercase tracking-wide text-primary-200"
			>
				Filter by Collection
			</h2>
			{#if collections.length === 0}
				<p class="text-sm italic text-primary-300">No collections</p>
			{:else}
				<ul class="space-y-1">
					{#each collections as collection (collection.id)}
						<li>
							<label
								class="flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-sm hover:bg-primary-500"
							>
								<input
									type="checkbox"
									checked={selectedCollections.has(collection.id)}
									onchange={() => toggleCollection(collection.id)}
									class="h-4 w-4 rounded border-primary-400 bg-primary-700 text-secondary-400 focus:ring-secondary-400"
								/>
								<span
									class="truncate {selectedCollections.has(collection.id)
										? 'text-surface'
										: 'text-primary-100'}"
								>
									{collection.name}
								</span>
								<span class="ml-auto text-xs text-primary-300">
									{collection.document_count}
								</span>
							</label>
						</li>
					{/each}
				</ul>
				<div class="mt-3 flex gap-2 text-xs">
					<button
						onclick={selectAll}
						disabled={selectedCollections.size === collections.length}
						class="text-primary-200 hover:text-surface disabled:cursor-default disabled:text-primary-400"
					>
						Select all
					</button>
					<span class="text-primary-400">|</span>
					<button
						onclick={selectNone}
						disabled={selectedCollections.size === 0}
						class="text-primary-200 hover:text-surface disabled:cursor-default disabled:text-primary-400"
					>
						Select none
					</button>
				</div>
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
			initialConversationId={storedId}
			onConversationStart={(id) => {
				activeConversationId = id;
				conversationSidebar?.refresh();
			}}
		/>
	</div>
</div>
