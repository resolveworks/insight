<script lang="ts">
	import { goto } from '$app/navigation';
	import { resolve } from '$app/paths';
	import { onMount } from 'svelte';

	onMount(() => {
		goto(resolve('/research'), { replaceState: true });
	});
</script>

<div class="flex h-screen items-center justify-center bg-slate-900">
	<p class="text-slate-500">Redirecting...</p>
</div>
