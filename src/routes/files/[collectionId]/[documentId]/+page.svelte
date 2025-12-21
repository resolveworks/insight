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
	let collectionName = $state<string>('');
	let loading = $state(true);
	let loadingContent = $state(false);
	let error = $state<string | null>(null);

	const collectionId = $derived($page.params.collectionId);
	const documentId = $derived($page.params.documentId);

	const breadcrumbs = $derived([
		{ label: 'Files', href: '/' },
		{
			label: collectionName || 'Collection',
			href: `/?tab=files&collection=${collectionId}`,
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

<main class="flex h-screen flex-col bg-slate-900 text-slate-100">
	<!-- Header with breadcrumbs -->
	<header class="border-b border-slate-700 bg-slate-800 px-6 py-4">
		<Breadcrumb segments={breadcrumbs} />
	</header>

	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-6">
		{#if loading}
			<p class="text-slate-500">Loading...</p>
		{:else if error}
			<div class="rounded-lg border border-red-500/50 bg-red-500/10 p-4">
				<p class="text-red-400">{error}</p>
				<a
					href={resolve('/')}
					class="mt-2 inline-block text-sm text-slate-400 hover:text-slate-200"
				>
					Back to files
				</a>
			</div>
		{:else if document}
			<h1 class="mb-6 text-2xl font-semibold text-slate-100">
				{document.name}
			</h1>

			<div class="max-w-2xl rounded-lg border border-slate-700 bg-slate-800">
				<table class="w-full">
					<tbody class="divide-y divide-slate-700">
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400">ID</td>
							<td class="px-4 py-3 font-mono text-sm text-slate-200"
								>{document.id}</td
							>
						</tr>
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400">Pages</td
							>
							<td class="px-4 py-3 text-sm text-slate-200"
								>{document.page_count}</td
							>
						</tr>
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400"
								>PDF Hash</td
							>
							<td
								class="px-4 py-3 font-mono text-sm text-slate-200"
								title={document.pdf_hash}
							>
								{truncateHash(document.pdf_hash)}
							</td>
						</tr>
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400"
								>Text Hash</td
							>
							<td
								class="px-4 py-3 font-mono text-sm text-slate-200"
								title={document.text_hash}
							>
								{truncateHash(document.text_hash)}
							</td>
						</tr>
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400">Tags</td>
							<td class="px-4 py-3 text-sm text-slate-200">
								{#if document.tags.length > 0}
									<div class="flex flex-wrap gap-1">
										{#each document.tags as tag (tag)}
											<span class="rounded bg-slate-700 px-2 py-0.5 text-xs"
												>{tag}</span
											>
										{/each}
									</div>
								{:else}
									<span class="italic text-slate-500">(none)</span>
								{/if}
							</td>
						</tr>
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-slate-400"
								>Created</td
							>
							<td class="px-4 py-3 text-sm text-slate-200"
								>{formatDate(document.created_at)}</td
							>
						</tr>
					</tbody>
				</table>
			</div>

			<!-- Document Content -->
			<div class="mt-6">
				<h2
					class="mb-3 text-sm font-medium uppercase tracking-wide text-slate-400"
				>
					Content
				</h2>
				{#if loadingContent}
					<p class="text-slate-500">Loading content...</p>
				{:else if content}
					<div
						class="max-h-[500px] overflow-y-auto rounded-lg border border-slate-700 bg-slate-800 p-4"
					>
						<pre
							class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-slate-300">{content}</pre>
					</div>
				{:else}
					<p class="italic text-slate-500">No content available</p>
				{/if}
			</div>

			<a
				href={resolve('/')}
				class="mt-6 inline-flex items-center gap-2 text-sm text-slate-400 hover:text-slate-200 transition-colors"
			>
				<span>←</span>
				<span>Back to files</span>
			</a>
		{/if}
	</div>
</main>
