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
	import ConfirmDialog from '$lib/components/ConfirmDialog.svelte';
	import type { Document } from '$lib/stores/collections.svelte';

	let documents = $state<Document[]>([]);
	let deletingDocument = $state<Document | null>(null);

	const collectionId = $derived($page.params.collectionId);
	const collection = $derived(
		collectionId ? collections.getCollection(collectionId) : undefined,
	);
	const collectionName = $derived(collection?.name ?? 'Collection');

	// Pipeline progress from global store (persists across navigation)
	const processing = $derived(
		collectionId ? collections.isProcessing(collectionId) : false,
	);
	const pipelineProgress = $derived(
		collectionId ? collections.getPipelineProgress(collectionId) : undefined,
	);
	const stages = $derived(
		pipelineProgress
			? [
					{ name: 'Store', data: pipelineProgress.store },
					{ name: 'Extract', data: pipelineProgress.extract },
					{ name: 'Index', data: pipelineProgress.index },
				]
			: [],
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

	function confirmDeleteDocument() {
		if (!deletingDocument || !collectionId) return;
		const documentId = deletingDocument.id;
		deletingDocument = null;
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
			<Button onclick={importPdf} disabled={processing}>Import PDF</Button>
		</div>

		{#if processing && stages.length > 0}
			<div class="mt-4 grid grid-cols-3 gap-4">
				{#each stages as stage (stage.name)}
					{@const total =
						stage.data.pending +
						stage.data.active +
						stage.data.completed +
						stage.data.failed}
					{@const done = stage.data.completed + stage.data.failed}
					{@const percent = total > 0 ? (done / total) * 100 : 0}
					<div class="flex flex-col gap-1">
						<div class="flex justify-between text-xs text-neutral-500">
							<span>{stage.name}</span>
							<span>{done}/{total}</span>
						</div>
						<div class="h-1.5 w-full rounded-full bg-neutral-200">
							<div
								class="h-1.5 rounded-full bg-primary-500 transition-all"
								style="width: {percent}%"
							></div>
						</div>
					</div>
				{/each}
			</div>
		{/if}
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
							onclick={() => (deletingDocument = doc)}
							class="cursor-pointer text-neutral-400 opacity-60 transition-opacity hover:opacity-100 hover:text-error"
							aria-label="Delete document"
							title="Delete document"
						>
							<svg
								class="h-4 w-4"
								viewBox="0 0 24 24"
								fill="none"
								stroke="currentColor"
							>
								<path
									stroke-linecap="round"
									stroke-linejoin="round"
									stroke-width="2"
									d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16"
								/>
							</svg>
						</button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

{#if deletingDocument}
	<ConfirmDialog
		title="Delete document"
		message="Delete &quot;{deletingDocument.name}&quot; from this collection? This removes the document and its extracted text from this machine."
		onConfirm={confirmDeleteDocument}
		onCancel={() => (deletingDocument = null)}
	/>
{/if}
