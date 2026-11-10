<script lang="ts">
	import type { Snippet } from 'svelte';

	type Width = 'full' | 'prose';

	interface Props {
		width?: Width;
		children: Snippet;
		class?: string;
	}

	let { width = 'full', children, class: className = '' }: Props = $props();
</script>

<div class="flex-1 overflow-y-auto p-6 {className}">
	{#if width === 'prose'}
		<div class="mx-auto max-w-2xl">
			{@render children()}
		</div>
	{:else}
		{@render children()}
	{/if}
</div>
