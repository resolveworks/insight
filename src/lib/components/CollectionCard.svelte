<script lang="ts">
	import Card from '#lib/components/Card.svelte';
	import {
		collectionDisplayName,
		documentCountLabel,
	} from '#lib/api/collection.js';
	import type { Collection } from '#lib/types/documents.js';

	interface Props {
		collection: Collection;
		href: string;
	}

	let { collection, href }: Props = $props();
</script>

<Card {href} padding="sm">
	<h3 class="text-sm font-semibold text-neutral-800">
		{collectionDisplayName(collection)}
	</h3>
	<p class="mt-1 text-sm text-neutral-500">
		{collection.name === null
			? 'Waiting for first sync'
			: documentCountLabel(collection.document_count)}
	</p>
</Card>
