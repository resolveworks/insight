<script lang="ts">
	import type { Snippet } from 'svelte';

	type Width = 'full' | 'prose';

	interface Props {
		width?: Width;
		children: Snippet;
	}

	let { width = 'full', children }: Props = $props();
</script>

<div class="flex-1 overflow-y-auto p-6">
	{#if width === 'prose'}
		<div class="mx-auto max-w-2xl">
			{@render children()}
		</div>
	{:else}
		{@render children()}
	{/if}
</div>
