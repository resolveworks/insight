<script lang="ts">
	import type { Snippet } from 'svelte';
	import type { HTMLButtonAttributes } from 'svelte/elements';
	import LoadingSpinner from './LoadingSpinner.svelte';

	type Variant = 'primary' | 'secondary' | 'ghost';
	type Size = 'sm' | 'md' | 'lg';
	type Color = 'slate' | 'emerald';

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
		color = 'slate',
		loading = false,
		fullWidth = false,
		disabled,
		class: className = '',
		children,
		...rest
	}: Props = $props();

	const baseClasses =
		'rounded-md font-medium text-white transition-colors cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed';

	const colorClasses: Record<Color, string> = {
		slate: 'bg-slate-600 hover:bg-slate-700',
		emerald: 'bg-emerald-600 hover:bg-emerald-700',
	};

	let variantClasses = $derived.by(() => ({
		primary: colorClasses[color],
		secondary: 'bg-neutral-600 hover:bg-neutral-500',
		ghost: 'bg-transparent hover:bg-neutral-700 text-neutral-300',
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
