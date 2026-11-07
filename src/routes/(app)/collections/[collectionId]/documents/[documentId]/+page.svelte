<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { onMount } from 'svelte';
	import Alert from '#lib/components/Alert.svelte';
	import BackLink from '#lib/components/BackLink.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import type { Document } from '#lib/stores/collections.svelte.js';

	let document = $state<Document | null>(null);
	let content = $state<string | null>(null);
	let chunks = $state<string[] | null>(null);
	let loading = $state(true);
	let loadingContent = $state(false);
	let loadingChunks = $state(false);
	let chunksExpanded = $state(false);
	let chunksError = $state<string | null>(null);
	let error = $state<string | null>(null);
	let contentError = $state<string | null>(null);

	const collectionId = $derived(page.params.collectionId);
	const documentId = $derived(page.params.documentId);

	interface MetadataRow {
		label: string;
		value: string;
		mono?: boolean;
	}

	const metadataRows: MetadataRow[] = $derived(
		document
			? [
					{ label: 'ID', value: document.id, mono: true },
					{ label: 'Type', value: document.file_type },
					...(document.page_count !== null
						? [{ label: 'Pages', value: `${document.page_count}` }]
						: []),
					{ label: 'Created', value: formatDate(document.created_at) },
				]
			: [],
	);

	/** Robustly convert any thrown value (including Tauri v2 errors) to a string. */
	function errorMessage(e: unknown): string {
		if (typeof e === 'string') return e;
		if (e instanceof Error) return e.message;
		// Tauri v2 errors may be plain objects with a `message` property
		if (
			e &&
			typeof e === 'object' &&
			'message' in e &&
			typeof (e as Record<string, unknown>).message === 'string'
		) {
			return (e as Record<string, unknown>).message as string;
		}
		return String(e);
	}

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
			chunksError = errorMessage(e);
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
			document = await invoke<Document>('get_document', {
				collectionId,
				documentId,
			});
		} catch (e) {
			error = errorMessage(e);
			loading = false;
			return;
		}

		// Load document content — failure here is non-critical;
		// the document was just imported and extraction may still be running.
		loadingContent = true;
		try {
			content = await invoke<string>('get_document_text', {
				collectionId,
				documentId,
			});
		} catch (e) {
			contentError = errorMessage(e);
		}

		loading = false;
		loadingContent = false;
	});
</script>

<div class="flex h-full flex-col bg-surface">
	<div class="flex-1 overflow-y-auto p-6">
		<div class="mb-4">
			<BackLink href={resolve(`collections/${collectionId}/documents`)}>
				Back to documents
			</BackLink>
		</div>

		{#if loading}
			<LoadingSpinner size="sm" ariaLabel="Loading document" />
		{:else if error}
			<Alert>{error}</Alert>
		{:else if document}
			<h1 class="text-xl font-medium text-neutral-800">{document.name}</h1>

			<div class="mt-6 max-w-2xl card">
				<table class="w-full">
					<tbody class="divide-y divide-line">
						{#each metadataRows as row (row.label)}
							<tr>
								<td class="px-4 py-3 text-sm font-medium text-muted">
									{row.label}
								</td>
								<td
									class="px-4 py-3 text-sm text-neutral-700"
									class:font-mono={row.mono}
								>
									{row.value}
								</td>
							</tr>
						{/each}
						<tr>
							<td class="px-4 py-3 text-sm font-medium text-muted">Tags</td>
							<td class="px-4 py-3 text-sm text-neutral-700">
								{#if document.tags.length > 0}
									<div class="flex flex-wrap gap-1">
										{#each document.tags as tag (tag)}
											<span
												class="rounded-sm bg-secondary-300 px-2 py-0.5 text-xs text-neutral-800"
												>{tag}</span
											>
										{/each}
									</div>
								{:else}
									<span class="text-sm italic text-subtle">(none)</span>
								{/if}
							</td>
						</tr>
					</tbody>
				</table>
			</div>

			<!-- Document Content -->
			<div class="mt-6">
				<h2 class="mb-3 text-sm font-medium uppercase tracking-wide text-muted">
					Content
				</h2>
				{#if loadingContent}
					<LoadingSpinner size="sm" ariaLabel="Loading content" />
				{:else if contentError}
					<Alert variant="warning">{contentError}</Alert>
				{:else if content}
					<div class="max-h-125 overflow-y-auto card p-4">
						<pre
							class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-neutral-700">{content}</pre>
					</div>
				{:else}
					<p class="text-sm italic text-subtle">No content available</p>
				{/if}
			</div>

			<!-- Text Chunks -->
			<div class="mt-6">
				<button
					onclick={toggleChunks}
					class="flex items-center gap-2 text-sm font-medium uppercase tracking-wide text-muted transition-colors hover:text-neutral-700 focus-visible:focus-ring"
				>
					<span
						class="inline-block transition-transform"
						class:rotate-90={chunksExpanded}
					>
						&#9654;
					</span>
					Text Chunks
					{#if chunks}
						<span class="text-xs font-normal normal-case text-subtle"
							>({chunks.length} chunks)</span
						>
					{/if}
				</button>

				{#if chunksExpanded}
					<div class="mt-3">
						{#if loadingChunks}
							<LoadingSpinner size="sm" ariaLabel="Loading chunks" />
						{:else if chunksError}
							<Alert variant="warning">{chunksError}</Alert>
						{:else if chunks && chunks.length > 0}
							<div class="space-y-3">
								{#each chunks as chunk, i (i)}
									<div class="card p-4">
										<div
											class="mb-2 flex items-center justify-between text-xs text-subtle"
										>
											<span>Chunk {i + 1}</span>
											<span>{chunk.length} chars</span>
										</div>
										<pre
											class="whitespace-pre-wrap font-sans text-sm leading-relaxed text-neutral-700">{chunk}</pre>
									</div>
								{/each}
							</div>
						{:else}
							<p class="text-sm italic text-subtle">No chunks generated</p>
						{/if}
					</div>
				{/if}
			</div>
		{/if}
	</div>
</div>
