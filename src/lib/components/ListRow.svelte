<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		variant?: 'card' | 'nav';
		active?: boolean;
		interactive?: boolean;
		actions?: Snippet;
		children: Snippet;
	}

	let {
		variant = 'card',
		active = false,
		interactive = false,
		actions,
		children,
	}: Props = $props();

	const variantClasses: Record<NonNullable<Props['variant']>, string> = {
		card: 'flex items-center justify-between gap-4 rounded-lg border border-line bg-surface-bright px-4 py-3',
		nav: 'flex items-center rounded-md transition-colors',
	};

	const navStateClasses = $derived(
		active
			? 'bg-surface-bright text-neutral-800'
			: 'text-muted hover:bg-surface-dim',
	);
</script>

<li
	class="{variantClasses[variant]} {variant === 'nav'
		? navStateClasses
		: interactive
			? 'transition-colors hover:border-blue-300 hover:shadow-sm'
			: ''}"
>
	<div class="min-w-0 flex-1">
		{@render children()}
	</div>
	{#if actions}
		<div class="shrink-0">
			{@render actions()}
		</div>
	{/if}
</li>
