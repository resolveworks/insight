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
	<div class="border-t border-error/50 bg-error/10 px-4 py-2 text-error">
		{@render children()}
	</div>
{:else}
	<p class="text-sm text-error">
		{@render children()}
	</p>
{/if}
