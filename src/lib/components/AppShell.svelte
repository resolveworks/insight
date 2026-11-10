<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		children: Snippet;
	}

	let { children }: Props = $props();
</script>

<main class="h-screen overflow-hidden bg-neutral-100">
	{@render children()}
</main>
