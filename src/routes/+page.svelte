<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { onDestroy, onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import Chat from '$lib/components/Chat.svelte';

	interface Collection {
		id: string;
		name: string;
		document_count: number;
	}

	interface Document {
		id: string;
		name: string;
		page_count: number;
	}

	interface SearchHit {
		document: Document;
		collection_id: string;
		snippet: string;
		score: number;
	}

	interface SearchResponse {
		hits: SearchHit[];
		total_hits: number;
		page: number;
		page_size: number;
	}

	interface ImportResult {
		successful: Document[];
		failed: { path: string; error: string }[];
	}

	type Tab = 'trajectory' | 'search' | 'files';
	let activeTab = $state<Tab>('search');

	let searchQuery = $state('');
	let results = $state<SearchHit[]>([]);
	let totalHits = $state(0);
	let currentPage = $state(0);
	let pageSize = $state(20);
	let collections = $state<Collection[]>([]);
	let documents = $state<Document[]>([]);
	let importing = $state(false);
	let newCollectionName = $state('');
	let selectedCollection = $state<string | null>(null);
	let selectedSearchCollections = new SvelteSet<string>();
	let searching = $state(false);

	// Debounced search-as-you-type
	let searchTimeout: ReturnType<typeof setTimeout> | null = null;

	$effect(() => {
		const query = searchQuery;
		const filterIds = selectedSearchCollections;

		// Reset to first page when query or filters change
		currentPage = 0;

		// Clear previous timeout
		if (searchTimeout) {
			clearTimeout(searchTimeout);
		}

		// Debounce search by 200ms
		searchTimeout = setTimeout(() => {
			performSearch(query, filterIds, 0);
		}, 200);

		return () => {
			if (searchTimeout) clearTimeout(searchTimeout);
		};
	});

	async function performSearch(
		query: string,
		filterIds: SvelteSet<string>,
		page: number,
	) {
		if (!query.trim()) {
			results = [];
			totalHits = 0;
			return;
		}
		searching = true;
		try {
			const collectionIds = filterIds.size > 0 ? Array.from(filterIds) : null;
			const response = await invoke<SearchResponse>('search', {
				query,
				collectionIds,
				page,
				pageSize,
			});
			results = response.hits;
			totalHits = response.total_hits;
			currentPage = response.page;
		} catch (e) {
			console.error('Search failed:', e);
		} finally {
			searching = false;
		}
	}

	function goToPage(page: number) {
		currentPage = page;
		performSearch(searchQuery, selectedSearchCollections, page);
	}

	const totalPages = $derived(Math.ceil(totalHits / pageSize));

	function toggleSearchCollection(collectionId: string) {
		const newSet = new SvelteSet(selectedSearchCollections);
		if (newSet.has(collectionId)) {
			newSet.delete(collectionId);
		} else {
			newSet.add(collectionId);
		}
		selectedSearchCollections = newSet;
	}

	function getCollectionName(collectionId: string): string {
		const col = collections.find((c) => c.id === collectionId);
		return col?.name ?? 'Unknown';
	}

	async function importPdf() {
		if (!selectedCollection) {
			console.error('No collection selected');
			return;
		}

		const files = await open({
			multiple: true,
			filters: [{ name: 'PDF', extensions: ['pdf'] }],
		});

		if (!files) return;

		importing = true;
		const paths = Array.isArray(files) ? files : [files];

		try {
			// Use batch import - documents are added via events as they're processed
			const result = await invoke<ImportResult>('import_pdfs_batch', {
				paths,
				collectionId: selectedCollection,
			});

			if (result.failed.length > 0) {
				console.error('Some imports failed:', result.failed);
			}
		} catch (e) {
			console.error('Batch import failed:', e);
		}
		importing = false;
	}

	async function loadDocuments(collectionId: string) {
		try {
			documents = await invoke<Document[]>('get_documents', { collectionId });
		} catch (e) {
			console.error('Failed to load documents:', e);
			documents = [];
		}
	}

	async function selectCollection(collectionId: string | null) {
		if (selectedCollection === collectionId) {
			selectedCollection = null;
			documents = [];
		} else {
			selectedCollection = collectionId;
			if (collectionId) {
				await loadDocuments(collectionId);
			}
		}
	}

	async function createCollection() {
		if (!newCollectionName.trim()) return;
		try {
			const collection = await invoke<Collection>('create_collection', {
				name: newCollectionName,
			});
			collections = [...collections, collection];
			newCollectionName = '';
		} catch (e) {
			console.error('Failed to create collection:', e);
		}
	}

	async function loadCollections() {
		try {
			collections = await invoke<Collection[]>('get_collections');
		} catch (e) {
			console.error('Failed to load collections:', e);
		}
	}

	function deleteCollection(collectionId: string, event: MouseEvent) {
		event.stopPropagation();
		// Optimistic update - remove from UI immediately
		const previousCollections = collections;
		collections = collections.filter((c) => c.id !== collectionId);
		if (selectedCollection === collectionId) {
			selectedCollection = null;
			documents = [];
		}
		// Fire and forget - index cleanup happens in background
		invoke('delete_collection', { collectionId }).catch((e) => {
			console.error('Failed to delete collection:', e);
			// Rollback on error
			collections = previousCollections;
		});
	}

	function deleteDocument(documentId: string) {
		if (!selectedCollection) return;
		// Optimistic update - remove from UI immediately
		const previousDocuments = documents;
		documents = documents.filter((d) => d.id !== documentId);
		const collectionId = selectedCollection;
		// Fire and forget - index cleanup happens in background
		invoke('delete_document', { collectionId, documentId }).catch((e) => {
			console.error('Failed to delete document:', e);
			// Rollback on error
			documents = previousDocuments;
		});
	}

	// Subscribe to backend events
	let unlistenReady: UnlistenFn;
	let unlistenDocAdded: UnlistenFn;

	onMount(async () => {
		loadCollections();

		unlistenReady = await listen('backend-ready', loadCollections);

		unlistenDocAdded = await listen<{
			collection_id: string;
			document: Document;
		}>('document-added', (event) => {
			const { collection_id, document } = event.payload;
			if (
				selectedCollection === collection_id &&
				!documents.some((d) => d.id === document.id)
			) {
				documents = [...documents, document];
			}
			collections = collections.map((c) =>
				c.id === collection_id
					? { ...c, document_count: c.document_count + 1 }
					: c,
			);
		});
	});

	onDestroy(() => {
		unlistenReady?.();
		unlistenDocAdded?.();
	});

	const tabs: { id: Tab; label: string }[] = [
		{ id: 'trajectory', label: 'Trajectory' },
		{ id: 'search', label: 'Search' },
		{ id: 'files', label: 'Files' },
	];
</script>

<main class="flex h-screen flex-col bg-slate-900 text-slate-100">
	<!-- Tab Navigation -->
	<nav class="flex border-b border-slate-700 bg-slate-800">
		{#each tabs as tab (tab.id)}
			<button
				onclick={() => (activeTab = tab.id)}
				class="px-6 py-3 text-sm font-medium transition-colors {activeTab ===
				tab.id
					? 'border-b-2 border-rose-500 text-rose-500'
					: 'text-slate-400 hover:text-slate-200'}"
			>
				{tab.label}
			</button>
		{/each}
	</nav>

	<!-- Tab Content -->
	<div class="flex-1 overflow-hidden">
		{#if activeTab === 'trajectory'}
			<!-- Trajectory Tab -->
			<Chat />
		{:else if activeTab === 'search'}
			<!-- Search Tab -->
			<div class="flex h-full">
				<Sidebar title="Filter by Collection">
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
											checked={selectedSearchCollections.has(collection.id)}
											onchange={() => toggleSearchCollection(collection.id)}
											class="h-4 w-4 rounded border-slate-600 bg-slate-900 text-rose-500 focus:ring-rose-500"
										/>
										<span
											class="truncate {selectedSearchCollections.has(
												collection.id,
											)
												? 'text-rose-400'
												: 'text-slate-300'}"
										>
											{collection.name}
										</span>
									</label>
								</li>
							{/each}
						</ul>
						{#if selectedSearchCollections.size > 0}
							<button
								onclick={() => (selectedSearchCollections = new SvelteSet())}
								class="mt-3 text-xs text-slate-500 hover:text-slate-300"
							>
								Clear filters
							</button>
						{/if}
					{/if}
				</Sidebar>

				<!-- Search Content -->
				<div class="flex flex-1 flex-col">
					<div class="flex items-center gap-2 border-b border-slate-700 p-4">
						<input
							type="text"
							placeholder="Search documents..."
							bind:value={searchQuery}
							class="flex-1 rounded-md border border-slate-600 bg-slate-900 px-4 py-2 text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
						/>
						{#if searching}
							<span class="text-sm text-slate-500">Searching...</span>
						{/if}
					</div>

					<section class="flex flex-1 flex-col overflow-hidden p-6">
						{#if results.length === 0}
							<p class="text-sm italic text-slate-500">
								{searchQuery ? 'No results found' : 'Start typing to search'}
							</p>
						{:else}
							<div class="mb-2 text-sm text-slate-500">
								{totalHits} result{totalHits === 1 ? '' : 's'} found
							</div>
							<ul class="flex-1 space-y-4 overflow-y-auto">
								{#each results as result (result.document.id)}
									<li
										class="rounded-lg border border-slate-700 bg-slate-800 p-4"
									>
										<div class="mb-2 flex items-center justify-between">
											<h3 class="font-medium text-rose-500">
												{result.document.name}
											</h3>
											<span
												class="rounded bg-slate-700 px-2 py-0.5 text-xs text-slate-400"
											>
												{getCollectionName(result.collection_id)}
											</span>
										</div>
										<p class="text-sm text-slate-400">{result.snippet}</p>
										<span class="mt-2 inline-block text-xs text-slate-600"
											>Score: {result.score.toFixed(2)}</span
										>
									</li>
								{/each}
							</ul>
							{#if totalPages > 1}
								<div
									class="mt-4 flex items-center justify-center gap-2 border-t border-slate-700 pt-4"
								>
									<button
										onclick={() => goToPage(currentPage - 1)}
										disabled={currentPage === 0}
										class="rounded px-3 py-1 text-sm text-slate-400 hover:bg-slate-700 disabled:opacity-50 disabled:hover:bg-transparent"
									>
										Previous
									</button>
									<span class="text-sm text-slate-500">
										Page {currentPage + 1} of {totalPages}
									</span>
									<button
										onclick={() => goToPage(currentPage + 1)}
										disabled={currentPage >= totalPages - 1}
										class="rounded px-3 py-1 text-sm text-slate-400 hover:bg-slate-700 disabled:opacity-50 disabled:hover:bg-transparent"
									>
										Next
									</button>
								</div>
							{/if}
						{/if}
					</section>
				</div>
			</div>
		{:else if activeTab === 'files'}
			<!-- Files Tab -->
			<div class="flex h-full">
				<Sidebar title="Collections">
					<div class="mb-4 flex gap-2">
						<input
							type="text"
							placeholder="New collection..."
							bind:value={newCollectionName}
							onkeydown={(e) => e.key === 'Enter' && createCollection()}
							class="min-w-0 flex-1 rounded-md border border-slate-600 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 placeholder-slate-500 focus:border-rose-500 focus:outline-none"
						/>
						<button
							onclick={createCollection}
							class="rounded-md bg-rose-600 px-3 py-1.5 font-medium text-white hover:bg-rose-700"
						>
							+
						</button>
					</div>
					{#if collections.length === 0}
						<p class="text-sm italic text-slate-500">No collections yet</p>
					{:else}
						<ul class="space-y-1">
							{#each collections as collection (collection.id)}
								<li
									class="group flex cursor-pointer items-center justify-between rounded px-3 py-2 text-sm {selectedCollection ===
									collection.id
										? 'bg-rose-600/20 text-rose-400'
										: 'hover:bg-slate-700'}"
								>
									<button
										type="button"
										onclick={() => selectCollection(collection.id)}
										class="flex-1 truncate text-left"
									>
										{collection.name}
									</button>
									<button
										type="button"
										onclick={(e) => deleteCollection(collection.id, e)}
										class="ml-2 hidden text-slate-500 hover:text-red-400 group-hover:block"
										title="Delete collection"
									>
										x
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				</Sidebar>

				<!-- Documents Area -->
				<section class="flex-1 overflow-y-auto p-6">
					<div class="mb-4 flex items-center justify-between">
						<h2 class="text-sm font-medium text-slate-400">
							{selectedCollection ? 'Documents' : 'Select a collection'}
						</h2>
						<button
							onclick={importPdf}
							disabled={importing || !selectedCollection}
							class="rounded-md bg-rose-600 px-4 py-2 text-sm font-medium text-white hover:bg-rose-700 disabled:opacity-60"
						>
							{importing ? 'Importing...' : 'Import PDF'}
						</button>
					</div>
					{#if documents.length === 0}
						<p class="text-sm italic text-slate-500">No documents yet</p>
					{:else}
						<ul class="space-y-2">
							{#each documents as doc (doc.id)}
								<li
									class="group flex items-center justify-between rounded-lg border border-slate-700 bg-slate-800 px-4 py-3"
								>
									<div>
										<span class="text-slate-200">{doc.name}</span>
										<span class="ml-2 text-xs text-slate-500"
											>{doc.page_count} pages</span
										>
									</div>
									<button
										onclick={() => deleteDocument(doc.id)}
										class="hidden text-slate-500 hover:text-red-400 group-hover:block"
										title="Delete document"
									>
										x
									</button>
								</li>
							{/each}
						</ul>
					{/if}
				</section>
			</div>
		{/if}
	</div>
</main>
