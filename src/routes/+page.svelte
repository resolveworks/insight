<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';

	onMount(() => {
		goto(resolve('/research'), { replaceState: true });
	});
</script>

<div class="flex h-screen items-center justify-center bg-neutral-900">
	<p class="text-neutral-500">Redirecting...</p>
</div>
