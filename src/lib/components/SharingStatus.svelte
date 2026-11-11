<script lang="ts">
	import type { CollectionSharingStatus } from '#lib/types/sharing.js';
	import { formatTimestamp, shortenId } from '#lib/format.js';
	import DescriptionList from './DescriptionList.svelte';
	import DescriptionRow from './DescriptionRow.svelte';

	interface Props {
		sharing: CollectionSharingStatus;
	}

	let { sharing }: Props = $props();

	const syncLabel = $derived(sharing.sync_running ? 'Running' : 'Not running');
	const relayLabel = $derived(
		{
			online: 'Connected',
			connecting: 'Connecting',
			offline: 'Disconnected',
		}[sharing.network],
	);
</script>

<DescriptionList>
	<DescriptionRow label="Sync">{syncLabel}</DescriptionRow>
	<DescriptionRow label="Relays">{relayLabel}</DescriptionRow>
	<DescriptionRow label="Peers">
		{sharing.connected_peers} connected · {sharing.known_peers}
		previously synced
	</DescriptionRow>
	<DescriptionRow label="Last sync">
		{#if sharing.last_sync}
			<div>
				{formatTimestamp(sharing.last_sync.finished_at)} ·
				<span class="font-mono">{shortenId(sharing.last_sync.peer)}</span>
			</div>
			{#if sharing.last_sync.error}
				<div class="text-neutral-700">
					Sync failed: {sharing.last_sync.error}
				</div>
			{:else}
				<div>
					{sharing.last_sync.entries_received} received · {sharing.last_sync
						.entries_sent} sent
				</div>
			{/if}
		{:else}
			No sync completed since startup
		{/if}
	</DescriptionRow>
</DescriptionList>
