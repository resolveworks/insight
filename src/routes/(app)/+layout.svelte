<script lang="ts">
	import { onMount } from 'svelte';
	import { startInvalidationListeners } from '#lib/invalidation.js';
	import AppShell from '#lib/components/AppShell.svelte';

	let { children } = $props();

	onMount(() => {
		void startInvalidationListeners();
	});
</script>

<AppShell>
	{@render children()}
</AppShell>
