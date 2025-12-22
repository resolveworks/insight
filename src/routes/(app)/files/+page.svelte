<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { resolve } from '$app/paths';
	import { onDestroy, onMount } from 'svelte';
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

	interface ImportResult {
		successful: Document[];
		failed: { path: string; error: string }[];
	}

	let collections = $state<Collection[]>([]);
	let documents = $state<Document[]>([]);
	let importing = $state(false);
	let newCollectionName = $state('');
	let selectedCollection = $state<string | null>(null);

	let unlistenDocAdded: UnlistenFn;

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
		const previousCollections = collections;
		collections = collections.filter((c) => c.id !== collectionId);
		if (selectedCollection === collectionId) {
			selectedCollection = null;
			documents = [];
		}
		invoke('delete_collection', { collectionId }).catch((e) => {
			console.error('Failed to delete collection:', e);
			collections = previousCollections;
		});
	}

	function deleteDocument(documentId: string) {
		if (!selectedCollection) return;
		const previousDocuments = documents;
		documents = documents.filter((d) => d.id !== documentId);
		const collectionId = selectedCollection;
		invoke('delete_document', { collectionId, documentId }).catch((e) => {
			console.error('Failed to delete document:', e);
			documents = previousDocuments;
		});
	}

	onMount(async () => {
		await loadCollections();

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
		unlistenDocAdded?.();
	});
</script>

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
						class="group flex items-center justify-between rounded-lg border border-slate-700 bg-slate-800 px-4 py-3 transition-colors hover:border-slate-600"
					>
						<a
							href={resolve(`/files/${selectedCollection}/${doc.id}`)}
							class="flex-1"
						>
							<span class="text-slate-200 transition-colors hover:text-rose-400"
								>{doc.name}</span
							>
							<span class="ml-2 text-xs text-slate-500"
								>{doc.page_count} pages</span
							>
						</a>
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
