<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Button from '#lib/components/Button.svelte';
	import CardSection from '#lib/components/CardSection.svelte';
	import CollectionHeader from '#lib/components/CollectionHeader.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import Page from '#lib/components/Page.svelte';
	import ShareTicket from '#lib/components/ShareTicket.svelte';
	import SharingStatus from '#lib/components/SharingStatus.svelte';
	import { deleteCollection, shareCollection } from '#lib/api/collection.js';
	import { describeError } from '#lib/api/provider.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const collection = $derived(data.collection);
	const sharing = $derived(data.sharing);

	// Share modal state
	let showShareModal = $state(false);
	let shareTicket = $state<string | null>(null);
	let shareError = $state<string | null>(null);
	let ticketCopied = $state(false);
	let copyError = $state<string | null>(null);

	const shareState = $derived(
		shareError
			? { status: 'error' as const, message: shareError }
			: shareTicket
				? { status: 'ready' as const, ticket: shareTicket }
				: { status: 'loading' as const },
	);

	// Delete confirmation state
	let confirmingDelete = $state(false);

	async function openShareModal() {
		showShareModal = true;
		shareTicket = null;
		shareError = null;
		ticketCopied = false;
		copyError = null;

		try {
			const ticket = await shareCollection(collection.id);
			if (!showShareModal) return; // modal was closed meanwhile
			shareTicket = ticket;
		} catch (e) {
			if (!showShareModal) return;
			console.error('Failed to share collection:', e);
			shareError = `Failed to generate share ticket: ${describeError(e)}`;
		}
	}

	function closeShareModal() {
		showShareModal = false;
		shareTicket = null;
		shareError = null;
		copyError = null;
	}

	async function copyTicket() {
		if (!shareTicket) return;
		try {
			await navigator.clipboard.writeText(shareTicket);
			copyError = null;
			ticketCopied = true;
			setTimeout(() => (ticketCopied = false), 2000);
		} catch (e) {
			console.error('Failed to copy ticket:', e);
			copyError = "Couldn't copy the ticket. Select it and copy it manually.";
		}
	}

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
	{#snippet header()}
		<CollectionHeader {collection} active="settings" />
	{/snippet}

	<CardSection
		title="Share"
		description="Anyone with a collaboration ticket can add, change, and delete documents in this collection. They sync with this collection, documents included, while this peer is available. Access cannot be revoked."
	>
		<SharingStatus {sharing} />
		{#snippet footer()}
			<Button variant="outline" onclick={openShareModal}
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
	<Modal title="Share collection" onClose={closeShareModal}>
		<ShareTicket
			state={shareState}
			copied={ticketCopied}
			{copyError}
			onCopy={copyTicket}
		/>
	</Modal>
{/if}

<!-- Delete collection confirmation -->
{#if confirmingDelete}
	<ConfirmDialog
		title="Delete collection"
		confirmLabel="Delete collection"
		message="Delete &quot;{collection.name}&quot;? This deletes the collection, its documents, and its local conversations. This cannot be undone."
		onConfirm={handleDelete}
		onCancel={() => (confirmingDelete = false)}
	/>
{/if}
