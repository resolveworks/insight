<script lang="ts">
	import { page } from '$app/stores';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { open } from '@tauri-apps/plugin-dialog';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount, getContext } from 'svelte';
	import Button from '$lib/components/Button.svelte';
	import {
		getImportState,
		startImport,
		recordFailures,
		completeImport,
	} from '$lib/stores/import-state.svelte';

	interface Document {
		id: string;
		name: string;
		file_type: string;
		page_count: number;
		tags: string[];
		created_at: string;
	}

	interface ImportResult {
		successful: Document[];
		failed: { path: string; error: string }[];
	}

	interface CollectionsContext {
		list: { id: string; name: string; document_count: number }[];
		selected: string | null;
	}

	const collectionsContext = getContext<CollectionsContext>('collections');

	let documents = $state<Document[]>([]);

	// Import state from global store (persists across navigation)
	const importState = getImportState();
	let importing = $derived(importState.importing);
	let importProgress = $derived(importState.progress);

	const collectionId = $derived($page.params.collectionId);
	const collectionName = $derived(
		collectionsContext.list.find((c) => c.id === collectionId)?.name ??
			'Collection',
	);

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
		startImport(collectionId, paths.length);

		try {
			const result = await invoke<ImportResult>('import_pdfs_batch', {
				paths,
				collectionId,
			});

			if (result.failed.length > 0) {
				recordFailures(result.failed);
				console.error('Some imports failed:', result.failed);
			}
		} catch (e) {
			console.error('Batch import failed:', e);
		}
		completeImport();
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

<div class="p-6">
	<div class="mb-4 flex items-center justify-between">
		<h2 class="text-sm font-medium text-neutral-600">
			{collectionName}
		</h2>
		<Button onclick={importPdf} disabled={importing}>
			{#if importing && importProgress}
				Importing {importProgress.completed}/{importProgress.total}...
			{:else}
				Import PDF
			{/if}
		</Button>
	</div>
	{#if documents.length === 0}
		<p class="text-sm italic text-neutral-500">No documents yet</p>
	{:else}
		<ul class="space-y-2">
			{#each documents as doc (doc.id)}
				<li
					class="group flex items-center justify-between rounded-lg border border-neutral-200 bg-surface-bright px-4 py-3 transition-colors hover:border-primary-300 hover:shadow-soft"
				>
					<a href={resolve(`/files/${collectionId}/${doc.id}`)} class="flex-1">
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
