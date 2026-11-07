<script lang="ts">
	import { page } from '$app/state';
	import { resolve } from '$app/paths';
	import { open } from '@tauri-apps/plugin-dialog';
	import Button from '#lib/components/Button.svelte';
	import * as collections from '#lib/stores/collections.svelte.js';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import type { Document } from '#lib/stores/collections.svelte.js';

	let deletingDocument = $state<Document | null>(null);

	const collectionId = $derived(page.params.collectionId);
	const documents = $derived(
		collectionId ? collections.getDocuments(collectionId) : [],
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
		<!-- Actions row -->
		<div class="mb-6 flex items-center justify-end">
			<Button onclick={importPdf} disabled={processing}>Import PDF</Button>
		</div>

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
							href={resolve(`collections/${collectionId}/documents/${doc.id}`)}
							class="flex-1"
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
