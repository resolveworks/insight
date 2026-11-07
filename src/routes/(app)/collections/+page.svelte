<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import PageHeader from '#lib/components/PageHeader.svelte';
	import EmptyState from '#lib/components/EmptyState.svelte';
	import Button from '#lib/components/Button.svelte';
	import Input from '#lib/components/Input.svelte';
	import Alert from '#lib/components/Alert.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import * as collections from '#lib/stores/collections.svelte.js';

	// Create collection state
	let newCollectionName = $state('');

	// Share modal state
	let sharingCollectionId = $state<string | null>(null);
	let shareTicket = $state<string | null>(null);
	let shareError = $state<string | null>(null);
	let ticketCopied = $state(false);

	// Import modal state
	let showImportModal = $state(false);
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);

	// Delete confirmation state
	let deletingCollection = $state<{ id: string; name: string } | null>(null);

	const collectionList = $derived(collections.getCollections());
	const loading = $derived(collections.isLoading());
	const loaded = $derived(collections.hasLoadedCollections());
	const loadError = $derived(collections.getError());

	async function handleCreateCollection() {
		if (!newCollectionName.trim()) return;
		const collection = await collections.createCollection(newCollectionName);
		if (collection) {
			newCollectionName = '';
			goto(resolve(`collections/${collection.id}`));
		}
	}

	async function openShareModal(collectionId: string) {
		sharingCollectionId = collectionId;
		shareTicket = null;
		shareError = null;
		ticketCopied = false;

		const ticket = await collections.shareCollection(collectionId);
		if (sharingCollectionId !== collectionId) return; // modal was closed meanwhile
		if (ticket) {
			shareTicket = ticket;
		} else {
			shareError = 'Failed to generate share ticket';
		}
	}

	function closeShareModal() {
		sharingCollectionId = null;
		shareTicket = null;
		shareError = null;
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

	async function handleImportCollection() {
		if (!importTicket.trim()) return;

		importingCollection = true;
		importError = null;

		const collection = await collections.importCollection(importTicket);
		if (collection) {
			importTicket = '';
			showImportModal = false;
			goto(resolve(`collections/${collection.id}`));
		} else {
			importError = 'Failed to import collection';
		}
		importingCollection = false;
	}

	async function handleDeleteCollection() {
		if (!deletingCollection) return;
		const id = deletingCollection.id;
		deletingCollection = null;
		await collections.deleteCollection(id);
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
		{#if !loaded}
			{#if loading}
				<div class="flex justify-center py-12">
					<LoadingSpinner size="lg" ariaLabel="Loading collections" />
				</div>
			{:else if loadError}
				<Alert>Failed to load collections: {loadError}</Alert>
			{/if}
		{:else}
			{#if loadError}
				<div class="mb-4">
					<Alert>Failed to load collections: {loadError}</Alert>
				</div>
			{/if}
			{#if collectionList.length === 0}
				<EmptyState
					title="No collections yet"
					description="Create one above to start organizing your documents."
				/>
			{:else}
				<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
					{#each collectionList as collection (collection.id)}
						<article
							class="card flex flex-col p-4 transition-colors hover:border-primary-300 hover:shadow-soft"
						>
							<a href={resolve(`collections/${collection.id}`)} class="block">
								<h3 class="font-medium text-neutral-800">
									{collection.name}
								</h3>
								<p class="mt-1 text-sm text-muted">
									{collections.documentCountLabel(collection.document_count)}
								</p>
								<p class="mt-3 text-sm font-medium text-primary-600">
									Open research workspace
									<span aria-hidden="true">→</span>
								</p>
							</a>

							<div class="mt-4 flex gap-2 border-t border-line pt-3">
								<Button
									variant="secondary"
									size="sm"
									onclick={() => openShareModal(collection.id)}
								>
									Share
								</Button>
								<Button
									variant="danger"
									size="sm"
									onclick={() =>
										(deletingCollection = {
											id: collection.id,
											name: collection.name,
										})}
								>
									Delete
								</Button>
							</div>
						</article>
					{/each}
				</div>
			{/if}
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

<!-- Share collection modal -->
{#if sharingCollectionId !== null}
	<Modal title="Share collection" onClose={closeShareModal}>
		{#if shareTicket}
			<div class="space-y-3">
				<div class="flex items-start gap-2">
					<code class="flex-1 break-all text-xs text-neutral-600">
						{shareTicket}
					</code>
					<Button size="sm" variant="secondary" onclick={copyTicket}>
						{ticketCopied ? 'Copied!' : 'Copy'}
					</Button>
				</div>
				<p class="text-xs text-subtle">
					Share this ticket with others to sync this collection
				</p>
				<p class="text-xs text-muted">Access cannot be revoked once shared.</p>
			</div>
		{:else if shareError}
			<Alert>{shareError}</Alert>
		{:else}
			<p class="text-sm text-subtle">Generating ticket...</p>
		{/if}
	</Modal>
{/if}

<!-- Delete collection confirmation -->
{#if deletingCollection}
	<ConfirmDialog
		title="Delete collection"
		message="Delete &quot;{deletingCollection.name}&quot;? This deletes the collection, its documents, and its local conversations. This cannot be undone."
		onConfirm={handleDeleteCollection}
		onCancel={() => (deletingCollection = null)}
	/>
{/if}
