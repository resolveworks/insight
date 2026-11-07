<script lang="ts">
	import { page } from '$app/state';
	import { invalidate } from '$app/navigation';
	import { resolve } from '$app/paths';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';

	const collectionId = $derived(page.params.collectionId);

	async function retry() {
		await invalidate(`app:collection:${collectionId}`);
	}
</script>

{#if page.status === 404}
	<div
		class="flex h-full flex-col items-center justify-center gap-2 bg-surface"
	>
		<p class="text-neutral-500">Collection not found</p>
		<a
			href={resolve('collections')}
			class="text-sm text-neutral-500 transition-colors hover:text-primary-600"
		>
			Back to collections
		</a>
	</div>
{:else}
	<div
		class="flex h-full flex-col items-start justify-center gap-3 bg-surface px-6"
	>
		<Alert>Failed to load documents: {page.error?.message}</Alert>
		<Button variant="secondary" onclick={retry}>Retry</Button>
	</div>
{/if}
