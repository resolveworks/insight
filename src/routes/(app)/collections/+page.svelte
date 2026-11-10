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
	import { describeError } from '#lib/api/provider.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// Create collection state
	let newCollectionName = $state('');

	// Import modal state
	let showImportModal = $state(false);
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);
	let importOperation = 0;

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

	function openImportModal() {
		importOperation += 1;
		importingCollection = false;
		importError = null;
		showImportModal = true;
	}

	function closeImportModal() {
		importOperation += 1;
		showImportModal = false;
	}

	async function handleImportCollection() {
		if (!importTicket.trim()) return;

		const operation = ++importOperation;
		importingCollection = true;
		importError = null;

		try {
			const collection = await importCollection(importTicket);
			if (operation !== importOperation) return;
			importTicket = '';
			showImportModal = false;
			goto(resolve(`collections/${collection.id}/documents`));
		} catch (e) {
			if (operation !== importOperation) return;
			console.error('Failed to import collection:', e);
			importError = `Failed to import collection: ${describeError(e)}`;
		} finally {
			if (operation === importOperation) importingCollection = false;
		}
	}
</script>

<div class="flex h-full flex-col">
	<PageHeader title="Collections">
		{#snippet actions()}
			<a
				href={resolve('settings')}
				aria-label="Settings"
				title="Settings"
				class="-mr-2 inline-flex rounded-md p-2 text-subtle transition-colors hover:bg-surface-dim hover:text-neutral-700 hover:no-underline focus-visible:focus-ring"
			>
				<svg
					class="h-5 w-5"
					viewBox="0 0 24 24"
					fill="none"
					stroke="currentColor"
				>
					<circle
						cx="12"
						cy="12"
						r="3"
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="1.75"
					/>
					<path
						stroke-linecap="round"
						stroke-linejoin="round"
						stroke-width="1.75"
						d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
					/>
				</svg>
			</a>
		{/snippet}
	</PageHeader>

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

			<Button variant="secondary" onclick={openImportModal}>Import…</Button>
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
	<Modal title="Import shared collection" onClose={closeImportModal}>
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
