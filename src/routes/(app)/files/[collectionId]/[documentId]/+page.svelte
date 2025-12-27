<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import Breadcrumb from '$lib/components/Breadcrumb.svelte';

	interface Document {
		id: string;
		name: string;
		pdf_hash: string;
		text_hash: string;
		page_count: number;
		tags: string[];
		created_at: string;
	}

	interface Collection {
		id: string;
		name: string;
		document_count: number;
	}

	let document = $state<Document | null>(null);
	let content = $state<string | null>(null);
	let chunks = $state<string[] | null>(null);
	let collectionName = $state<string>('');
	let loading = $state(true);
	let loadingContent = $state(false);
	let loadingChunks = $state(false);
	let chunksExpanded = $state(false);
	let chunksError = $state<string | null>(null);
	let error = $state<string | null>(null);

	const collectionId = $derived($page.params.collectionId);
	const documentId = $derived($page.params.documentId);

	const breadcrumbs = $derived([
		{ label: 'Files', href: '/files' },
		{
			label: collectionName || 'Collection',
			href: '/files',
		},
		{ label: document?.name || 'Document' },
	]);

	function formatDate(dateString: string): string {
		try {
			const date = new Date(dateString);
			return date.toLocaleDateString('en-US', {
				year: 'numeric',
				month: 'short',
				day: 'numeric',
				hour: '2-digit',
				minute: '2-digit',
			});
		} catch {
			return dateString;
		}
	}

	function truncateHash(hash: string): string {
		if (hash.length <= 16) return hash;
		return `${hash.slice(0, 8)}...${hash.slice(-8)}`;
	}

	async function loadChunks() {
		if (chunks !== null || loadingChunks) return;

		loadingChunks = true;
		chunksError = null;

		try {
			chunks = await invoke<string[]>('get_document_chunks', {
				collectionId,
				documentId,
			});
		} catch (e) {
			chunksError = e instanceof Error ? e.message : String(e);
		} finally {
			loadingChunks = false;
		}
	}

	function toggleChunks() {
		chunksExpanded = !chunksExpanded;
		if (chunksExpanded && chunks === null) {
			loadChunks();
		}
	}

	onMount(async () => {
		try {
			// Load collection name
			const collections = await invoke<Collection[]>('get_collections');
			const collection = collections.find((c) => c.id === collectionId);
			if (collection) {
				collectionName = collection.name;
			}

			// Load document metadata
			document = await invoke<Document>('get_document', {
				collectionId,
				documentId,
			});

			// Load document content
			loadingContent = true;
			content = await invoke<string>('get_document_text', {
				collectionId,
				documentId,
			});
		} catch (e) {
			error = e instanceof Error ? e.message : String(e);
		} finally {
			loading = false;
			loadingContent = false;
		}
	});
</script>

<!-- Breadcrumb header -->
<header class="border-b border-neutral-700 bg-neutral-800 px-6 py-4">
	<Breadcrumb segments={breadcrumbs} />
</header>

<!-- Content -->
<div class="flex-1 overflow-y-auto p-6">
	{#if loading}
		<p class="text-neutral-500">Loading...</p>
	{:else if error}
		<div class="rounded-lg border border-red-500/50 bg-red-500/10 p-4">
			<p class="text-red-400">{error}</p>
			<a
				href={resolve('/files')}
				class="mt-2 inline-block text-sm text-neutral-400 hover:text-neutral-200"
			>
				Back to files
			</a>
		</div>
	{:else if document}
		<h1 class="mb-6 text-2xl font-semibold text-neutral-100">
			{document.name}
		</h1>

		<div class="max-w-2xl rounded-lg border border-neutral-700 bg-neutral-800">
			<table class="w-full">
				<tbody class="divide-y divide-neutral-700">
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400">ID</td>
						<td class="px-4 py-3 font-mono text-sm text-neutral-200"
							>{document.id}</td
						>
					</tr>
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400">Pages</td
						>
						<td class="px-4 py-3 text-sm text-neutral-200"
							>{document.page_count}</td
						>
					</tr>
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400"
							>PDF Hash</td
						>
						<td
							class="px-4 py-3 font-mono text-sm text-neutral-200"
							title={document.pdf_hash}
						>
							{truncateHash(document.pdf_hash)}
						</td>
					</tr>
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400"
							>Text Hash</td
						>
						<td
							class="px-4 py-3 font-mono text-sm text-neutral-200"
							title={document.text_hash}
						>
							{truncateHash(document.text_hash)}
						</td>
					</tr>
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400">Tags</td>
						<td class="px-4 py-3 text-sm text-neutral-200">
							{#if document.tags.length > 0}
								<div class="flex flex-wrap gap-1">
									{#each document.tags as tag (tag)}
										<span class="rounded bg-neutral-700 px-2 py-0.5 text-xs"
											>{tag}</span
										>
									{/each}
								</div>
							{:else}
								<span class="italic text-neutral-500">(none)</span>
							{/if}
						</td>
					</tr>
					<tr>
						<td class="px-4 py-3 text-sm font-medium text-neutral-400"
							>Created</td
						>
						<td class="px-4 py-3 text-sm text-neutral-200"
							>{formatDate(document.created_at)}</td
						>
					</tr>
				</tbody>
			</table>
		</div>

		<!-- Document Content -->
		<div class="mt-6">
			<h2
				class="mb-3 text-sm font-medium uppercase tracking-wide text-neutral-400"
			>
				Content
			</h2>
			{#if loadingContent}
				<p class="text-neutral-500">Loading content...</p>
			{:else if content}
				<div
					class="max-h-[500px] overflow-y-auto rounded-lg border border-neutral-700 bg-neutral-800 p-4"
				>
					<pre
						class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-neutral-300">{content}</pre>
				</div>
			{:else}
				<p class="italic text-neutral-500">No content available</p>
			{/if}
		</div>

		<!-- Embedding Chunks -->
		<div class="mt-6">
			<button
				onclick={toggleChunks}
				class="flex items-center gap-2 text-sm font-medium uppercase tracking-wide text-neutral-400 transition-colors hover:text-neutral-200"
			>
				<span
					class="inline-block transition-transform"
					class:rotate-90={chunksExpanded}
				>
					&#9654;
				</span>
				Embedding Chunks
				{#if chunks}
					<span class="text-xs font-normal normal-case text-neutral-500"
						>({chunks.length} chunks)</span
					>
				{/if}
			</button>

			{#if chunksExpanded}
				<div class="mt-3">
					{#if loadingChunks}
						<p class="text-neutral-500">Loading chunks...</p>
					{:else if chunksError}
						<div
							class="rounded-lg border border-amber-500/50 bg-amber-500/10 p-3"
						>
							<p class="text-sm text-amber-400">{chunksError}</p>
						</div>
					{:else if chunks && chunks.length > 0}
						<div class="space-y-3">
							{#each chunks as chunk, i (i)}
								<div
									class="rounded-lg border border-neutral-700 bg-neutral-800 p-4"
								>
									<div
										class="mb-2 flex items-center justify-between text-xs text-neutral-500"
									>
										<span>Chunk {i + 1}</span>
										<span>{chunk.length} chars</span>
									</div>
									<pre
										class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-neutral-300">{chunk}</pre>
								</div>
							{/each}
						</div>
					{:else}
						<p class="italic text-neutral-500">No chunks generated</p>
					{/if}
				</div>
			{/if}
		</div>

		<a
			href={resolve('/files')}
			class="mt-6 inline-flex items-center gap-2 text-sm text-neutral-400 transition-colors hover:text-neutral-200"
		>
			<span>&larr;</span>
			<span>Back to files</span>
		</a>
	{/if}
</div>
