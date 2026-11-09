<script lang="ts">
	import { page } from '$app/state';
	import { invalidate } from '$app/navigation';
	import { dataKeys } from '#lib/invalidation.js';
	import ErrorScreen from '#lib/components/ErrorScreen.svelte';

	const collectionId = $derived(page.params.collectionId!);

	async function retry() {
		await invalidate(dataKeys.collection(collectionId));
	}
</script>

<ErrorScreen
	notFound={page.status === 404}
	message="Failed to load documents: {page.error?.message}"
	onRetry={retry}
/>
