<script lang="ts">
	import type { Snippet } from 'svelte';

	type Variant = 'error' | 'warning';

	interface Props {
		variant?: Variant;
		action?: Snippet;
		children: Snippet;
	}

	let { variant = 'error', action, children }: Props = $props();
</script>

<div
	role={variant === 'error' ? 'alert' : 'status'}
	class="rounded-lg border border-neutral-300 bg-neutral-100 p-4 text-sm text-neutral-800"
>
	{@render children()}
	{#if action}
		<div class="mt-4">
			{@render action()}
		</div>
	{/if}
</div>
