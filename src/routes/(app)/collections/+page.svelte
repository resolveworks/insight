<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Button from '#lib/components/Button.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import Textarea from '#lib/components/Textarea.svelte';
	import Alert from '#lib/components/Alert.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import NewCollectionForm from '#lib/components/NewCollectionForm.svelte';
	import CardGrid from '#lib/components/CardGrid.svelte';
	import CollectionCard from '#lib/components/CollectionCard.svelte';
	import { createCollection, importCollection } from '#lib/api/collection.js';
	import { describeError } from '#lib/api/provider.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// Import modal state
	let showImportModal = $state(false);
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);
	let importOperation = 0;

	const collectionList = $derived(data.collections);

	async function handleCreateCollection(name: string) {
		try {
			const collection = await createCollection(name);
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

<Page>
	{#snippet header()}
		<PageHeader title="Collections">
			{#snippet actions()}
				<ButtonLink href={resolve('settings')} variant="outline" size="sm">
					Settings
				</ButtonLink>
			{/snippet}
		</PageHeader>
	{/snippet}

	<div class="flex flex-wrap items-center gap-2">
		<NewCollectionForm onCreate={handleCreateCollection} />
		<Button variant="outline" onclick={openImportModal}>Import</Button>
	</div>

	<!-- Collections grid -->
	{#if collectionList.length === 0}
		<EmptyState
			title="No collections yet"
			description="Create one above to start organizing your documents."
		/>
	{:else}
		<CardGrid>
			{#each collectionList as collection (collection.id)}
				<CollectionCard
					{collection}
					href={resolve(`collections/${collection.id}`)}
				/>
			{/each}
		</CardGrid>
	{/if}
</Page>

<!-- Import shared collection modal -->
{#if showImportModal}
	<Modal title="Import shared collection" onClose={closeImportModal}>
		<Textarea
			placeholder="Paste ticket here..."
			bind:value={importTicket}
			rows={3}
		/>
		{#if importError}
			<Alert>{importError}</Alert>
		{/if}
		{#snippet footer()}
			<Button variant="outline" onclick={closeImportModal}>Cancel</Button>
			<Button
				variant="solid"
				onclick={handleImportCollection}
				disabled={importingCollection || !importTicket.trim()}
			>
				{importingCollection ? 'Importing...' : 'Import'}
			</Button>
		{/snippet}
	</Modal>
{/if}
