<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Page from '#lib/components/Page.svelte';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Button from '#lib/components/Button.svelte';
	import ButtonLink from '#lib/components/ButtonLink.svelte';
	import ImportCollectionModal from '#lib/components/ImportCollectionModal.svelte';
	import NewCollectionForm from '#lib/components/NewCollectionForm.svelte';
	import CardGrid from '#lib/components/CardGrid.svelte';
	import CollectionCard from '#lib/components/CollectionCard.svelte';
	import { createCollection } from '#lib/api/collection.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	// Import modal state
	let showImportModal = $state(false);
	let importTicket = $state('');

	const collectionList = $derived(data.collections);

	async function handleCreateCollection(name: string) {
		try {
			const collection = await createCollection(name);
			goto(resolve(`collections/${collection.id}`));
		} catch (e) {
			console.error('Failed to create collection:', e);
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
		<Button variant="outline" onclick={() => (showImportModal = true)}
			>Import</Button
		>
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
	<ImportCollectionModal
		bind:ticket={importTicket}
		onClose={() => (showImportModal = false)}
		onImported={(collection) => {
			showImportModal = false;
			goto(resolve(`collections/${collection.id}/documents`));
		}}
	/>
{/if}
