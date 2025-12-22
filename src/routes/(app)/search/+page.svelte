<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import { SvelteSet } from 'svelte/reactivity';
	import Sidebar from '$lib/components/Sidebar.svelte';

	interface Collection {
		id: string;
		name: string;
		document_count: number;
	}

	interface Document {
		id: string;
		name: string;
		pdf_hash: string;
		text_hash: string;
		page_count: number;
		tags: string[];
		created_at: string;
	}

	interface SearchHit {
		chunk_id: string;
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

	let searchQuery = $state('');
	let results = $state<SearchHit[]>([]);
	let totalHits = $state(0);
	let currentPage = $state(0);
	let pageSize = $state(20);
	let collections = $state<Collection[]>([]);
	let selectedSearchCollections = new SvelteSet<string>();
	let searching = $state(false);
	let semanticRatio = $state(0);

	let searchTimeout: ReturnType<typeof setTimeout> | null = null;

	$effect(() => {
		const query = searchQuery;
		const filterIds = selectedSearchCollections;
		const ratio = semanticRatio;

		currentPage = 0;

		if (searchTimeout) {
			clearTimeout(searchTimeout);
		}

		searchTimeout = setTimeout(() => {
			performSearch(query, filterIds, 0, ratio);
		}, 200);

		return () => {
			if (searchTimeout) clearTimeout(searchTimeout);
		};
	});

	async function performSearch(
		query: string,
		filterIds: SvelteSet<string>,
		page: number,
		ratio: number,
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
				semanticRatio: ratio,
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
		performSearch(searchQuery, selectedSearchCollections, page, semanticRatio);
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
								class="truncate {selectedSearchCollections.has(collection.id)
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
		<div class="border-b border-slate-700 p-4">
			<div class="flex items-center gap-2">
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
			<!-- Semantic ratio slider -->
			<div class="mt-3 flex items-center gap-3">
				<span class="w-16 text-xs text-slate-500">Keyword</span>
				<input
					type="range"
					min="0"
					max="1"
					step="0.1"
					bind:value={semanticRatio}
					class="h-1.5 flex-1 cursor-pointer appearance-none rounded-lg bg-slate-700 accent-rose-500"
				/>
				<span class="w-16 text-right text-xs text-slate-500">Semantic</span>
				<span class="w-8 text-center font-mono text-xs text-slate-400"
					>{Math.round(semanticRatio * 100)}%</span
				>
			</div>
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
					{#each results as result (result.chunk_id)}
						<li class="rounded-lg border border-slate-700 bg-slate-800 p-4">
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
