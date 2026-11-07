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
		error: 'border-error/50 bg-error/10 text-error',
		warning: 'border-warning/50 bg-warning/10 text-warning',
	};
</script>

<div
	role="alert"
	class="rounded-lg border p-4 text-sm {variantClasses[variant]}"
>
	{@render children()}
	{#if action}
		<div class="mt-3">
			{@render action()}
		</div>
	{/if}
</div>
