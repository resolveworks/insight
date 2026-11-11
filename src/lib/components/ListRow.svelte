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
		card: 'flex items-center justify-between gap-4 rounded-lg border border-neutral-200 bg-white px-4 py-3',
		nav: 'flex items-center rounded-md px-2 py-1.5 transition-colors',
	};

	const navStateClasses = $derived(
		active
			? 'bg-white text-neutral-800'
			: 'text-neutral-500 hover:bg-neutral-100',
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
