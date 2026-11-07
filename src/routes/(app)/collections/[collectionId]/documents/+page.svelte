<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { open } from '@tauri-apps/plugin-dialog';
	import Button from '#lib/components/Button.svelte';
	import ErrorAlert from '#lib/components/ErrorAlert.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import * as collections from '#lib/stores/collections.svelte.js';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import type { Document } from '#lib/stores/collections.svelte.js';

	let deletingDocument = $state<Document | null>(null);

	const collectionId = $derived(page.params.collectionId);
	const documents = $derived(
		collectionId ? collections.getDocuments(collectionId) : [],
	);
	const documentsLoaded = $derived(
		collectionId ? collections.hasLoadedDocuments(collectionId) : false,
	);
	const documentsError = $derived(
		collectionId ? collections.getDocumentsError(collectionId) : null,
	);

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

	// Load this collection's documents on first view; the store keeps them
	// current afterwards via invalidation.
	$effect(() => {
		if (collectionId) collections.ensureDocuments(collectionId);
	});

	function retryDocuments() {
		if (collectionId) collections.ensureDocuments(collectionId);
	}

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

	function confirmDeleteDocument() {
		if (!deletingDocument || !collectionId) return;
		const documentId = deletingDocument.id;
		deletingDocument = null;
		collections.deleteDocument(collectionId, documentId).catch((e) => {
			console.error('Failed to delete document:', e);
		});
	}
</script>

<div class="flex h-full flex-col">
	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-6">
		{#if processing && stages.length > 0}
			<div class="mb-6 grid grid-cols-3 gap-4">
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

		{#if !documentsLoaded && documentsError}
			<div class="flex flex-col items-start gap-3 py-12">
				<ErrorAlert>Failed to load documents: {documentsError}</ErrorAlert>
				<Button variant="secondary" onclick={retryDocuments}>Retry</Button>
			</div>
		{:else if !documentsLoaded}
			<div
				class="flex flex-col items-center justify-center py-12"
				role="status"
			>
				<LoadingSpinner size="lg" />
				<p class="sr-only">Loading documents…</p>
			</div>
		{:else if documents.length === 0 && processing}
			<div class="flex flex-col items-center justify-center py-12 text-center">
				<p class="text-neutral-500">Processing documents…</p>
				<p class="mt-1 max-w-sm text-sm text-neutral-400">
					Documents will appear here as they finish importing.
				</p>
			</div>
		{:else if documents.length === 0}
			<div class="flex flex-col items-center justify-center py-12 text-center">
				<p class="text-neutral-500">No documents yet</p>
				<p class="mt-1 max-w-sm text-sm text-neutral-400">
					The research agent answers from documents in this collection. Import a
					PDF to give it sources.
				</p>
				<Button class="mt-4" onclick={importPdf}>Import PDF</Button>
			</div>
		{:else}
			<div class="mb-4 flex items-center justify-end">
				<Button onclick={importPdf} disabled={processing}>Import PDF</Button>
			</div>

			<ul class="space-y-2">
				{#each documents as doc (doc.id)}
					<li
						class="flex items-center justify-between gap-4 rounded-lg border border-neutral-200 bg-surface-bright px-4 py-3 transition-colors hover:border-primary-300 hover:shadow-soft"
					>
						<a
							href={resolve(`collections/${collectionId}/documents/${doc.id}`)}
							class="min-w-0 flex-1"
						>
							<span
								class="text-neutral-800 transition-colors hover:text-primary-600"
								>{doc.name}</span
							>
							{#if doc.page_count !== null}
								<span class="ml-2 text-xs text-neutral-500"
									>{doc.page_count} pages</span
								>
							{/if}
						</a>
						<Button
							variant="danger"
							size="sm"
							onclick={() => (deletingDocument = doc)}
						>
							Delete
						</Button>
					</li>
				{/each}
			</ul>
		{/if}
	</div>
</div>

{#if deletingDocument}
	<ConfirmDialog
		title="Delete document"
		message="Delete &quot;{deletingDocument.name}&quot; from this collection? This removes the document and its extracted text from this collection."
		onConfirm={confirmDeleteDocument}
		onCancel={() => (deletingDocument = null)}
	/>
{/if}
