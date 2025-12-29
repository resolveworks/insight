<script lang="ts">
	import { getContext } from 'svelte';

	interface CollectionsContext {
		list: { id: string; name: string; document_count: number }[];
		selected: string | null;
	}

	const collections = getContext<CollectionsContext>('collections');
</script>

<div class="flex h-full flex-col items-center justify-center p-6">
	<div class="max-w-md text-center">
		<h2 class="mb-2 text-lg font-medium text-neutral-700">
			{#if collections.list.length === 0}
				No collections yet
			{:else}
				Select a collection
			{/if}
		</h2>
		<p class="text-sm text-neutral-500">
			{#if collections.list.length === 0}
				Create a collection to start organizing your documents.
			{:else}
				Choose a collection from the sidebar to view its documents.
			{/if}
		</p>
	</div>
</div>
