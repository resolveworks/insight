<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import LoadingSpinner from './LoadingSpinner.svelte';

	type Variant = 'primary' | 'secondary' | 'ghost';
	type Size = 'sm' | 'md' | 'lg';
	type Color = 'primary' | 'accent';

	interface Props extends HTMLButtonAttributes {
		variant?: Variant;
		size?: Size;
		color?: Color;
		loading?: boolean;
		fullWidth?: boolean;
		children: Snippet;
	}

	let {
		variant = 'primary',
		size = 'md',
		color = 'primary',
		loading = false,
		fullWidth = false,
		disabled,
		class: className = '',
		children,
		...rest
	}: Props = $props();

	const baseClasses =
		'rounded-md font-medium transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed';

	const colorClasses: Record<Color, string> = {
		primary: 'bg-primary-600 hover:bg-primary-700 text-white',
		accent: 'bg-tertiary-500 hover:bg-tertiary-600 text-white',
	};

	let variantClasses = $derived.by(() => ({
		primary: colorClasses[color],
		secondary:
			'bg-secondary-400 hover:bg-secondary-500 text-neutral-800 border border-secondary-600',
		ghost: 'bg-transparent hover:bg-neutral-200 text-neutral-700',
	}));

	const sizeClasses: Record<Size, string> = {
		sm: 'px-3 py-1.5 text-sm',
		md: 'px-4 py-2',
		lg: 'px-6 py-2',
	};
</script>

<button
	class="{baseClasses} {variantClasses[variant]} {sizeClasses[size]} {fullWidth
		? 'w-full'
		: ''} {className}"
	disabled={disabled || loading}
	{...rest}
>
	{#if loading}
		<LoadingSpinner size="sm" color="current" class="mr-2 inline-block" />
	{/if}
	{@render children()}
</button>
