<script lang="ts">
	import { page } from '$app/state';
	import { refreshCollections } from '#lib/invalidation.js';
	import ErrorScreen from '#lib/components/ErrorScreen.svelte';

	async function retry() {
		await refreshCollections();
	}
</script>

<ErrorScreen
	message="Failed to load collections: {page.error?.message}"
	onRetry={retry}
/>
