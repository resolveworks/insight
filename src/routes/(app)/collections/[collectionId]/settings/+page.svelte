<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Button from '#lib/components/Button.svelte';
	import CardSection from '#lib/components/CardSection.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import Page from '#lib/components/Page.svelte';
	import ShareCollectionModal from '#lib/components/ShareCollectionModal.svelte';
	import SharingStatus from '#lib/components/SharingStatus.svelte';
	import {
		collectionDisplayName,
		deleteCollection,
	} from '#lib/api/collection.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const collection = $derived(data.collection);
	const collectionName = $derived(collectionDisplayName(collection));
	const sharing = $derived(data.sharing);

	// Share modal state
	let showShareModal = $state(false);

	// Delete confirmation state
	let confirmingDelete = $state(false);

	async function handleDelete() {
		confirmingDelete = false;
		try {
			await deleteCollection(collection.id);
			await goto(resolve('collections'));
		} catch (e) {
			console.error('Failed to delete collection:', e);
		}
	}
</script>

<Page measure="prose">
	<CardSection
		title="Share"
		description="Anyone with a collaboration ticket can add, change, and delete documents in this collection. They sync with this collection, documents included, while this peer is available. Access cannot be revoked."
	>
		<SharingStatus {sharing} />
		{#snippet footer()}
			<Button variant="outline" onclick={() => (showShareModal = true)}
				>Create collaboration ticket</Button
			>
		{/snippet}
	</CardSection>

	<CardSection
		title="Delete collection"
		description="This deletes the collection, its documents, and its local conversations. This cannot be undone."
	>
		{#snippet footer()}
			<Button variant="outline" onclick={() => (confirmingDelete = true)}
				>Delete collection</Button
			>
		{/snippet}
	</CardSection>
</Page>

<!-- Share collection modal -->
{#if showShareModal}
	<ShareCollectionModal
		collectionId={collection.id}
		onClose={() => (showShareModal = false)}
	/>
{/if}

<!-- Delete collection confirmation -->
{#if confirmingDelete}
	<ConfirmDialog
		title="Delete collection"
		confirmLabel="Delete collection"
		message="Delete &quot;{collectionName}&quot;? This deletes the collection, its documents, and its local conversations. This cannot be undone."
		onConfirm={handleDelete}
		onCancel={() => (confirmingDelete = false)}
	/>
{/if}
