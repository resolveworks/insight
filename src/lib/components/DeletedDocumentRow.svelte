<script lang="ts">
	import Button from './Button.svelte';
	import ListRow from './ListRow.svelte';
	import { formatTimestamp, shortenId } from '#lib/format.js';
	import type { DeletedDocument } from '#lib/types/documents.js';

	interface Props {
		document: DeletedDocument;
		onRestore: (documentId: string) => void;
	}

	let { document, onRestore }: Props = $props();
</script>

<ListRow>
	<p class="text-sm text-neutral-800">{document.name}</p>
	<p class="mt-1 text-xs text-neutral-500">
		Deleted by {shortenId(document.deleted_by)} · {formatTimestamp(
			document.deleted_at,
		)}
	</p>
	{#snippet actions()}
		<Button
			variant="secondary"
			size="sm"
			onclick={() => onRestore(document.id)}
		>
			Restore
		</Button>
	{/snippet}
</ListRow>
