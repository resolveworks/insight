<script lang="ts">
	import type { Snippet } from 'svelte';

	type Width = 'full' | 'prose';

	interface Props {
		width?: Width;
		children: Snippet;
	}

	let { width = 'full', children }: Props = $props();
</script>

<div class="h-full overflow-y-auto p-6">
	{#if width === 'prose'}
		<div class="mx-auto max-w-2xl space-y-6">
			{@render children()}
		</div>
	{:else}
		<div class="space-y-6">
			{@render children()}
		</div>
	{/if}
</div>
