<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount } from 'svelte';
	import Sidebar from '$lib/components/Sidebar.svelte';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import ErrorAlert from '$lib/components/ErrorAlert.svelte';

	let { children } = $props();

	interface Collection {
		id: string;
		name: string;
		document_count: number;
	}

	interface Document {
		id: string;
		name: string;
		file_type: string;
		page_count: number;
		tags: string[];
		created_at: string;
	}

	let collections = $state<Collection[]>([]);
	let newCollectionName = $state('');

	// Get selected collection from URL
	const selectedCollection = $derived($page.params.collectionId ?? null);

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

	async function createCollection() {
		if (!newCollectionName.trim()) return;
		try {
			const collection = await invoke<Collection>('create_collection', {
				name: newCollectionName,
			});
			collections = [...collections, collection];
			newCollectionName = '';
			// Navigate to the new collection
			goto(resolve(`/files/${collection.id}`));
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

		// Navigate away if deleting the selected collection
		if (selectedCollection === collectionId) {
			goto(resolve('/files'));
		}

		invoke('delete_collection', { collectionId }).catch((e) => {
			console.error('Failed to delete collection:', e);
			collections = previousCollections;
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
			// Navigate to the imported collection
			goto(resolve(`/files/${collection.id}`));
		} catch (e) {
			importError = String(e);
			console.error('Failed to import collection:', e);
		} finally {
			importingCollection = false;
		}
	}

	function selectCollection(collectionId: string) {
		if (selectedCollection === collectionId) {
			goto(resolve('/files'));
		} else {
			goto(resolve(`/files/${collectionId}`));
		}
	}

	onMount(async () => {
		await loadCollections();

		unlistenDocAdded = await listen<{
			collection_id: string;
			document: Document;
		}>('document-added', (event) => {
			const { collection_id } = event.payload;
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

	// Export collections for child pages
	import { setContext } from 'svelte';
	setContext('collections', {
		get list() {
			return collections;
		},
		get selected() {
			return selectedCollection;
		},
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
				class="cursor-pointer text-xs text-primary-200 hover:text-surface"
			>
				Import shared collection
			</summary>
			<div class="mt-2 space-y-2">
				<textarea
					placeholder="Paste ticket here..."
					bind:value={importTicket}
					rows="2"
					class="w-full resize-none rounded-md border border-primary-400 bg-primary-700 px-3 py-1.5 text-xs text-surface placeholder-primary-300 focus:border-secondary-400 focus:outline-none"
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
			<p class="text-sm italic text-primary-300">No collections yet</p>
		{:else}
			<ul class="space-y-1">
				{#each collections as collection (collection.id)}
					<li>
						<div
							class="group flex cursor-pointer items-center justify-between rounded px-3 py-2 text-sm {selectedCollection ===
							collection.id
								? 'bg-primary-500 text-surface'
								: 'hover:bg-primary-500 text-primary-100'}"
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
									class="hidden text-primary-200 hover:text-tertiary-300 group-hover:block {sharingCollectionId ===
									collection.id
										? '!block text-tertiary-300'
										: ''}"
									title="Share collection"
								>
									&#8599;
								</button>
								<button
									type="button"
									onclick={(e) => deleteCollection(collection.id, e)}
									class="hidden text-primary-200 hover:text-error group-hover:block"
									title="Delete collection"
								>
									x
								</button>
							</div>
						</div>
						<!-- Share ticket display -->
						{#if sharingCollectionId === collection.id}
							<div
								class="mx-3 mb-2 rounded border border-primary-400 bg-primary-700 p-2"
							>
								{#if shareTicket}
									<div class="flex items-start gap-2">
										<code class="flex-1 break-all text-xs text-primary-100">
											{shareTicket.slice(0, 40)}...
										</code>
										<button
											onclick={copyTicket}
											class="shrink-0 text-xs text-tertiary-300 hover:text-tertiary-200"
										>
											{ticketCopied ? 'Copied!' : 'Copy'}
										</button>
									</div>
									<p class="mt-1 text-xs text-primary-300">
										Share this ticket with others to sync this collection
									</p>
								{:else if shareError}
									<p class="text-xs text-error">{shareError}</p>
								{:else}
									<p class="text-xs text-primary-300">Generating ticket...</p>
								{/if}
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	</Sidebar>

	<!-- Page content slot -->
	<section class="flex-1 overflow-y-auto bg-surface">
		{@render children()}
	</section>
</div>
