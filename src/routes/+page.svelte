<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';
	import CenteredLayout from '$lib/components/CenteredLayout.svelte';

	onMount(() => {
		goto(resolve('/research'), { replaceState: true });
	});
</script>

<CenteredLayout width="sm">
	<p class="text-center text-neutral-500">Redirecting...</p>
</CenteredLayout>
