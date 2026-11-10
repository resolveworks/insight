<script lang="ts">
	import Icon from './Icon.svelte';
	import IconButton from './IconButton.svelte';
	import ListRow from './ListRow.svelte';
	import type { Document } from '#lib/types/documents.js';

	interface Props {
		document: Document;
		href: string;
		onDelete: (document: Document) => void;
	}

	let { document, href, onDelete }: Props = $props();
</script>

<ListRow interactive>
	<!-- Callers pass already-resolved paths. -->
	<!-- eslint-disable svelte/no-navigation-without-resolve -->
	<a {href} class="block min-w-0 focus-visible:focus-ring">
		<span class="text-sm text-neutral-800">{document.name}</span>
		{#if document.page_count !== null}
			<p class="mt-1 text-xs text-neutral-500">{document.page_count} pages</p>
		{/if}
	</a>
	<!-- eslint-enable svelte/no-navigation-without-resolve -->
	{#snippet actions()}
		<IconButton
			variant="danger"
			label="Delete document"
			onclick={() => onDelete(document)}
		>
			<Icon name="trash" size="sm" />
		</IconButton>
	{/snippet}
</ListRow>
