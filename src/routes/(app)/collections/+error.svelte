<script lang="ts">
	import { page } from '$app/state';
	import { invalidate } from '$app/navigation';
	import ErrorScreen from '#lib/components/ErrorScreen.svelte';

	async function retry() {
		await invalidate('app:collections');
	}
</script>

<ErrorScreen
	notFound={page.status === 404}
	message="Failed to load collections: {page.error?.message}"
	onRetry={retry}
/>
