<script lang="ts">
	import { page } from '$app/state';
	import { invalidate } from '$app/navigation';
	import Alert from '#lib/components/Alert.svelte';
	import Button from '#lib/components/Button.svelte';

	async function retry() {
		await invalidate('app:collections');
	}
</script>

<div
	class="flex h-full flex-col items-start justify-center gap-3 bg-surface px-6"
>
	<Alert>Failed to load collections: {page.error?.message}</Alert>
	<Button variant="secondary" onclick={retry}>Retry</Button>
</div>
