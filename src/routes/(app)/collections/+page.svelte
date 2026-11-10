<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Button from '#lib/components/Button.svelte';
	import Input from '#lib/components/Input.svelte';
	import Alert from '#lib/components/Alert.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import {
		createCollection,
		documentCountLabel,
		importCollection,
	} from '#lib/api/collection.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// Create collection state
	let newCollectionName = $state('');

	// Import modal state
	let showImportModal = $state(false);
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);

	const collectionList = $derived(data.collections);

	async function handleCreateCollection() {
		if (!newCollectionName.trim()) return;
		try {
			const collection = await createCollection(newCollectionName);
			newCollectionName = '';
			goto(resolve(`collections/${collection.id}`));
		} catch (e) {
			console.error('Failed to create collection:', e);
		}
	}

	async function handleImportCollection() {
		if (!importTicket.trim()) return;

		importingCollection = true;
		importError = null;

		try {
			const collection = await importCollection(importTicket);
			importTicket = '';
			showImportModal = false;
			goto(resolve(`collections/${collection.id}/documents`));
		} catch (e) {
			console.error('Failed to import collection:', e);
			importError = 'Failed to import collection';
		} finally {
			importingCollection = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	<PageHeader title="Collections" />

	<!-- Content -->
	<div class="flex-1 overflow-y-auto p-6">
		<!-- Actions row -->
		<div class="mb-6 flex flex-wrap items-center gap-4">
			<div class="flex gap-2">
				<Input
					type="text"
					placeholder="New collection..."
					bind:value={newCollectionName}
					onkeydown={(e) => e.key === 'Enter' && handleCreateCollection()}
					class="w-48"
				/>
				<Button onclick={handleCreateCollection}>Create</Button>
			</div>

			<Button variant="secondary" onclick={() => (showImportModal = true)}
				>Import…</Button
			>
		</div>

		<!-- Collections grid -->
		{#if collectionList.length === 0}
			<EmptyState
				title="No collections yet"
				description="Create one above to start organizing your documents."
			/>
		{:else}
			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
				{#each collectionList as collection (collection.id)}
					<article
						class="card p-4 transition-colors hover:border-primary-300 hover:shadow-soft"
					>
						<a href={resolve(`collections/${collection.id}`)} class="block">
							<h3 class="font-medium text-neutral-800">
								{collection.name}
							</h3>
							<p class="mt-1 text-sm text-muted">
								{documentCountLabel(collection.document_count)}
							</p>
							<p class="mt-3 text-sm font-medium text-primary-600">
								Open research workspace
								<span aria-hidden="true">→</span>
							</p>
						</a>
					</article>
				{/each}
			</div>
		{/if}
	</div>
</div>

<!-- Import shared collection modal -->
{#if showImportModal}
	<Modal
		title="Import shared collection"
		onClose={() => (showImportModal = false)}
	>
		<div class="space-y-3">
			<textarea
				placeholder="Paste ticket here..."
				bind:value={importTicket}
				rows="3"
				class="field w-full resize-none text-sm"></textarea>
			<Button
				fullWidth
				onclick={handleImportCollection}
				disabled={importingCollection || !importTicket.trim()}
			>
				{importingCollection ? 'Importing...' : 'Import'}
			</Button>
			{#if importError}
				<Alert>{importError}</Alert>
			{/if}
		</div>
	</Modal>
{/if}
