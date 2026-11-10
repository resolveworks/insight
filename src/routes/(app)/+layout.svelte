<script lang="ts">
	import { onMount } from 'svelte';
	import { startInvalidationListeners } from '#lib/invalidation.js';

	let { children } = $props();

	onMount(() => {
		void startInvalidationListeners();
	});
</script>

<main class="h-screen overflow-hidden bg-surface-dim text-neutral-800">
	{@render children()}
</main>
