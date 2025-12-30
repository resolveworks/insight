<script lang="ts">
	import { page } from '$app/stores';
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { invoke } from '@tauri-apps/api/core';
	import { listen, type UnlistenFn } from '@tauri-apps/api/event';
	import { onDestroy, onMount, setContext } from 'svelte';

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

	// Get selected collection from URL
	const selectedCollection = $derived($page.params.collectionId ?? null);

	let unlistenDocAdded: UnlistenFn;

	async function loadCollections() {
		try {
			collections = await invoke<Collection[]>('get_collections');
		} catch (e) {
			console.error('Failed to load collections:', e);
		}
	}

	async function createCollection(name: string): Promise<Collection | null> {
		if (!name.trim()) return null;
		try {
			const collection = await invoke<Collection>('create_collection', {
				name,
			});
			collections = [...collections, collection];
			return collection;
		} catch (e) {
			console.error('Failed to create collection:', e);
			return null;
		}
	}

	async function deleteCollection(collectionId: string): Promise<boolean> {
		const previousCollections = collections;
		collections = collections.filter((c) => c.id !== collectionId);

		// Navigate away if deleting the selected collection
		if (selectedCollection === collectionId) {
			goto(resolve('/files'));
		}

		try {
			await invoke('delete_collection', { collectionId });
			return true;
		} catch (e) {
			console.error('Failed to delete collection:', e);
			collections = previousCollections;
			return false;
		}
	}

	async function shareCollection(collectionId: string): Promise<string | null> {
		try {
			return await invoke<string>('share_collection', {
				collectionId,
				writable: false,
			});
		} catch (e) {
			console.error('Failed to share collection:', e);
			return null;
		}
	}

	async function importCollection(ticket: string): Promise<Collection | null> {
		try {
			const collection = await invoke<Collection>('import_collection', {
				ticket: ticket.trim(),
			});
			collections = [...collections, collection];
			return collection;
		} catch (e) {
			console.error('Failed to import collection:', e);
			return null;
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

	// Export collections context for child pages
	setContext('collections', {
		get list() {
			return collections;
		},
		get selected() {
			return selectedCollection;
		},
		createCollection,
		deleteCollection,
		shareCollection,
		importCollection,
	});
</script>

<div class="h-full overflow-y-auto bg-surface">
	{@render children()}
</div>
