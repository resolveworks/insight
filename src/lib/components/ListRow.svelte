<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		variant?: 'card' | 'nav';
		active?: boolean;
		/** Renders the row as a link. */
		href?: string;
		/** Renders the row as a button. */
		onclick?: () => void;
		actions?: Snippet;
		children: Snippet;
	}

	let {
		variant = 'card',
		active = false,
		href,
		onclick,
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

	const actionable = $derived(href !== undefined || onclick !== undefined);
</script>

<li
	class="{variantClasses[variant]} {variant === 'nav'
		? navStateClasses
		: actionable
			? 'transition-colors hover:border-neutral-400 hover:shadow-sm'
			: ''}"
>
	{#if href}
		<!-- Callers pass already-resolved paths. -->
		<!-- eslint-disable svelte/no-navigation-without-resolve -->
		<a {href} class="block min-w-0 flex-1 focus-visible:focus-ring">
			{@render children()}
		</a>
		<!-- eslint-enable svelte/no-navigation-without-resolve -->
	{:else if onclick}
		<button
			{onclick}
			aria-current={active ? 'true' : undefined}
			class="min-w-0 flex-1 truncate text-start text-sm focus-visible:focus-ring"
		>
			{@render children()}
		</button>
	{:else}
		<div class="min-w-0 flex-1">
			{@render children()}
		</div>
	{/if}
	{#if actions}
		<div class="shrink-0">
			{@render actions()}
		</div>
	{/if}
</li>
