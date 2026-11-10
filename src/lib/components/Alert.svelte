<script lang="ts">
	import type { Snippet } from 'svelte';

	type Variant = 'error' | 'warning';

	interface Props {
		variant?: Variant;
		action?: Snippet;
		children: Snippet;
	}

	let { variant = 'error', action, children }: Props = $props();

	const variantClasses: Record<Variant, string> = {
		error: 'border-red-600/50 bg-red-600/10 text-red-600',
		warning: 'border-amber-600/50 bg-amber-600/10 text-amber-600',
	};
</script>

<div
	role={variant === 'error' ? 'alert' : 'status'}
	class="rounded-lg border p-4 text-sm {variantClasses[variant]}"
>
	{@render children()}
	{#if action}
		<div class="mt-4">
			{@render action()}
		</div>
	{/if}
</div>
