<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { open } from '@tauri-apps/plugin-dialog';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';
	import Button from '$lib/components/Button.svelte';
	import Breadcrumb from '$lib/components/Breadcrumb.svelte';
	import * as collections from '$lib/stores/collections.svelte';
	import type { Document } from '$lib/stores/collections.svelte';

	let documents = $state<Document[]>([]);

	const collectionId = $derived($page.params.collectionId);
	const collection = $derived(
		collectionId ? collections.getCollection(collectionId) : undefined,
	);
	const collectionName = $derived(collection?.name ?? 'Collection');

	// Import state from global store (persists across navigation)
	const importing = $derived(
		collectionId ? collections.isImporting(collectionId) : false,
	);
	const importProgress = $derived(
		collectionId ? collections.getImportProgress(collectionId) : undefined,
	);

	const breadcrumbs = $derived([
		{ label: 'Files', href: '/files' },
		{ label: collectionName },
	]);

	let unlistenDocAdded: UnlistenFn;

	async function importPdf() {
		if (!collectionId) {
			console.error('No collection selected');
			return;
		}

		const files = await open({
			multiple: true,
			filters: [{ name: 'PDF', extensions: ['pdf'] }],
		});

		if (!files) return;

		const paths = Array.isArray(files) ? files : [files];
		await collections.startImport(collectionId, paths);
	}

	async function loadDocuments() {
		if (!collectionId) return;
		try {
			documents = await invoke<Document[]>('get_documents', { collectionId });
		} catch (e) {
			console.error('Failed to load documents:', e);
			documents = [];
		}
	}

	function deleteDocument(documentId: string) {
		if (!collectionId) return;
		const previousDocuments = documents;
		documents = documents.filter((d) => d.id !== documentId);
		invoke('delete_document', { collectionId, documentId }).catch((e) => {
			console.error('Failed to delete document:', e);
			documents = previousDocuments;
		});
	}

	onMount(async () => {
		await loadDocuments();

		unlistenDocAdded = await listen<{
			collection_id: string;
			document: Document;
		}>('document-added', (event) => {
			const { collection_id, document } = event.payload;
			if (
				collectionId === collection_id &&
				!documents.some((d) => d.id === document.id)
			) {
				documents = [...documents, document];
			}
		});
	});

	onDestroy(() => {
		unlistenDocAdded?.();
	});

	// Reload documents when collection changes
	$effect(() => {
		if (collectionId) {
			loadDocuments();
		}
	});
</script>

<div class="flex h-full flex-col">
	<!-- Header -->
	<header class="border-b border-neutral-200 bg-surface-bright px-6 py-4">
		<div class="flex items-center justify-between">
			<Breadcrumb segments={breadcrumbs} />
			<Button onclick={importPdf} disabled={importing}>
				{#if importing && importProgress}
					Importing ({importProgress.pending + importProgress.in_progress} remaining)
				{:else}
					Import PDF
				{/if}
			</Button>
		</div>
	</header>

	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-6">
		{#if documents.length === 0}
			<div class="flex flex-col items-center justify-center py-12">
				<p class="text-neutral-500">No documents yet</p>
				<p class="mt-1 text-sm text-neutral-400">
					Import PDFs to add documents to this collection.
				</p>
			</div>
		{:else}
			<ul class="space-y-2">
				{#each documents as doc (doc.id)}
					<li
						class="group flex items-center justify-between rounded-lg border border-neutral-200 bg-surface-bright px-4 py-3 transition-colors hover:border-primary-300 hover:shadow-soft"
					>
						<a
							href={resolve(`/files/${collectionId}/${doc.id}`)}
							class="flex-1"
						>
							<span
								class="text-neutral-800 transition-colors hover:text-primary-600"
								>{doc.name}</span
							>
							<span class="ml-2 text-xs text-neutral-500"
								>{doc.page_count} pages</span
							>
						</a>
						<button
							onclick={() => deleteDocument(doc.id)}
							class="hidden text-neutral-400 hover:text-error group-hover:block"
							title="Delete document"
						>
							x
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>
