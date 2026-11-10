<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Button from '#lib/components/Button.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import Input from '#lib/components/Input.svelte';
	import Textarea from '#lib/components/Textarea.svelte';
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
			<ButtonLink href={resolve('settings')} variant="ghost" size="sm">
				Settings
			</ButtonLink>
		{/snippet}
	</PageHeader>

	<!-- Content -->
	<PageContent>
		<!-- Actions row -->
		<div class="mb-6 flex flex-wrap items-center gap-4">
			<div class="flex gap-2">
				<Input
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
					<article class="card-interactive p-4">
						<a href={resolve(`collections/${collection.id}`)} class="block">
							<h3 class="font-medium text-neutral-800">
								{collection.name}
							</h3>
							<p class="mt-1 text-sm text-muted">
								{documentCountLabel(collection.document_count)}
							</p>
							<p class="mt-3 text-sm font-medium text-primary-600">
								Open research workspace
							</p>
						</a>
					</article>
				{/each}
			</div>
		{/if}
	</PageContent>
</div>

<!-- Import shared collection modal -->
{#if showImportModal}
	<Modal title="Import shared collection" onClose={closeImportModal}>
		<div class="space-y-3">
			<Textarea
				placeholder="Paste ticket here..."
				bind:value={importTicket}
				rows={3}
			/>
			{#if importError}
				<Alert>{importError}</Alert>
			{/if}
		</div>
		{#snippet footer()}
			<Button variant="ghost" onclick={closeImportModal}>Cancel</Button>
			<Button
				onclick={handleImportCollection}
				disabled={importingCollection || !importTicket.trim()}
			>
				{importingCollection ? 'Importing...' : 'Import'}
			</Button>
		{/snippet}
	</Modal>
{/if}
