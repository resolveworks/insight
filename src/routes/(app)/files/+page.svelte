<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Breadcrumb from '$lib/components/Breadcrumb.svelte';
	import Button from '$lib/components/Button.svelte';
	import Input from '$lib/components/Input.svelte';
	import ErrorAlert from '$lib/components/ErrorAlert.svelte';
	import * as collections from '$lib/stores/collections.svelte';

	// Create collection state
	let newCollectionName = $state('');

	// Sharing state
	let sharingCollectionId = $state<string | null>(null);
	let shareTicket = $state<string | null>(null);
	let shareError = $state<string | null>(null);
	let ticketCopied = $state(false);

	// Import from ticket state
	let importTicket = $state('');
	let importingCollection = $state(false);
	let importError = $state<string | null>(null);

	const breadcrumbs = [{ label: 'Files' }];

	const collectionList = $derived(collections.getCollections());

	async function handleCreateCollection() {
		if (!newCollectionName.trim()) return;
		const collection = await collections.createCollection(newCollectionName);
		if (collection) {
			newCollectionName = '';
			goto(resolve(`/files/${collection.id}`));
		}
	}

	async function handleShareCollection(collectionId: string) {
		shareError = null;
		ticketCopied = false;

		if (sharingCollectionId === collectionId) {
			sharingCollectionId = null;
			shareTicket = null;
			return;
		}

		sharingCollectionId = collectionId;
		shareTicket = null;

		const ticket = await collections.shareCollection(collectionId);
		if (ticket) {
			shareTicket = ticket;
		} else {
			shareError = 'Failed to generate share ticket';
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

	async function handleImportCollection() {
		if (!importTicket.trim()) return;

		importingCollection = true;
		importError = null;

		const collection = await collections.importCollection(importTicket);
		if (collection) {
			importTicket = '';
			goto(resolve(`/files/${collection.id}`));
		} else {
			importError = 'Failed to import collection';
		}
		importingCollection = false;
	}

	async function handleDeleteCollection(collectionId: string) {
		await collections.deleteCollection(collectionId);
		if (sharingCollectionId === collectionId) {
			sharingCollectionId = null;
			shareTicket = null;
		}
	}
</script>

<div class="flex h-full flex-col">
	<!-- Header -->
	<header class="border-b border-neutral-200 bg-surface-bright px-6 py-4">
		<Breadcrumb segments={breadcrumbs} />
	</header>

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

			<details class="relative">
				<summary
					class="cursor-pointer text-sm text-neutral-500 hover:text-neutral-700"
				>
					Import shared collection
				</summary>
				<div
					class="absolute left-0 top-full z-10 mt-2 w-80 rounded-lg border border-neutral-200 bg-surface-bright p-4 shadow-lg"
				>
					<textarea
						placeholder="Paste ticket here..."
						bind:value={importTicket}
						rows="3"
						class="w-full resize-none rounded-md border border-neutral-300 bg-surface px-3 py-2 text-sm placeholder-neutral-400 focus:border-primary-400 focus:outline-none"
					></textarea>
					<Button
						class="mt-2"
						fullWidth
						onclick={handleImportCollection}
						disabled={importingCollection || !importTicket.trim()}
					>
						{importingCollection ? 'Importing...' : 'Import'}
					</Button>
					{#if importError}
						<div class="mt-2">
							<ErrorAlert>{importError}</ErrorAlert>
						</div>
					{/if}
				</div>
			</details>
		</div>

		<!-- Collections grid -->
		{#if collectionList.length === 0}
			<div class="flex flex-col items-center justify-center py-12">
				<p class="text-neutral-500">No collections yet</p>
				<p class="mt-1 text-sm text-neutral-400">
					Create a collection to start organizing your documents.
				</p>
			</div>
		{:else}
			<div class="grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3">
				{#each collectionList as collection (collection.id)}
					<div
						class="group relative rounded-lg border border-neutral-200 bg-surface-bright p-4 transition-colors hover:border-primary-300 hover:shadow-soft"
					>
						<a href={resolve(`/files/${collection.id}`)} class="block">
							<h3 class="font-medium text-neutral-800">
								{collection.name}
							</h3>
							<p class="mt-1 text-sm text-neutral-500">
								{collection.document_count}
								{collection.document_count === 1 ? 'document' : 'documents'}
							</p>
						</a>

						<!-- Action buttons -->
						<div
							class="absolute right-3 top-3 flex gap-2 opacity-0 transition-opacity group-hover:opacity-100"
						>
							<button
								onclick={() => handleShareCollection(collection.id)}
								class="text-neutral-400 hover:text-tertiary-500"
								title="Share collection"
							>
								&#8599;
							</button>
							<button
								onclick={() => handleDeleteCollection(collection.id)}
								class="text-neutral-400 hover:text-error"
								title="Delete collection"
							>
								&times;
							</button>
						</div>

						<!-- Share ticket display -->
						{#if sharingCollectionId === collection.id}
							<div
								class="mt-3 rounded border border-neutral-200 bg-surface p-3"
							>
								{#if shareTicket}
									<div class="flex items-start gap-2">
										<code class="flex-1 break-all text-xs text-neutral-600">
											{shareTicket.slice(0, 50)}...
										</code>
										<button
											onclick={copyTicket}
											class="shrink-0 text-xs text-tertiary-500 hover:text-tertiary-600"
										>
											{ticketCopied ? 'Copied!' : 'Copy'}
										</button>
									</div>
									<p class="mt-2 text-xs text-neutral-400">
										Share this ticket with others to sync this collection
									</p>
								{:else if shareError}
									<p class="text-xs text-error">{shareError}</p>
								{:else}
									<p class="text-xs text-neutral-400">Generating ticket...</p>
								{/if}
							</div>
						{/if}
					</div>
				{/each}
			</div>
		{/if}
	</div>
</div>
