<script lang="ts">
	import type { Snippet } from 'svelte';

	type Variant = 'inline' | 'banner';

	interface Props {
		variant?: Variant;
		children: Snippet;
	}

	let { variant = 'inline', children }: Props = $props();
</script>

{#if variant === 'banner'}
	<div class="border-t border-red-700 bg-red-900/50 px-4 py-2 text-red-300">
		{@render children()}
	</div>
{:else}
	<p class="text-sm text-red-400">
		{@render children()}
	</p>
{/if}
