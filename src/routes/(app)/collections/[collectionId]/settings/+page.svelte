<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';
	import ConfirmDialog from '#lib/components/ConfirmDialog.svelte';
	import LoadingSpinner from '#lib/components/LoadingSpinner.svelte';
	import Modal from '#lib/components/Modal.svelte';
	import PageContent from '#lib/components/PageContent.svelte';
	import SectionHeading from '#lib/components/SectionHeading.svelte';
	import { deleteCollection, shareCollection } from '#lib/api/collection.js';
	import { describeError } from '#lib/api/provider.js';
	import { formatTimestamp, shortenId } from '#lib/format.js';
	import type { PageProps } from './$types';

	let { data }: PageProps = $props();

	const collection = $derived(data.collection);
	const sharing = $derived(data.sharing);

	const syncLabel = $derived(sharing.sync_running ? 'Running' : 'Not running');
	const relayLabel = $derived(
		{
			online: 'Connected',
			connecting: 'Connecting',
			offline: 'Disconnected',
		}[sharing.network],
	);

	// Share modal state
	let showShareModal = $state(false);
	let shareTicket = $state<string | null>(null);
	let shareError = $state<string | null>(null);
	let ticketCopied = $state(false);
	let copyError = $state<string | null>(null);

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

<div class="flex h-full flex-col">
	<PageContent width="prose">
		<div class="space-y-6">
			<section class="card p-6">
				<SectionHeading title="Share" />
				<p class="mt-1 text-sm text-muted">
					Anyone with a collaboration ticket can add, change, and delete
					documents in this collection. They sync with this collection,
					documents included, while this peer is available. Access cannot be
					revoked.
				</p>
				<dl class="mt-4 divide-y divide-line border-t border-line">
					<div class="flex items-baseline justify-between gap-4 py-2">
						<dt class="text-sm text-muted">Sync</dt>
						<dd class="text-sm text-neutral-800">{syncLabel}</dd>
					</div>
					<div class="flex items-baseline justify-between gap-4 py-2">
						<dt class="text-sm text-muted">Relays</dt>
						<dd class="text-sm text-neutral-800">{relayLabel}</dd>
					</div>
					<div class="flex items-baseline justify-between gap-4 py-2">
						<dt class="text-sm text-muted">Peers</dt>
						<dd class="text-sm text-neutral-800">
							{sharing.connected_peers} connected · {sharing.known_peers}
							previously synced
						</dd>
					</div>
					<div class="flex items-baseline justify-between gap-4 py-2">
						<dt class="text-sm text-muted">Last sync</dt>
						<dd class="text-right text-sm text-neutral-800">
							{#if sharing.last_sync}
								<div>
									{formatTimestamp(sharing.last_sync.finished_at)} ·
									<span class="font-mono"
										>{shortenId(sharing.last_sync.peer)}</span
									>
								</div>
								{#if sharing.last_sync.error}
									<div class="text-error">
										Sync failed: {sharing.last_sync.error}
									</div>
								{:else}
									<div>
										{sharing.last_sync.entries_received} received · {sharing
											.last_sync.entries_sent} sent
									</div>
								{/if}
							{:else}
								No sync completed since startup
							{/if}
						</dd>
					</div>
				</dl>
				<div class="mt-4">
					<Button variant="secondary" onclick={openShareModal}
						>Create collaboration ticket</Button
					>
				</div>
			</section>

			<section class="card border-error/30 p-6">
				<SectionHeading title="Delete collection" />
				<p class="mt-1 text-sm text-muted">
					This deletes the collection, its documents, and its local
					conversations. This cannot be undone.
				</p>
				<div class="mt-4">
					<Button variant="danger" onclick={() => (confirmingDelete = true)}
						>Delete collection</Button
					>
				</div>
			</section>
		</div>
	</PageContent>
</div>

<!-- Share collection modal -->
{#if showShareModal}
	<Modal title="Share collection" onClose={closeShareModal}>
		{#if shareTicket}
			<div class="space-y-3">
				<div class="flex items-start gap-2">
					<code class="flex-1 break-all font-mono text-xs text-muted">
						{shareTicket}
					</code>
					<Button size="sm" variant="secondary" onclick={copyTicket}>
						{ticketCopied ? 'Copied!' : 'Copy'}
					</Button>
				</div>
				<p class="text-xs text-subtle">
					Share this ticket with others so they can add, change, and delete
					documents in this collection.
				</p>
				{#if copyError}
					<Alert>{copyError}</Alert>
				{/if}
				<p class="text-xs text-muted">
					Access cannot be revoked once shared. Syncing requires this peer to be
					available.
				</p>
			</div>
		{:else if shareError}
			<Alert>{shareError}</Alert>
		{:else}
			<div class="flex items-center gap-2 text-sm text-subtle">
				<LoadingSpinner size="sm" />
				<span>Generating ticket...</span>
			</div>
		{/if}
	</Modal>
{/if}

<!-- Delete collection confirmation -->
{#if confirmingDelete}
	<ConfirmDialog
		title="Delete collection"
		message="Delete &quot;{collection.name}&quot;? This deletes the collection, its documents, and its local conversations. This cannot be undone."
		onConfirm={handleDelete}
		onCancel={() => (confirmingDelete = false)}
	/>
{/if}
