<script lang="ts">
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { open } from '@tauri-apps/plugin-dialog';
	import { resolve } from '$app/paths';
	import { onDestroy, onMount } from 'svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import ErrorAlert from '$lib/components/ErrorAlert.svelte';

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

	// Sharing state
	let sharingCollectionId = $state<string | null>(null);
	let shareTicket = $state<string | null>(null);
	let shareError = $state<string | null>(null);
	let ticketCopied = $state(false);

	// Import from ticket state
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);

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

	async function shareCollection(collectionId: string, event: MouseEvent) {
		event.stopPropagation();
		shareError = null;
		ticketCopied = false;

		if (sharingCollectionId === collectionId) {
			// Toggle off
			sharingCollectionId = null;
			shareTicket = null;
			return;
		}

		sharingCollectionId = collectionId;
		shareTicket = null;

		try {
			shareTicket = await invoke<string>('share_collection', {
				collectionId,
				writable: false,
			});
		} catch (e) {
			shareError = String(e);
			console.error('Failed to share collection:', e);
		}
	}

	async function copyTicket() {
		if (!shareTicket) return;
		try {
			await navigator.clipboard.writeText(shareTicket);
			ticketCopied = true;
			setTimeout(() => (ticketCopied = false), 2000);
		} catch (e) {
			console.error('Failed to copy ticket:', e);
		}
	}

	async function importFromTicket() {
		if (!importTicket.trim()) return;

		importingCollection = true;
		importError = null;

		try {
			const collection = await invoke<Collection>('import_collection', {
				ticket: importTicket.trim(),
			});
			collections = [...collections, collection];
			importTicket = '';
			// Auto-select the imported collection
			await selectCollection(collection.id);
		} catch (e) {
			importError = String(e);
			console.error('Failed to import collection:', e);
		} finally {
			importingCollection = false;
		}
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
		<!-- Create new collection -->
		<div class="mb-3 flex gap-2">
			<Input
				type="text"
				placeholder="New collection..."
				bind:value={newCollectionName}
				onkeydown={(e) => e.key === 'Enter' && createCollection()}
				class="min-w-0 text-sm"
			/>
			<Button size="sm" onclick={createCollection}>+</Button>
		</div>

		<!-- Import from ticket -->
		<details class="mb-4">
			<summary
				class="cursor-pointer text-xs text-neutral-400 hover:text-neutral-300"
			>
				Import shared collection
			</summary>
			<div class="mt-2 space-y-2">
				<textarea
					placeholder="Paste ticket here..."
					bind:value={importTicket}
					rows="2"
					class="w-full resize-none rounded-md border border-neutral-600 bg-neutral-900 px-3 py-1.5 text-xs text-neutral-100 placeholder-neutral-500 focus:border-slate-400 focus:outline-none"
				></textarea>
				<Button
					variant="secondary"
					size="sm"
					fullWidth
					onclick={importFromTicket}
					disabled={importingCollection || !importTicket.trim()}
				>
					{importingCollection ? 'Importing...' : 'Import'}
				</Button>
				{#if importError}
					<ErrorAlert>{importError}</ErrorAlert>
				{/if}
			</div>
		</details>

		{#if collections.length === 0}
			<p class="text-sm italic text-neutral-500">No collections yet</p>
		{:else}
			<ul class="space-y-1">
				{#each collections as collection (collection.id)}
					<li>
						<div
							class="group flex cursor-pointer items-center justify-between rounded px-3 py-2 text-sm {selectedCollection ===
							collection.id
								? 'bg-slate-600/20 text-slate-300'
								: 'hover:bg-neutral-700'}"
						>
							<button
								type="button"
								onclick={() => selectCollection(collection.id)}
								class="flex-1 truncate text-left"
							>
								{collection.name}
							</button>
							<div class="ml-2 flex gap-1">
								<button
									type="button"
									onclick={(e) => shareCollection(collection.id, e)}
									class="hidden text-neutral-500 hover:text-blue-400 group-hover:block {sharingCollectionId ===
									collection.id
										? '!block text-blue-400'
										: ''}"
									title="Share collection"
								>
									&#8599;
								</button>
								<button
									type="button"
									onclick={(e) => deleteCollection(collection.id, e)}
									class="hidden text-neutral-500 hover:text-red-400 group-hover:block"
									title="Delete collection"
								>
									x
								</button>
							</div>
						</div>
						<!-- Share ticket display -->
						{#if sharingCollectionId === collection.id}
							<div
								class="mx-3 mb-2 rounded border border-neutral-600 bg-neutral-900 p-2"
							>
								{#if shareTicket}
									<div class="flex items-start gap-2">
										<code class="flex-1 break-all text-xs text-neutral-300">
											{shareTicket.slice(0, 40)}...
										</code>
										<button
											onclick={copyTicket}
											class="shrink-0 text-xs text-blue-400 hover:text-blue-300"
										>
											{ticketCopied ? 'Copied!' : 'Copy'}
										</button>
									</div>
									<p class="mt-1 text-xs text-neutral-500">
										Share this ticket with others to sync this collection
									</p>
								{:else if shareError}
									<p class="text-xs text-red-400">{shareError}</p>
								{:else}
									<p class="text-xs text-neutral-400">Generating ticket...</p>
								{/if}
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</Sidebar>

	<!-- Documents Area -->
	<section class="flex-1 overflow-y-auto p-6">
		<div class="mb-4 flex items-center justify-between">
			<h2 class="text-sm font-medium text-neutral-400">
				{selectedCollection ? 'Documents' : 'Select a collection'}
			</h2>
			<Button onclick={importPdf} disabled={importing || !selectedCollection}>
				{importing ? 'Importing...' : 'Import PDF'}
			</Button>
		</div>
		{#if documents.length === 0}
			<p class="text-sm italic text-neutral-500">No documents yet</p>
		{:else}
			<ul class="space-y-2">
				{#each documents as doc (doc.id)}
					<li
						class="group flex items-center justify-between rounded-lg border border-neutral-700 bg-neutral-800 px-4 py-3 transition-colors hover:border-neutral-600"
					>
						<a
							href={resolve(`/files/${selectedCollection}/${doc.id}`)}
							class="flex-1"
						>
							<span
								class="text-neutral-200 transition-colors hover:text-slate-300"
								>{doc.name}</span
							>
							<span class="ml-2 text-xs text-neutral-500"
								>{doc.page_count} pages</span
							>
						</a>
						<button
							onclick={() => deleteDocument(doc.id)}
							class="hidden text-neutral-500 hover:text-red-400 group-hover:block"
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
